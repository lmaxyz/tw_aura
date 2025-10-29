FROM debian:12
ARG DEBIAN_FRONTEND=noninteractive

RUN apt-get update && apt-get install --assume-yes --no-install-recommends \
    build-essential wget tar git pkg-config nasm \
    gcc-aarch64-linux-gnu g++-aarch64-linux-gnu \
    libc6-dev-arm64-cross

# Сборка OpenSSL 1.1.1
RUN wget --no-check-certificate https://github.com/openssl/openssl/releases/download/OpenSSL_1_1_1w/openssl-1.1.1w.tar.gz
RUN tar -xzf openssl-1.1.1w.tar.gz
RUN cd openssl-1.1.1w && \
    CC=gcc ./Configure linux-aarch64 \
        --prefix=/opt/openssl \
        --cross-compile-prefix=aarch64-linux-gnu- \
        no-shared && \
    make -j$(nproc) && make install

ENV CROSS_TOOLCHAIN_PREFIX=aarch64-linux-gnu-
ENV CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_LINKER="$CROSS_TOOLCHAIN_PREFIX"gcc \
    AR_aarch64_unknown_linux_gnu="$CROSS_TOOLCHAIN_PREFIX"ar \
    CC_aarch64_unknown_linux_gnu="$CROSS_TOOLCHAIN_PREFIX"gcc \
    CXX_aarch64_unknown_linux_gnu="$CROSS_TOOLCHAIN_PREFIX"g++ \
    RUST_TEST_THREADS=1 \
    PKG_CONFIG_PATH="/usr/lib/aarch64-linux-gnu/pkgconfig/:/opt/openssl/lib/pkgconfig:${PKG_CONFIG_PATH}"

ENV OPENSSL_DIR=/opt/openssl
ENV OPENSSL_INCLUDE_DIR=/opt/openssl/include
ENV OPENSSL_LIB_DIR=/opt/openssl/lib
