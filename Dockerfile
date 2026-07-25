# Self-contained image: compiles from source, unlike docker/Dockerfile which
# expects binaries that the release CI cross-compiled beforehand.
#
#   docker build -t quarkdrive-webdav:local .
#
# Builds for whatever architecture you build on. rust:alpine is musl-based, so
# the result is a static binary that runs on old kernels (DSM 7 is on 4.4).

# ---- build ----
# edition 2024 needs Rust >= 1.85. Pin this tag once you have a build that works,
# so a toolchain bump never surprises you mid-deploy.
FROM rust:alpine AS builder

# ring needs a C toolchain; rustls here uses ring, not aws-lc-rs, so no
# cmake/perl/go are required.
RUN apk add --no-cache build-base

WORKDIR /src

# Compile dependencies first, against a stub main. On a NAS CPU the ~200 crates
# dominate the build, and this layer is reused until the manifests change —
# turning an edit-rebuild cycle from ~15 minutes into under a minute.
COPY Cargo.toml Cargo.lock ./
RUN mkdir -p src && echo 'fn main() {}' > src/main.rs \
 && cargo build --release \
 && rm -rf src

COPY src ./src
# Drop the stub's artifacts: cargo decides by mtime, and a fresh COPY is not
# always enough to convince it the crate changed.
RUN rm -f target/release/quarkdrive-webdav \
 && rm -f target/release/deps/quarkdrive_webdav* \
 && cargo build --release \
 && strip target/release/quarkdrive-webdav

# ---- runtime ----
FROM alpine:3.21

RUN addgroup -S app && adduser -S app -G app -s /bin/sh

COPY --from=builder /src/target/release/quarkdrive-webdav /usr/local/bin/quarkdrive-webdav

# Staging directory for the fallback upload path, used only by clients that send
# no Content-Length. Uploads that declare a length stream straight to the cloud
# and never write here. Deliberately not /tmp: on many hosts that is tmpfs, i.e.
# RAM, and a few concurrent multi-GB uploads there will take the machine down.
# Mount a volume on this path if you use clients without Content-Length.
ENV UPLOAD_TEMP_DIR=/var/cache/quarkdrive
RUN mkdir -p /var/cache/quarkdrive && chown app:app /var/cache/quarkdrive

# No crond. The upstream image ran one to delete /tmp files older than 15
# minutes, working around staged files that leaked when a client abandoned a PUT.
# That leak is fixed at the source now, and the sweep was itself unsafe: a large
# upload being read chunk by chunk is only ever *read*, so under relatime its
# timestamps go stale and the job would delete it mid-upload.

USER app
EXPOSE 8080
ENTRYPOINT ["/usr/local/bin/quarkdrive-webdav"]
