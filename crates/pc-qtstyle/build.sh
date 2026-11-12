#!/bin/sh
# Builds pc-qtstyle (needs Qt 6's development files) into the folder given,
# beside the app's programs, where the app looks for it.
set -eu
OUT=${1:-target/debug}
mkdir -p "$OUT"
c++ -O2 -std=c++17 -fPIC "$(dirname "$0")/pc-qtstyle.cpp" -o "$OUT/pc-qtstyle" $(pkg-config --cflags --libs Qt6Widgets)
echo "$OUT/pc-qtstyle"
