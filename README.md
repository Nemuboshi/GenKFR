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

`genkfr.exe` links everything statically, so the release has no DLL files.

FFMS2 is a git submodule in `vendor/ffms2`, pinned to a commit. `build.rs`
compiles its core sources, because FFMS2 is normally built as a DLL and GenKFR
links it statically. Clone with submodules:

```bash
git clone --recursive <repository>
```

Two things must be available:

- **FFmpeg**, for its headers and static libraries. Set `FFMPEG_DIR` to the
  prefix that holds `include/` and `lib/`. `build.rs` also looks in a vcpkg
  tree when `VCPKG_ROOT` is set.
- **xvidcore.lib**, from xvidcore's sources. Put it on the library path with
  `RUSTFLAGS`.

```bash
FFMPEG_DIR=/path/to/ffmpeg RUSTFLAGS="-L native=/path/to/xvidcore" cargo build --release
```

The workflow in `.github/workflows/windows-release.yml` builds both from
source. See it for the exact flags.
