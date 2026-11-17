# syntax=docker/dockerfile:1

# ---- builder ----
# Build natively for the target arch instead of cross-compiling: rusqlite
# (bundled SQLite) and ring ship C/asm, and a glibc cross toolchain cannot
# build them against musl. The rustup toolchain's musl targets already default
# to crt-static, so the binary comes out fully static; do NOT set a global
# RUSTFLAGS="-C target-feature=+crt-static" here, which rustc also applies to
# proc-macro crates (clap_derive) and which they reject.
FROM rust:1-alpine AS builder
WORKDIR /build
ARG CARGO_OFFLINE=false
# Layer cache: manifests first, sources after. The .dockerignore whitelist
# keeps the context to the manifests and sources, plus optional vendor/ and
# .cargo/ that only offline builds stage, so `COPY . .` picks them up when
# present and skips them cleanly when absent.
COPY Cargo.toml Cargo.lock ./
COPY crates ./crates
COPY src ./src
COPY . .
RUN cargo build --release --locked \
    $(if [ "$CARGO_OFFLINE" = "true" ]; then echo --offline; fi) \
 && mkdir -p /artifact \
 && install -m 0755 target/release/home /artifact/home

# ---- runtime ----
# alpine:3.21 ships /etc/ssl/certs/ca-certificates.crt, but reqwest here uses
# rustls + webpki-roots, which compiles its own trust anchors in, so no
# ca-certificates package is needed (and none is installed: the image stays
# as small as possible).
FROM alpine:3.21
RUN addgroup -S home \
 && adduser -S -G home -u 10001 -h /data home
COPY --from=builder /artifact/home /usr/local/bin/home

# SQLite lives on a mounted volume, never baked into the image.
# `adduser -h /data` already creates /data owned by home:home.
VOLUME ["/data"]
ENV HOME_DB=/data/home.sqlite \
    HOME_HOST=0.0.0.0 \
    HOME_PORT=8080
EXPOSE 8080
USER home
ENTRYPOINT ["home"]
CMD ["serve"]
