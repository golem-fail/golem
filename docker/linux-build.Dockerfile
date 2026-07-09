# syntax=docker/dockerfile:1
#
# Cross-compile a fully static musl `golem` for Linux.
#
# The golem binary is self-contained (companions baked in). On a non-macOS
# target the iOS companion is auto-excluded (build.rs gates the embed on
# CARGO_CFG_TARGET_OS), so a Linux build carries the Android companion only.
#
# STAGE 1 (this file, for now): toolchain only — proves the musl build and that
# the resulting static binary runs. Without the Android SDK, build.rs writes an
# empty Android-companion marker (it handles a missing gradle/SDK gracefully), so
# the binary compiles + runs but reports the Android companion as not embedded.
# The Android SDK + JDK layer (to actually embed the companion) comes next.
#
# Build (native arch on the host; no QEMU when host arch == TARGETARCH):
#   docker build -f docker/linux-build.Dockerfile \
#     --build-arg TARGET=aarch64-unknown-linux-musl -t golem-linux:musl .
# Run:
#   docker run --rm golem-linux:musl --help

ARG RUST_VERSION=1
FROM rust:${RUST_VERSION}-slim-bookworm AS build

# musl toolchain + the target. rustc's self-contained musl support links fully
# static; musl-tools/musl-gcc covers any C dependency build scripts.
ARG TARGET=aarch64-unknown-linux-musl
RUN apt-get update \
 && apt-get install -y --no-install-recommends musl-tools musl-dev pkg-config \
 && rm -rf /var/lib/apt/lists/*
RUN rustup target add "${TARGET}"

WORKDIR /src
COPY . .
RUN cargo build --release --target "${TARGET}" -p golem-cli --bin golem \
 && cp "target/${TARGET}/release/golem" /golem

# Final image: scratch proves the binary is fully static (no libc, no shell).
FROM scratch AS dist
COPY --from=build /golem /golem
ENTRYPOINT ["/golem"]
