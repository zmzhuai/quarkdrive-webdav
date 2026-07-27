use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;
use dashmap::DashMap;
use moka::future::Cache as MokaCache;
use tokio::sync::{Mutex, OwnedMutexGuard};
use tracing::debug;
use crate::drive::{QuarkDrive};
use crate::drive::model::QuarkFile;

#[derive(Clone)]
pub struct Cache {
    inner: MokaCache<String, Vec<QuarkFile>>,
    drive: QuarkDrive,
    // Filling an entry takes two steps — read the listing from Quark, then write
    // it back — and an invalidation can land in between. Uploads invalidate the
    // parent the moment they finish, so without a marker the fetch already in
    // flight writes the pre-upload listing back over that invalidation, and the
    // file that was just uploaded stays invisible until the next full refresh.
    generations: Arc<DashMap<String, u64>>,
    epoch: Arc<AtomicU64>,
    // Concurrent misses on one directory each used to run their own full listing:
    // a photo sync finishing eight uploads at once read the same directory eight
    // times over. The first caller reads, the rest wait on it and then find the
    // listing already there.
    fills: Arc<DashMap<String, Arc<Mutex<()>>>>,
}
const ONE_PAGE: u32 = 500;

impl Cache {
    pub fn new(max_capacity: u64, ttl: u64, drive: QuarkDrive) -> Self {
        let inner = MokaCache::builder()
            .max_capacity(max_capacity)
            .time_to_live(Duration::from_secs(ttl))
            .build();
        
        Self {
            inner,
            drive,
            generations: Arc::new(DashMap::new()),
            epoch: Arc::new(AtomicU64::new(0)),
            fills: Arc::new(DashMap::new()),
        }
    }
    pub async fn get_or_insert(&self, key: &str) -> Option<Vec<QuarkFile>> {
        debug!(key = %key, "cache: get_or_insert");
        if let Some(files) = self.get(key).await {
            return Some(files);
        }
        // Only one caller reads a given directory at a time.
        let _fill = self.fill_lock(key).await;
        // Whoever held the lock has just filled it, so look again before reading.
        if let Some(files) = self.get(key).await {
            debug!(key = %key, "cache: filled while waiting");
            return Some(files);
        }
        // What the walk below read from the drive, whether or not it was cached.
        let fetched;
        if key == "/" {
            fetched = self.dfs(QuarkFile::new_root(), key, "/").await;
        }else {
            let mut path = Path::new(key);
            let mut dsf_root_file = None;
            while let Some(parent) = path.parent() {
                if let Some(c_files) = self.get(parent.to_str().unwrap()).await {
                    let file_name = path.file_name().and_then(|os_str| os_str.to_str());
                    let found = c_files.iter().find(|quark_file| {
                        Some(quark_file.file_name.as_str()) == file_name
                    }).cloned();
                    if found.is_none() {
                        debug!(key = %key, "cache: no file found for path: {}", path.to_str().unwrap());
                        path = parent;
                        continue;
                    }
                    dsf_root_file = found;
                    break;
                }

                path = parent;

                if path.to_str() == Some("/") {
                    break;
                }

            }
            if path.to_str() == Some("/") {
                fetched = self.dfs(QuarkFile::new_root(), key, "/").await;
            }else {
                match dsf_root_file { 
                    Some(dsf_root_fil) => {
                        debug!(key = %key, "cache: found root file: {}", dsf_root_fil.file_name);
                        fetched = self.dfs(dsf_root_fil, key, path.to_str().unwrap()).await;
                    },
                    None => {
                        debug!(key = %key, "cache: no root file found for path: {}", path.to_str().unwrap());
                        return None;
                    }
                }
            }

        }
        if let Some(files) = self.get(key).await {
            Some(files)
        }else if fetched.is_some() {
            // The listing was read but an invalidation landed while it was being
            // read, so it was not kept. Still answer this caller with it — a
            // dropped write-back must not turn into a spurious 404.
            debug!(key = %key, "cache: answering with uncached listing");
            fetched
        }else {
            debug!(key = %key, "cache: no files found for key");
            None
        }
    }

    async fn dfs(&self, file: QuarkFile, target_path: &str, dfs_path: &str) -> Option<Vec<QuarkFile>> {
        if file.dir {
            // Taken before the first read, so an invalidation racing this fetch
            // can be told apart from one that happened before it started.
            let stamp = self.stamp(dfs_path);
            let mut current_files = Vec::<QuarkFile>::new();
            for page_no in 1..=20 {
                let (files, total) =
                    match self.drive.get_files_by_pdir_fid(&file.fid, page_no, ONE_PAGE).await{
                    Ok((k, v)) => (k, v),
                    Err(e) => {
                        debug!(error = %e, file_id = &file.fid, file_name = &file.file_name,
                                page_no = page_no,
                            "Failed to get files from drive");
                        return None;
                    }
                };
                let mut files = files.unwrap();
                // add dfs_path to each file
                for f in files.list.iter_mut() {
                    f.parent_path = Some(dfs_path.to_string());
                }
                let size = files.list.len();
                current_files.extend(files.list);
                // guess: es limit is 10000
                if size < ONE_PAGE as usize || page_no >= total / ONE_PAGE + 1   {
                    break;
                }
            }

            self.insert_if_current(dfs_path.to_string(), current_files.clone(), stamp).await;
            debug!("{} in cache", &dfs_path);
            if dfs_path == target_path {
                return Some(current_files);
            }
            for curr_f in current_files {
                let file_path = if dfs_path == "/" {
                    format!("{}{}", dfs_path, curr_f.file_name)
                }else {
                    format!("{}/{}", dfs_path, curr_f.file_name)
                };
                if !target_path.starts_with(&file_path) {
                    continue;
                }
                if let Some(files) = Box::pin(self.dfs(curr_f, target_path, &file_path)).await {
                    return Some(files);
                }
            }

        }
        None
    }

