# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What this is

A WebDAV server (Rust, edition 2024) that exposes 夸克网盘 (Quark Drive) as a WebDAV filesystem. Authentication is a raw browser cookie string; there is no official API — every endpoint in `src/drive/` is reverse-engineered from the web client, which is why requests carry hardcoded `Origin`/`Referer`/`User-Agent` constants.

## Commands

```bash
cargo build --release          # lto = "thin"
cargo test                     # unit tests only; live tests are #[ignore]d
cargo test test_is_url_expired # run a single test by name substring
cargo clippy                   # CI runs this with continue-on-error, so it does not gate

# Live integration tests hit the real Quark API and mutate the account (create/rename/
# move/delete folders and files). They require a real cookie:
QUARK_COOKIE='...' cargo test -- --ignored
QUARK_COOKIE='...' cargo test test_upload_pre_and_hash -- --ignored --nocapture

# Run locally
cargo run -- --quark-cookie '...' -U admin -W admin -p 8080 --debug
```

Tests print diagnostics to stderr (`eprintln!`) even when green — read it, live tests report skips and API anomalies that way.

`docker/Dockerfile` cannot be built standalone: it `COPY`s `bin/${TARGETARCH}/quarkdrive-webdav`, a musl binary cross-compiled by `.github/workflows/docker.yml` beforehand. Both release workflows are `workflow_dispatch`-only.

`.bumpversion.cfg` is stale — its `current_version` (2.3.3) disagrees with `Cargo.toml` (1.3.9) and it lists `openwrt/` and `snap/` files that don't exist here. Bump `Cargo.toml` by hand.

## Architecture

Four layers, bottom-up:

| File | Role |
|---|---|
| `src/drive/mod.rs` | Quark HTTP client. One method per reverse-engineered endpoint. `QuarkDrive` is cheap to `clone()` (shares cookie map + md5 cache via `Arc`). |
| `src/drive/model.rs` | Request/response structs. `QuarkFile` doubles as `DavMetaData` + `DavDirEntry`. |
| `src/cache.rs` | `moka` cache mapping absolute path → full directory listing. **Also the path resolver** (see below). |
| `src/vfs.rs` | `DavFileSystem` / `DavFile` impls. All upload logic lives here. |
| `src/webdav.rs` | `hyper` service: basic auth, browser HTML directory UI, RFC 3230 `Digest` header. |
| `src/main.rs` | `clap` CLI (every flag also reads an env var), wiring, SIGHUP handler. |

### The cache is the path resolver — not an optimization

The Quark API only lists children by `pdir_fid`; there is no "stat this path" call. So `Cache::get_or_insert(path)` walks up to the nearest cached ancestor, then DFS's back down, caching each directory it passes through. Consequences:

- `QuarkDriveFileSystem::get_file()` returns `None` when the path isn't reachable through the cache — there is **no** drive fallback. A missed invalidation shows up as a 404, not as stale data.
- Every mutating op (`create_dir`, `remove_*`, `rename`) must invalidate the affected parents itself. Quark's backend needs a moment to reflect writes, which is why those paths `sleep` 1–2s *before* invalidating. Removing those sleeps reintroduces stale listings.
- Directory listings are paginated at 500 entries and capped at 20 pages (~10k files) per directory.
- Whole cache is flushed every `--refresh-cache-secs-interval` (default 300s) and on `SIGHUP` (unix only).

### Upload flow (`QuarkDavFile::do_flush`, `src/vfs.rs`)

Bytes are **not** buffered in memory. `consume_buf()` appends each `write_buf` chunk to `/tmp/<timestamp>_<name>` and folds it into running MD5 + SHA-1 contexts. On `flush()`:

1. If overwriting, compare cloud MD5 → identical content short-circuits without re-uploading.
2. `up_pre` → may return `finish: true` (秒传 / instant server-side dedup).
3. `up_hash(md5, sha1)` → second 秒传 chance.
4. Otherwise per chunk: `up_part_auth_meta` → `auth` → `up_part`, then `up_auth_and_commit` → `finish`.

The chunk loop runs inside `tokio::spawn` so a client disconnect doesn't abort it. The caller awaits with a `--upload-wait-timeout` (default 280s); on timeout it returns success to the client while the upload continues in the background. Temp-file cleanup and cache invalidation happen inside the spawned task, so they run on both paths. The Docker image also cron-deletes stale `/tmp` files nightly as a backstop.

The `uploading: DashMap<parent_path, Vec<QuarkFile>>` holds placeholder entries with empty `fid` so `metadata()` can answer for in-flight files that don't exist in the cloud yet. `read_bytes` on an empty `fid` returns `NotFound`.

### OSS signature strings are load-bearing

`up_part_auth_meta` and `up_commit_auth_meta` build canonical strings that Quark forwards to Aliyun OSS. The exact newline layout, header ordering, and the hardcoded `x-oss-user-agent: aliyun-sdk-js/6.6.1 Chrome 98.0.4758.80 on Windows 10 64-bit` are part of the signed payload. Reformatting them breaks uploads with an auth error, not a compile error.

### Other invariants

- `copy` returns `NotImplemented` — the Quark API has no copy. `rename` covers both rename (same parent) and move (`move_file` + optional `rename_file`).
- Open in append mode is rejected; `open()` always starts the size counter at 0 because clients often omit `Content-Length` for large uploads.
- Download URLs carry an `Expires` query param; `is_url_expired` treats them as dead 60s early and re-fetches.
- The `__puus` cookie is rotated by the server — `update_cookie_from_response` writes it back into the shared `DashMap` on every response. Cookies must stay in that shared map, not be snapshotted.
- Browser detection is `GET` + `Accept: text/html`; those requests get the hand-rendered HTML listing in `webdav.rs` instead of a WebDAV response, so both code paths need updating when listing behavior changes.

## Conventions

- Logging is `tracing` with structured fields (`debug!(path = %path.display(), "fs: read_dir")`), gated by `RUST_LOG` or `--debug`.
- Comments and log messages are mixed Chinese/English; match whatever the surrounding block uses.
- `Cargo.toml` keeps many commented-out dependencies. Leave them.
