#!/bin/sh
# Builds sc-gtkstyle (needs GTK 3's development files) into the folder given,
# beside the app's programs, where the app looks for it. CC and PKG_CONFIG
# name another compiler's, to build for another processor.
set -eu
OUT=${1:-target/debug}
mkdir -p "$OUT"
${CC:-cc} -O2 -std=c11 "$(dirname "$0")/sc-gtkstyle.c" -o "$OUT/sc-gtkstyle" $(${PKG_CONFIG:-pkg-config} --cflags --libs gtk+-3.0)
echo "$OUT/sc-gtkstyle"
