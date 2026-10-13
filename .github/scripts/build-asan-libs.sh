#!/usr/bin/env bash
set -euo pipefail

# Linux only: build every media dependency with the same Clang ASan runtime.
: "${RUNNER_TEMP:?set RUNNER_TEMP to a fresh build directory}"
: "${FFMPEG_VERSION:?}"
: "${XVIDCORE_VERSION:?}"
: "${ZLIB_VERSION:?}"
prefix="$RUNNER_TEMP/asan-prefix"
sources="$RUNNER_TEMP/asan-sources"
mkdir -p "$prefix" "$sources"
export CC=clang CXX=clang++
export CFLAGS='-O1 -g -fsanitize=address -fno-omit-frame-pointer'
export CXXFLAGS="$CFLAGS" LDFLAGS='-fsanitize=address'
jobs="$(nproc)"

curl --fail --location --retry 3 "https://zlib.net/fossils/zlib-$ZLIB_VERSION.tar.gz" -o "$sources/zlib.tar.gz"
tar -xzf "$sources/zlib.tar.gz" -C "$sources"
(
  cd "$sources/zlib-$ZLIB_VERSION"
  ./configure --static --prefix="$prefix"
  make -j"$jobs"
  make install
)

curl --fail --location --retry 3 "https://ffmpeg.org/releases/ffmpeg-$FFMPEG_VERSION.tar.xz" -o "$sources/ffmpeg.tar.xz"
tar -xJf "$sources/ffmpeg.tar.xz" -C "$sources"
(
  cd "$sources/ffmpeg-$FFMPEG_VERSION"
  # No external codec libraries or assembly may escape instrumentation.
  PKG_CONFIG_LIBDIR="$prefix/lib/pkgconfig" ./configure \
    --prefix="$prefix" --cc="$CC" --cxx="$CXX" \
    --enable-static --disable-shared --disable-programs --disable-doc \
    --disable-autodetect --disable-network --disable-asm --disable-inline-asm \
    --enable-zlib --extra-cflags="$CFLAGS -I$prefix/include" \
    --extra-ldflags="$LDFLAGS -L$prefix/lib"
  make -j"$jobs"
  make install
  # Keep the effective configuration for review when a sanitizer test fails.
  cp config.h ffbuild/config.mak "$prefix/"
)

curl --fail --location --retry 3 \
  "https://downloads.xvid.com/downloads/xvidcore-$XVIDCORE_VERSION.tar.bz2" \
  -o "$sources/xvid.tar.bz2"
tar -xjf "$sources/xvid.tar.bz2" -C "$sources"
(
  cd "$sources/xvidcore/build/generic"
  if [[ ! -f configure ]]; then
    bash bootstrap.sh
  fi
  ./configure --disable-assembly --prefix="$prefix"
  grep -q '^ARCHITECTURE=-DARCH_IS_GENERIC$' platform.inc
  cp platform.inc "$prefix/xvid-platform.inc"
  make -j"$jobs" libxvidcore.a
  cp '=build/libxvidcore.a' "$prefix/lib/"
  cp ../../src/xvid.h "$prefix/include/"
)

# Fail if the static libraries are missing or any archive lacks ASan instrumentation.
for library in avcodec avformat avutil swscale swresample z xvidcore; do
  archive="$prefix/lib/lib$library.a"
  nm -u "$archive" > "$sources/$library.symbols"
  grep -q '__asan_' "$sources/$library.symbols"
done
grep -q '^#define HAVE_X86ASM 0$' "$prefix/config.h"
grep -q '^#define HAVE_INLINE_ASM 0$' "$prefix/config.h"
echo "ASan libraries built in $prefix"
