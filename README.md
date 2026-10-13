# GenKFR

Makes an Aegisub keyframe file for each input video. The keyframes come from
SCXvid's scene detection: xvidcore's `MEanalysis` frame type decision. GenKFR
calls `xvid_encore` directly, so it needs no VapourSynth and no plugin.

```bash
genkfr "C:/video/input.mkv"
genkfr "C:/video/input1.mkv" "C:/video/input2.mkv"
```

Output:

```txt
<input>.kf.txt
```

## Crop

`--crop` takes four values, the same ones as the crop arguments of the
VapourSynth `resize` filter: `src_left`, `src_top`, `src_width` and
`src_height`.

```bash
genkfr --crop 40,0,-,- "C:/video/input.mkv"
```

Use `-` for a value to keep its default. The default `src_left` and `src_top`
are zero, and the default `src_width` and `src_height` are the rest of the
frame. The crop selects the region that the filter samples. It does not change
the size of the output.

## Build

The native libraries link statically into `genkfr.exe`. The program does not
need FFmpeg, FFMS2 or xvidcore DLL files. It still needs the Microsoft Visual
C++ runtime.

`crates/genkfr-media` provides the safe Rust interfaces for decoding and scene
detection. Its native declarations stay private. bindgen generates them from
the library headers during each build, including checks for structure layouts.
The application rejects `unsafe` code.

FFMS2 is a git submodule in `vendor/ffms2`, pinned to a commit. The binding
package's `build.rs` compiles its core sources. Clone with submodules:

```bash
git clone --recursive <repository>
```

Builds currently support Windows x64 with MSVC. Run Cargo in an MSVC developer
environment. Set these paths to native Windows paths, such as `D:/libs/ffmpeg`:

- `FFMPEG_DIR`: the directory with FFmpeg headers in `include/` and static
  libraries in `lib/`. The build script also accepts a vcpkg installation
  through `VCPKG_ROOT`.
- `XVIDCORE_DIR`: the directory containing both `xvid.h` and the static
  `xvidcore.lib`. Use the header from the sources that built that library.
  The encoder currently requires xvidcore 1.3.7.
- `LIBCLANG_PATH`: the directory containing `libclang.dll` from LLVM.
  libclang reads the headers for bindgen. It is a build dependency, not a
  runtime dependency.

```bash
FFMPEG_DIR=D:/libs/ffmpeg XVIDCORE_DIR=D:/libs/xvidcore LIBCLANG_PATH='C:/Program Files/LLVM/bin' cargo build --release --locked
```

`.github/scripts/build-xvid.sh` builds xvidcore and places its header beside
the static library. The release workflow uses this script. The independent
test workflow is `.github/workflows/tests.yml`:

```bash
cargo test --workspace --release --locked
```
