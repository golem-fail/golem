# syntax=docker/dockerfile:1
#
# Cross-compile a fully static musl `golem` for Linux.
#
# The golem binary is self-contained (companions baked in). On a non-macOS
# target the iOS companion is auto-excluded (build.rs gates the embed on
# CARGO_CFG_TARGET_OS), so a Linux build carries the Android companion only.
#
# build.rs compiles the Android companion via gradle, so this image carries the
# Android SDK + JDK 17. The iOS companion is auto-excluded on Linux, so no Xcode.
#
# IMPORTANT — build on linux/amd64. Android's Linux build-tools (aapt2) ship as
# x86_64-only binaries, so gradle can't build the companion on an arm64 build
# host. The musl *target* is independent of the build-host arch. On an Apple
# Silicon host the amd64 build runs under emulation (slow); CI amd64 runners are
# native.
#
# Build:
#   docker build --platform linux/amd64 -f docker/linux-build.Dockerfile \
#     --build-arg TARGET=x86_64-unknown-linux-musl -t golem-linux:musl .
# Run:
#   docker run --rm golem-linux:musl doctor

ARG RUST_VERSION=1
FROM --platform=linux/amd64 rust:${RUST_VERSION}-slim-bookworm AS build

# musl toolchain + the target (rustc links fully static; musl-gcc covers C deps),
# plus JDK 17 to run the Android Gradle Plugin and the tools to fetch the SDK.
ARG TARGET=x86_64-unknown-linux-musl
RUN apt-get update \
 && apt-get install -y --no-install-recommends \
      musl-tools musl-dev pkg-config \
      gcc-aarch64-linux-gnu g++-aarch64-linux-gnu \
      openjdk-17-jdk-headless curl unzip ca-certificates \
 && rm -rf /var/lib/apt/lists/*
# Add the musl target to the PINNED toolchain, not the image's default. The
# `cargo build` below honors rust-toolchain.toml and auto-selects that channel;
# if the target was added to a different (image-default) toolchain the build
# fails with "can't find crate for core". Copy the pin file first so
# `rustup target add` resolves the same channel — kept before `COPY . .` so this
# layer still caches across source changes.
WORKDIR /src
COPY rust-toolchain.toml ./
RUN rustup target add "${TARGET}"

# Cross-compiling to aarch64-musl from an amd64 build host: use the aarch64 GNU
# cross toolchain as the linker + C compiler (ring & friends have C). These vars
# only affect the aarch64 target, so the native x86_64 build is unchanged.
ENV CARGO_TARGET_AARCH64_UNKNOWN_LINUX_MUSL_LINKER=aarch64-linux-gnu-gcc \
    CC_aarch64_unknown_linux_musl=aarch64-linux-gnu-gcc \
    CXX_aarch64_unknown_linux_musl=aarch64-linux-gnu-g++

# Android SDK (companion needs compileSdk/build-tools 34, AGP 8.2.2 → JDK 17).
# Cached layer: independent of the source, so it survives code changes.
ENV ANDROID_HOME=/opt/android-sdk ANDROID_SDK_ROOT=/opt/android-sdk
ENV PATH="${PATH}:/opt/android-sdk/cmdline-tools/latest/bin:/opt/android-sdk/platform-tools"
ARG CMDLINE_TOOLS=commandlinetools-linux-11076708_latest.zip
RUN curl -fsSL "https://dl.google.com/android/repository/${CMDLINE_TOOLS}" -o /tmp/cmdtools.zip \
 && mkdir -p "${ANDROID_HOME}/cmdline-tools" \
 && unzip -q /tmp/cmdtools.zip -d "${ANDROID_HOME}/cmdline-tools" \
 && mv "${ANDROID_HOME}/cmdline-tools/cmdline-tools" "${ANDROID_HOME}/cmdline-tools/latest" \
 && rm /tmp/cmdtools.zip \
 && yes | sdkmanager --licenses >/dev/null \
 && sdkmanager --install "platform-tools" "platforms;android-34" "build-tools;34.0.0" >/dev/null

WORKDIR /src
COPY . .
RUN cargo build --release --target "${TARGET}" -p golem-cli --bin golem \
 && cp "target/${TARGET}/release/golem" /golem

# Final image: scratch proves the binary is fully static (no libc, no shell).
FROM scratch AS dist
COPY --from=build /golem /golem
ENTRYPOINT ["/golem"]