    async fn get(&self, key: &str) -> Option<Vec<QuarkFile>> {
        debug!(key = %key, "cache: get");
        self.inner.get(key).await
    }

    async fn insert(&self, key: String, value: Vec<QuarkFile>) {
        debug!(key = %key, "cache: insert");
        self.inner.insert(key, value).await;
    }

    /// Held for the whole read-then-write-back of one directory, so that callers
    /// arriving during it wait for that result instead of asking Quark again.
    async fn fill_lock(&self, key: &str) -> OwnedMutexGuard<()> {
        let lock = self
            .fills
            .entry(key.to_string())
            .or_insert_with(|| Arc::new(Mutex::new(())))
            .clone();
        lock.lock_owned().await
    }

    /// How many times this key has been invalidated. A listing may only be kept
    /// if the marker it took before reading is still the current one.
    fn stamp(&self, key: &str) -> (u64, u64) {
        let generation = self.generations.get(key).map(|g| *g.value()).unwrap_or(0);
        (self.epoch.load(Ordering::SeqCst), generation)
    }

    async fn insert_if_current(&self, key: String, value: Vec<QuarkFile>, stamp: (u64, u64)) {
        if self.stamp(&key) != stamp {
            debug!(key = %key, "cache: invalidated while reading, dropping listing");
            return;
        }
        self.insert(key.clone(), value).await;
        // The check above and the insert are not one atomic step. An invalidation
        // that slipped between them would otherwise be lost, so undo the insert.
        if self.stamp(&key) != stamp {
            debug!(key = %key, "cache: invalidated while inserting, dropping listing");
            self.inner.invalidate(&key).await;
        }
    }

    pub async fn invalidate(&self, path: &Path) {
        let key = path.to_string_lossy().into_owned();
        debug!(path = %path.display(), key = %key, "cache: invalidate");
        // Bump before removing: a fetch that reads the marker after this point
        // sees the new value and refuses to write its stale listing back.
        *self.generations.entry(key.clone()).or_insert(0) += 1;
        self.inner.invalidate(&key).await;
    }

    pub async fn invalidate_parent(&self, path: &Path) {
        if let Some(parent) = path.parent() {
            self.invalidate(parent).await;
        }
    }

