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

`genkfr.exe` links everything statically, so the release has no DLL files. The
build needs three libraries:

- `ffms2.lib`, built from FFMS2's `src/core` sources
- `xvidcore.lib`, built from xvidcore's sources
- FFmpeg's static libraries, which FFMS2 needs

The workflow in `.github/workflows/windows-release.yml` builds all three from
source. To build by hand, put the three `.lib` files in one directory and point
`rustflags` at it:

```bash
RUSTFLAGS="-L native=/path/to/libs" cargo build --release
```

FFMS2 also needs the system libraries listed in the workflow's build step.
