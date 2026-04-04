FROM debian:12
ARG DEBIAN_FRONTEND=noninteractive

RUN apt-get update && apt-get install --assume-yes --no-install-recommends \
    build-essential wget tar git pkg-config nasm \
    gcc-arm-linux-gnueabihf g++-arm-linux-gnueabihf \
    libc6-dev-armhf-cross

# Сборка OpenSSL 1.1.1
# RUN wget --no-check-certificate https://github.com/openssl/openssl/releases/download/OpenSSL_1_1_1w/openssl-1.1.1w.tar.gz
# RUN tar -xzf openssl-1.1.1w.tar.gz
# RUN cd openssl-1.1.1w && \
#     CC=gcc ./Configure linux-aarch64 \
#         --prefix=/opt/openssl \
#         --cross-compile-prefix=aarch64-linux-gnu- \
#         no-shared && \
#     make -j$(nproc) && make install

ENV CROSS_TOOLCHAIN_PREFIX=arm-linux-gnueabihf-
ENV CROSS_SYSROOT=/usr/arm-linux-gnueabihf
ENV CROSS_TARGET_RUNNER="/linux-runner armv7hf"
ENV CARGO_TARGET_ARMV7_UNKNOWN_LINUX_GNUEABIHF_LINKER="$CROSS_TOOLCHAIN_PREFIX"gcc \
    AR_armv7_unknown_linux_gnueabihf="$CROSS_TOOLCHAIN_PREFIX"ar \
    CC_armv7_unknown_linux_gnueabihf="$CROSS_TOOLCHAIN_PREFIX"gcc \
    CXX_armv7_unknown_linux_gnueabihf="$CROSS_TOOLCHAIN_PREFIX"g++ \
    RUST_TEST_THREADS=1 \
    PKG_CONFIG_PATH="/usr/lib/arm-linux-gnueabihf/pkgconfig/:/opt/openssl/lib/pkgconfig:${PKG_CONFIG_PATH}"

# ENV OPENSSL_DIR=/opt/openssl
# ENV OPENSSL_INCLUDE_DIR=/opt/openssl/include
# ENV OPENSSL_LIB_DIR=/opt/openssl/lib
