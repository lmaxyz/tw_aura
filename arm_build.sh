#!/bin/bash

set -e

if [ -z "$PSDK_DIR" ]; then
    echo '`PSDK_DIR` not found.';
    exit 1;
fi


CURRENT_DIR="$(pwd)";
TARGET=armv7-unknown-linux-gnueabihf

aurora_psdk="$PSDK_DIR/sdk-chroot"
PKG_VERSION="$(cargo pkgid | cut -d '#' -f 2)"

mkdir -p RPMS/

CARGO_FEATURE_STATIC=1 cross build --release --target $TARGET
cp target/$TARGET/release/tw_aura ./com.lmaxyz.TwAura
# cargo generate-rpm -a aarch64 --target $TARGET -o RPMS/
$aurora_psdk mb2 --target AuroraOS-5.1.3.85-base-armv7hl build
rm ./com.lmaxyz.TwAura

$aurora_psdk rpmsign-external sign -k $PSDK_DIR/../../certs/lmaxyz_key.pem -c $PSDK_DIR/../../certs/lmaxyz_cert.pem "$CURRENT_DIR/RPMS/com.lmaxyz.TwAura-$PKG_VERSION-1.armv7hl.rpm"
$aurora_psdk rpm-validator -p regular "$CURRENT_DIR/RPMS/com.lmaxyz.TwAura-$PKG_VERSION-1.armv7hl.rpm"

rm documentation.list
