# Native memory safety

This document covers the encoder output allocation in `crates/genkfr-media/src/xvid.rs`.
It does not prove that the complete C/C++ libraries are memory-safe.

## Supported encoder settings

The capacity calculation applies to xvidcore 1.3.7 with the settings in `Xvid::new` and `Xvid::is_keyframe`:

- No B-frames, plugins, packed output or frame dropping.
- No custom quantization matrices, interlacing, global motion compensation or quarter-pixel motion.
- Six blocks per macroblock. A macroblock is a 16 by 16 luma region plus its chroma pixels.
- Fixed `FINCR = FBASE = 1`. Each submitted frame advances time by one second in the encoder's internal clock.

`Xvid::new` rejects other library versions. Its dimensions must be positive and even.
`buffer_size` rejects allocations larger than `c_int::MAX` bytes.
If any encoder setting or the native implementation changes, review this calculation before using it.

## Alignment and initialized memory

`BitstreamInit` in xvidcore's `src/bitstream/bitstream.h` rounds the supplied address down to a four-byte boundary.
It then reads two `uint32_t` words, including when the encoder starts an empty output stream.
A `Vec<u8>` does not guarantee four-byte alignment.

The output is a zero-initialized `Vec<u32>`. Its allocation guarantees the required alignment, even with a custom Rust allocator.
The smallest allocation contains 6,144 bytes. Both initial word reads stay inside initialized memory.
The `length` passed to xvidcore counts bytes, not words. The encoder does not enforce that length.

## Capacity calculation

Let `M = ceil(width / 16) * ceil(height / 16)`.
The allocation contains `M * 2048 + 4096` bytes. Both terms are multiples of four.
Checked multiplication and addition prevent wrapping. The final byte count must fit `c_int` and therefore also fits `isize` on Windows x64.

The bound uses the writing paths in xvidcore's `release-1_3_7` sources:

- Coefficients: `CodeCoeffIntra`, `CodeCoeffInter` and the DC tables in `src/bitstream/mbcoding.c`.
  Each block has at most 64 coefficients. The escape code uses 30 bits. Reserve 32 bits per coefficient, including intra DC.
  Six blocks therefore need at most `6 * 64 * 32 / 8 = 1536` bytes per macroblock.
- Motion: `CodeBlockInter` writes at most four vectors with two components each.
  `CodeVector` writes a motion VLC and residual bits. A VLC is a variable-length bit code.
  Reserve 64 bits per component, which exceeds the VLC plus residual bits for valid native motion codes.
  Eight components therefore need at most 64 bytes per macroblock.
- Macroblock control: `MBCoding`, `CodeBlockIntra` and `CodeBlockInter` write the coded flag, block pattern and quantizer change.
  With the supported settings, these fields fit 32 bits. Reserve four bytes per macroblock.
- Slice boundaries: `write_video_packet_header` in `src/bitstream/bitstream.c` writes a resync marker, macroblock number, quantizer and one flag.
  The allocation limit keeps `M` below `2^20`. The macroblock number therefore needs at most 20 bits.
  Reserve 16 bytes for each packet header and its byte padding. The encoder creates at most one slice per macroblock row.
  Charging one header per macroblock covers every slice and also covers boundaries from threaded slice output.
- Word writes: `BitstreamForward`, the final write in `enc_encode`, and `FrameOutput` write or read complete four-byte words.
  Reserve another eight bytes per macroblock for rounding at boundaries and the initial word reads of each stream.

The sum is `1536 + 64 + 4 + 16 + 8 = 1628` bytes per macroblock.
The 2048-byte allocation leaves 420 additional bytes per macroblock.

The separate 4096-byte allowance covers frame-level headers and userdata.
`BitstreamWriteVolHeader`, `BitstreamWriteVopHeader`, `BitstreamWriteUserData` and the start/stop headers are in `src/bitstream/bitstream.c`.
The selected VOL fields are fixed-width. Custom matrices and optional motion fields are disabled.
The userdata strings are fixed by xvidcore 1.3.7. The unary VOP time field stays short because each frame advances time by one second.
These headers stay below 4096 bytes with the supported settings. Frame-level word rounding also fits this allowance.

This bound assumes that xvidcore produces valid internal coefficients and motion codes, and follows its writing paths.
It does not bound writes caused by an independent bug inside xvidcore.

## Tests and their limits

Unit tests cover exact capacity values, the last byte-count range accepted by the calculation, and output pointer alignment.
Encoding tests use deterministic noise, dimensions around 16-pixel block boundaries, tight and padded strides, and multiple threads.
An identical-frame test exercises P-frame output after the initial I-frame. An I-frame contains the complete image; a P-frame uses a previous image.
A tail sentinel detects writes into the reserved end of the output. It does not detect out-of-bounds reads or writes beyond the allocation.
Other tests cover invalid planes, invalid files and audio-only files.

## ASan builds

