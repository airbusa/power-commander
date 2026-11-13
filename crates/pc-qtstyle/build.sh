#!/bin/sh
# Builds pc-qtstyle (needs Qt 6's development files) into the folder given,
# beside the app's programs, where the app looks for it. CXX and PKG_CONFIG
# name another compiler's, to build for another processor.
set -eu
OUT=${1:-target/debug}
mkdir -p "$OUT"
${CXX:-c++} -O2 -std=c++17 -fPIC "$(dirname "$0")/pc-qtstyle.cpp" -o "$OUT/pc-qtstyle" $(${PKG_CONFIG:-pkg-config} --cflags --libs Qt6Widgets)
echo "$OUT/pc-qtstyle"
