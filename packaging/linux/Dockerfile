# The Linux release's build machine: Ubuntu 20.04, glibc 2.31, so what it
# builds runs on Debian 11, Ubuntu 20.04, RHEL 9 and every distribution since;
# with Rust and what the app, libmpv (vendor/mpv) and sc-gtkstyle build with.
#   docker build -t sparkcommander-build packaging/linux
# gcc 10 for libplacebo's C++20; Python 3.9 for the Meson and Jinja2 vendor/mpv
# brings (MarkupSafe 3 needs it). sc-qtstyle needs Qt 6, which 20.04 lacks:
# Dockerfile.qt builds it.
FROM ubuntu:20.04
RUN apt-get update && DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends \
    build-essential gcc-10 g++-10 pkg-config curl ca-certificates clang cmake git \
    libxkbcommon-dev libxkbcommon-x11-dev libwayland-dev libvulkan-dev libfontconfig1-dev libfreetype-dev \
    libx11-dev libx11-xcb-dev libxcb1-dev libxcb-xkb-dev libxi-dev libdbus-1-dev libssl-dev libzstd-dev libasound2-dev \
    ninja-build nasm patchelf python3 python3.9 xz-utils bzip2 perl \
    libpulse-dev libharfbuzz-dev libfribidi-dev zlib1g-dev libgtk-3-dev \
 && update-alternatives --install /usr/bin/gcc gcc /usr/bin/gcc-10 100 --slave /usr/bin/g++ g++ /usr/bin/g++-10 \
 && update-alternatives --install /usr/bin/cc cc /usr/bin/gcc-10 100 \
 && update-alternatives --install /usr/bin/c++ c++ /usr/bin/g++-10 100
# FFmpeg 9's assembly needs a newer nasm than 20.04's 2.14 (checksum as Homebrew's).
RUN cd /tmp && curl -fsSLO https://www.nasm.us/pub/nasm/releasebuilds/3.02/nasm-3.02.tar.xz \
 && echo "87336eba53b4acfe917424ab5d500d2b0054d9f5148d35c2273ccf2cfb712f0d  nasm-3.02.tar.xz" | sha256sum -c - \
 && tar -xf nasm-3.02.tar.xz && cd nasm-3.02 && ./configure --prefix=/usr/local >/dev/null && make -j"$(nproc)" >/dev/null && make install \
 && cd /tmp && rm -rf nasm-3.02*
# mpv's generators run `python3`, and need 3.9: found first on PATH; apt keeps /usr/bin/python3 (3.8).
RUN ln -s /usr/bin/python3.9 /usr/local/bin/python3
ENV RUSTUP_HOME=/opt/rustup CARGO_HOME=/opt/cargo PATH=/opt/cargo/bin:$PATH PYTHON=python3.9
RUN curl -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal --default-toolchain 1.99.0 && chmod -R a+rX /opt/rustup /opt/cargo