AddressSanitizer detects illegal memory accesses in code compiled with its instrumentation.
The separate `ASan` workflow runs only when you trigger it manually through `workflow_dispatch`.
Normal pushes and pull requests run the Windows release tests.
Run ASan after changes to FFI, frame buffers, encoder configuration or native dependencies.
A previous successful run does not cover later changes.
The Linux job rebuilds FFmpeg 9.0.2, zlib 1.3.2 and xvidcore 1.3.7 from source.
FFmpeg uses no external codec libraries in this build.
FFMS2 uses the same instrumented libraries.
Clang compiles all these C/C++ sources with `-fsanitize=address`.
The build script makes sure that each static archive contains ASan symbols.

The Linux job compiles the project and its Rust dependencies with nightly Rust.
It uses `-Zsanitizer=address` and `-Zbuild-std` to instrument Rust code and rebuild the standard library.
The `-Zexternal-clangrt` flag selects the Clang ASan runtime for both languages.
Rust disables the linker's default libraries with `-nodefaultlibs`.
The `-C link-arg=-lstdc++` flag supplies the C++ runtime that Clang's sanitizer requires.
The explicit Linux target keeps build scripts and procedural macros outside the instrumented application.

Stable Rust does not expose ASan compiler instrumentation.
The [Rust sanitizer documentation](https://doc.rust-lang.org/unstable-book/compiler-flags/sanitizer.html) also excludes Windows MSVC from its supported ASan targets.
The Linux job therefore supplies Rust coverage for this project.
Windows release builds continue to use stable Rust.

Handwritten assembly cannot receive automatic ASan instrumentation.
The ASan builds compile xvidcore as `ARCH_IS_GENERIC`, without assembly routines or assembly CPU detection.
The Linux FFmpeg build disables both external and inline assembly.
These builds test the C implementations of the media algorithms.
They do not test the assembly implementations that normal release builds use.
Compiler-generated machine instructions from instrumented C/C++ code remain checked.
Operating system libraries and the system C/C++ runtime remain outside compiler instrumentation.

The Linux job first makes sure that ASan detects deliberate overflows in both Rust and C code.
Both probes access an allocation that Rust creates.
The job then runs the unit tests with heap, stack and leak detection enabled.
It also generates noise, odd-dimension and 10-bit videos, then processes them with the instrumented executable.
The damaged inputs contain a truncated packet, modified packet bytes and an incomplete header.
Damaged videos can produce a partial result or an application error.
A sanitizer report or process crash fails the job.
The workflow saves the media logs and FFmpeg configuration as an artifact.

To run the Linux build locally, install Clang, LLVM, libclang, autoconf, automake, libtool, pkg-config, curl, make, Python 3 and ffmpeg.
Use a fresh directory for `RUNNER_TEMP`.
Install nightly Rust with the `rust-src` component.
Run these commands from the repository root:

```bash
export RUNNER_TEMP="$(mktemp -d)"
export FFMPEG_VERSION=9.0.2 XVIDCORE_VERSION=1.3.7 ZLIB_VERSION=1.3.2
bash .github/scripts/build-asan-libs.sh
export GENKFR_ASAN=1 CC=clang CXX=clang++
export FFMPEG_DIR="$RUNNER_TEMP/asan-prefix"
export XVIDCORE_DIR="$FFMPEG_DIR/lib"
cp "$FFMPEG_DIR/include/xvid.h" "$XVIDCORE_DIR/"
export CARGO_TARGET_DIR="$PWD/target/asan-linux"
export RUSTFLAGS='-Zsanitizer=address -Zexternal-clangrt -C linker=clang++ -C link-arg=-fsanitize=address -C link-arg=-lstdc++ -C force-frame-pointers=yes'
export RUSTDOCFLAGS="$RUSTFLAGS"
export ASAN_OPTIONS='detect_leaks=1:detect_stack_use_after_return=1:strict_string_checks=1:exitcode=86'
bash .github/scripts/test-asan-runtime.sh
cargo +nightly test -Zbuild-std --target x86_64-unknown-linux-gnu --workspace --release --locked
cargo +nightly build -Zbuild-std --target x86_64-unknown-linux-gnu --release --locked
export GENKFR_ASAN_BIN="$CARGO_TARGET_DIR/x86_64-unknown-linux-gnu/release/genkfr"
bash .github/scripts/test-asan-media.sh
```

For the Windows native checks, set `GENKFR_ASAN=1` in an MSVC developer environment.
Run `.github/scripts/build-xvid.sh` with a fresh `RUNNER_TEMP` and the workflow's version variables.
Set `XVIDCORE_DIR` to that directory.
Keep the MSVC AddressSanitizer runtime directory on `PATH`.
Set `ASAN_OPTIONS=windows_hook_rtl_allocators=true` to track Rust's Windows heap allocations.
Then run:

```bash
GENKFR_ASAN=1 RUSTFLAGS="-C link-arg=/INFERASANLIBS" cargo test --workspace --release --locked
```

This Windows command instruments FFMS2 and xvidcore C code.
Its prebuilt FFmpeg libraries and stable Rust code remain uninstrumented.
Use the Linux job for the broader C/C++ and Rust checks.
Neither configuration proves that the whole program is memory-safe.
