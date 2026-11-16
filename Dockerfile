# syntax=docker/dockerfile:1

# ---- builder ----
# Cross-compile both arches from this single builder image: the host
# architecture is taken from the build platform (BUILDPLATFORM), the
# target arch from TARGETPLATFORM. On an amd64 host an arm64 image
# needs the aarch64 rust target; on an arm64 host the native target
# is used and nothing extra is installed.
FROM --platform=$BUILDPLATFORM rust:1-bookworm AS builder
WORKDIR /build
ARG TARGETPLATFORM
# Set to true only when vendored crate sources are staged next to the
# manifests (vendor/ plus .cargo/config.toml). Online builds leave it
# false and fetch from crates.io. Committed repos ship neither dir,
# so the same Dockerfile serves both cases.
ARG CARGO_OFFLINE=false
# Layer cache: manifests first, sources after. The .dockerignore whitelist
# keeps the context to the manifests and sources, plus optional vendor/ and
# .cargo/ that only offline builds stage, so `COPY . .` picks them up when
# present and skips them cleanly when absent.
COPY Cargo.toml Cargo.lock ./
COPY crates ./crates
COPY src ./src
COPY . .
RUN if [ "$TARGETPLATFORM" = "linux/arm64" ]; then \
      rustup target add aarch64-unknown-linux-gnu \
 && apt-get update \
 && apt-get install -y --no-install-recommends gcc-aarch64-linux-gnu libc6-dev-arm64-cross \
 && rm -rf /var/lib/apt/lists/*; fi
ENV CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_LINKER=aarch64-linux-gnu-gcc \
    CC_aarch64_unknown_linux_gnu=aarch64-linux-gnu-gcc \
    AR_aarch64_unknown_linux_gnu=aarch64-linux-gnu-ar
# Layer cache: manifests first, sources after.
RUN cargo build --release --locked \
    $(if [ "$CARGO_OFFLINE" = "true" ]; then echo --offline; fi) \
    $(if [ "$TARGETPLATFORM" = "linux/amd64" ]; then echo ""; else echo --target aarch64-unknown-linux-gnu; fi) \
 && mkdir -p /artifact \
 && install -m 0755 $(if [ "$TARGETPLATFORM" = "linux/amd64" ]; then echo "target/release/home"; else echo "target/aarch64-unknown-linux-gnu/release/home"; fi) /artifact/home

# ---- runtime ----
FROM debian:bookworm-slim
RUN apt-get update \
 && apt-get install -y --no-install-recommends ca-certificates \
 && rm -rf /var/lib/apt/lists/* \
 && groupadd -r home \
 && useradd -r -g home -u 10001 -m -d /data home \
 && mkdir -p /data \
 && chown home:home /data
COPY --from=builder /artifact/home /usr/local/bin/home

# SQLite lives on a mounted volume, never baked into the image.
# Store::open creates the parent dir, but /data must be writable by `home`.
VOLUME ["/data"]
ENV HOME_DB=/data/home.sqlite \
    HOME_HOST=0.0.0.0 \
    HOME_PORT=8080
EXPOSE 8080
USER home
ENTRYPOINT ["home"]
CMD ["serve"]
