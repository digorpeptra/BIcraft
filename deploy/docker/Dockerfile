# syntax=docker/dockerfile:1
#
# The `bicraft serve` image: one static musl binary on an empty base (docs/kubernetes.md).
#
#   docker build -f deploy/docker/Dockerfile -t bicraft:0.1.0 .
#
# builds the binary from the checked-out sources (`cargo build --profile dist --locked`, the
# profile of the release archives, docs/releasing.md). To package a binary built beforehand by
# `cargo xtask dist` instead (the `bicraft` file from the linux-x64 archive), pass its folder as a
# named build context:
#
#   docker build -f deploy/docker/Dockerfile --build-arg BINARY_SOURCE=prebuilt \
#     --build-context prebuilt=/path/to/bicraft-0.1.0-linux-x64 -t bicraft:0.1.0 .
#
# The final image holds the binary, its licence notices (/usr/share/doc/bicraft: LICENSE-MIT,
# LICENSE-APACHE, NOTICE, THIRD_PARTY_NOTICES.md and licenses/, as in the release archives; `bicraft
# licenses` prints the same text) and two empty folders, nothing else: no shell, no package
# manager, no CA bundle. The HTTPS clients (OIDC discovery and JWKS, the Databricks connector) use
# the Mozilla roots compiled into the binary (webpki-roots), so no system certificate store is read.
#
# Base images are pinned by tag and by digest (the multi-architecture index, read from the registry
# on 2026-10-27). Update both together.

ARG BINARY_SOURCE=source

# ---------------------------------------------------------------------------------------------
# Build from source: Alpine's Rust targets musl natively on amd64 and arm64, and links statically.
FROM rust:1.99.1-alpine3.22@sha256:731f64e16c12af70970d9a3655eecb1eecab4898352f09688a8e5ca2ab8b77c5 AS build-source
# ring (rustls) compiles a little C and assembly.
RUN apk add --no-cache musl-dev gcc
# The browser app (docs/webui.md) is part of every release binary: the wasm32 target and the
# wasm-bindgen CLI of exactly the version in Cargo.lock (`cargo xtask deploy check` compares them).
ARG WASM_BINDGEN_VERSION=0.2.129
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    rustup target add wasm32-unknown-unknown && \
    cargo install wasm-bindgen-cli --version "$WASM_BINDGEN_VERSION" --locked --root /tools
WORKDIR /src
COPY . .
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/src/target \
    set -eu; \
    triple="$(rustc -vV | sed -n 's/^host: //p')"; \
    BICRAFT_WEBUI_TOOLS=/tools cargo xtask webui --out /webui; \
    export BICRAFT_WEBUI_DIR=/webui BICRAFT_REQUIRE_WEBUI=1; \
    cargo build --profile dist --locked -p bicraft-cli --bin bicraft --target "$triple"; \
    mkdir -p /out; \
    cp "target/$triple/dist/bicraft" /out/bicraft
FROM scratch AS bin-source
COPY --from=build-source /out/bicraft /bicraft

# ---------------------------------------------------------------------------------------------
# Or package a prebuilt static binary (named build context `prebuilt`, see above).
FROM scratch AS bin-prebuilt
COPY --from=prebuilt bicraft /bicraft

FROM bin-${BINARY_SOURCE} AS bin

# ---------------------------------------------------------------------------------------------
# The folders the image provides: the workspace root and a temporary folder. In Kubernetes both are
# volumes (a PersistentVolumeClaim per workspace, a disk-backed emptyDir for /tmp) and the root
# file system is read-only.
FROM rust:1.99.1-alpine3.22@sha256:731f64e16c12af70970d9a3655eecb1eecab4898352f09688a8e5ca2ab8b77c5 AS layout
COPY --from=bin /bicraft /layout/usr/local/bin/bicraft
# The notices the licences of the embedded third-party material require (THIRD_PARTY_NOTICES.md).
COPY LICENSE-MIT LICENSE-APACHE NOTICE THIRD_PARTY_NOTICES.md /layout/usr/share/doc/bicraft/
COPY licenses/ /layout/usr/share/doc/bicraft/licenses/
# The checks: the binary runs, and it embeds the browser app (docs/webui.md; a release binary
# without it is refused, prebuilt ones too).
RUN set -eu; \
    /layout/usr/local/bin/bicraft --version; \
    /layout/usr/local/bin/bicraft build-info | grep -q '"embedded":true'; \
    mkdir -p /layout/srv/bicraft /layout/tmp; \
    chown 65532:65532 /layout/srv/bicraft; \
    chmod 0755 /layout/usr/local/bin/bicraft /layout/srv/bicraft; \
    chmod -R a+rX,go-w /layout/usr/share/doc/bicraft; \
    chmod 1777 /layout/tmp

# ---------------------------------------------------------------------------------------------
FROM scratch
LABEL org.opencontainers.image.licenses="MIT OR Apache-2.0" \
      org.opencontainers.image.title="bicraft"
COPY --from=layout /layout/ /
# A fixed, unprivileged identity (the distroless "nonroot" numbers). No /etc/passwd is needed.
USER 65532:65532
# The temporary folder holds the spill files of models larger than the memory budget
# (docs/beyond-ram.md): mount a disk-backed volume there, never a memory-backed one.
ENV TMPDIR=/tmp \
    BICRAFT_SPILL_DIR=/tmp
EXPOSE 7878
# `bicraft serve` refuses a non-loopback address until authentication is configured
# (BICRAFT_API_TOKENS_FILE, BICRAFT_OIDC_ISSUERS, BICRAFT_TRUSTED_PROXIES or BICRAFT_AUTH_CONFIG,
# plus BICRAFT_AUTH_GRANTS and BICRAFT_ALLOWED_HOSTS or BICRAFT_PUBLIC_URL: docs/auth.md). The
# default command serves the subfolders of /srv/bicraft on every interface.
ENTRYPOINT ["/usr/local/bin/bicraft", "serve"]
CMD ["--root", "/srv/bicraft", "--listen", "0.0.0.0:7878"]