    pub fn invalidate_all(&self) {
        debug!("cache: invalidate all");
        // One epoch bump stands in for bumping every key's marker, which lets the
        // per-key markers be dropped here — that is what keeps the map bounded.
        self.epoch.fetch_add(1, Ordering::SeqCst);
        self.generations.clear();
        // Locks currently held stay alive through their Arc; dropping the map
        // here is what keeps it from growing for the life of the process.
        self.fills.clear();
        self.inner.invalidate_all();
    }

}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;
    use crate::drive::DriveConfig;

    const KEY: &str = "/我的备份/NAS/zm/Photos/MobileBackup/iPhone/2025/01";

    // Nothing here reaches the network: the listings are handed to the cache
    // directly, standing in for what a read from the drive would have returned.
    fn test_cache() -> Cache {
        let drive = QuarkDrive::new(DriveConfig {
            api_base_url: "http://127.0.0.1:1".to_string(),
            cookie: Arc::new(DashMap::new()),
        })
        .unwrap();
        Cache::new(64, 300, drive)
    }

    fn listing(names: &[&str]) -> Vec<QuarkFile> {
        names
            .iter()
            .map(|name| QuarkFile {
                file_name: name.to_string(),
                dir: false,
                file: true,
                ..QuarkFile::new_root()
            })
            .collect()
    }

    fn names(files: &[QuarkFile]) -> Vec<&str> {
        files.iter().map(|f| f.file_name.as_str()).collect()
    }

    #[tokio::test]
    async fn listing_read_before_an_invalidation_is_not_kept() {
        let cache = test_cache();
        cache.insert(KEY.to_string(), listing(&["DSCF2100.jpg"])).await;

        // A read of the directory starts here and takes a while to come back.
        let stamp = cache.stamp(KEY);
        // Meanwhile DSCF2101.jpg finishes uploading and invalidates the parent.
        cache.invalidate(Path::new(KEY)).await;
        // Only now does the read land, carrying the listing from before the upload.
        cache
            .insert_if_current(KEY.to_string(), listing(&["DSCF2100.jpg"]), stamp)
            .await;

        assert!(
            cache.get(KEY).await.is_none(),
            "a listing read before the invalidation must not survive it — \
             caching it hides the file that was just uploaded until the next full refresh",
        );
    }

    #[tokio::test]
    async fn listing_read_after_an_invalidation_is_kept() {
        let cache = test_cache();
        cache.insert(KEY.to_string(), listing(&["DSCF2100.jpg"])).await;

        cache.invalidate(Path::new(KEY)).await;
        let stamp = cache.stamp(KEY);
        cache
            .insert_if_current(KEY.to_string(), listing(&["DSCF2100.jpg", "DSCF2101.jpg"]), stamp)
            .await;

        let cached = cache.get(KEY).await.expect("a listing read after the invalidation is current");
        assert_eq!(names(&cached), vec!["DSCF2100.jpg", "DSCF2101.jpg"]);
    }

    #[tokio::test]
    async fn a_full_refresh_also_discards_in_flight_listings() {
        let cache = test_cache();

        // This key was never invalidated on its own, so only the epoch tells the
        // two sides of the periodic refresh apart.
        let stamp = cache.stamp(KEY);
        cache.invalidate_all();
        cache
            .insert_if_current(KEY.to_string(), listing(&["DSCF2100.jpg"]), stamp)
            .await;

        assert!(
            cache.get(KEY).await.is_none(),
            "a listing read before the periodic refresh must not survive it",
        );
    }

    // Mirrors what get_or_insert does around the drive read — take the fill lock,
    // look again, and only then read — with the read replaced by a counter.
    async fn fill_once(cache: &Cache, key: &str, reads: &AtomicUsize) {
        if cache.get(key).await.is_some() {
            return;
        }
        let _fill = cache.fill_lock(key).await;
        if cache.get(key).await.is_some() {
            return;
        }
        reads.fetch_add(1, Ordering::SeqCst);
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        cache.insert(key.to_string(), listing(&["DSCF2100.jpg"])).await;
    }

    #[tokio::test]
    async fn concurrent_misses_on_one_directory_read_it_once() {
        let cache = test_cache();
        let reads = Arc::new(AtomicUsize::new(0));

        // Eight uploads into the same directory finishing at once.
        let mut tasks = Vec::new();
        for _ in 0..8 {
            let cache = cache.clone();
            let reads = reads.clone();
            tasks.push(tokio::spawn(async move { fill_once(&cache, KEY, &reads).await }));
        }
        for task in tasks {
            task.await.unwrap();
        }

        assert_eq!(
            reads.load(Ordering::SeqCst),
            1,
            "concurrent misses on one directory must share a single read, \
             not ask Quark once per upload",
        );
    }

    // Sharing one read between callers is only safe because of the marker above:
    // a waiter must never be handed a listing that was read before its own upload
    // finished, or "my file is not in it" would mean nothing and it would drop its
    // placeholder while 404 is still the only answer metadata() could give.
    #[tokio::test]
    async fn a_shared_read_older_than_the_waiter_is_not_handed_to_it() {
        let cache = test_cache();

        // A finishes uploading, invalidates the parent and starts reading it.
        cache.invalidate(Path::new(KEY)).await;
        let a_read = cache.stamp(KEY);

        // B finishes while A's read is still out.
        cache.invalidate(Path::new(KEY)).await;

        // A's read lands: it has A but not B, because Quark had not indexed B
        // when the read went out.
        cache
            .insert_if_current(KEY.to_string(), listing(&["DSCF2100.jpg"]), a_read)
            .await;

        assert!(
            cache.get(KEY).await.is_none(),
            "B waits behind A's read, so that read must not be left in the cache \
             for B to conclude anything from — it predates B finishing",
        );
    }

    #[tokio::test]
    async fn misses_on_different_directories_do_not_block_each_other() {
        let cache = test_cache();
        let reads = Arc::new(AtomicUsize::new(0));

        let mut tasks = Vec::new();
        for i in 0..4 {
            let cache = cache.clone();
            let reads = reads.clone();
            let key = format!("{KEY}/{i}");
            tasks.push(tokio::spawn(async move { fill_once(&cache, &key, &reads).await }));
        }
        for task in tasks {
            task.await.unwrap();
        }

        assert_eq!(
            reads.load(Ordering::SeqCst),
            4,
            "separate directories must not serialise behind one another",
        );
    }

    #[tokio::test]
    async fn invalidations_of_other_directories_do_not_discard_a_listing() {
        let cache = test_cache();

        let stamp = cache.stamp(KEY);
        // Sibling directories churn constantly while a photo sync runs; that must
        // not cost every other directory its listing.
        cache.invalidate(Path::new("/我的备份/NAS/li/Photos")).await;
        cache
            .insert_if_current(KEY.to_string(), listing(&["DSCF2100.jpg"]), stamp)
            .await;

        let cached = cache.get(KEY).await.expect("another directory's invalidation is unrelated");
        assert_eq!(names(&cached), vec!["DSCF2100.jpg"]);
    }
}