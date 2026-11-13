# The Linux release's build machine: Ubuntu 24.04, the oldest the release is
# for (glibc 2.39), with Rust and what the app and libmpv (vendor/mpv) build with.
#   docker build -t powercommander-build packaging/linux
FROM ubuntu:24.04
RUN apt-get update && DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends \
    build-essential pkg-config curl ca-certificates clang cmake git \
    libxkbcommon-dev libxkbcommon-x11-dev libwayland-dev libvulkan-dev libfontconfig1-dev libfreetype-dev \
    libx11-dev libx11-xcb-dev libxcb1-dev libxcb-xkb-dev libxi-dev libdbus-1-dev libssl-dev libzstd-dev libasound2-dev \
    ninja-build nasm patchelf python3 xz-utils bzip2 perl \
    libpulse-dev libharfbuzz-dev libfribidi-dev zlib1g-dev
ENV RUSTUP_HOME=/opt/rustup CARGO_HOME=/opt/cargo PATH=/opt/cargo/bin:$PATH
RUN curl -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal --default-toolchain 1.99.0 && chmod -R a+rX /opt/rustup /opt/cargo
