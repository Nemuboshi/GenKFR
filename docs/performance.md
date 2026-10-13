# Resize and Windows encoder performance

GenKFR uses a fixed height of 360 pixels for scene detection.
Each frame passes through decoding, three plane resizes and xvid encoding in that order.
The measurements below separate a native build fix from a resize optimization.

## Windows xvid build

The Windows build defines `HAVE_PTHREAD`, as xvidcore 1.3.7's official Windows project does.
On Windows, xvid maps these calls to `CreateThread` and `WaitForSingleObject`.
The build script makes sure that the encoder object references `CreateThread`.
Without this macro, requesting multiple threads does not compile in the worker code.
The thread and slice arguments remain unchanged.

The fixed library passed the native wrapper tests.
Complete local videos with 5,745, 965 and 247 frames produced the same keyframe lists as the previous baseline.
Their keyframe counts remained 57, 4 and 1.
The resize measurements use this fixed library for both versions.

## Resize implementation

Each video plane has a `Resizer` that stores its filter coefficients and work buffers.
The application builds these coefficients once for the fixed dimensions and crop.
It reads input rows directly, without copying the source region into another buffer.
It passes the borrowed output directly to xvid, without a final `clone_into` copy.

The horizontal filter bounds its pixel and weight slices before the tap loop.
The vertical filter accumulates each source row across a reusable float row.
This layout exposes contiguous work to the compiler's vectorizer.
Each pixel keeps the previous tap order.
Both passes keep the previous conversion to 8-bit pixels and rounding behavior.
The implementation adds no dependency or unsafe code.

## Local measurements

The host was an Intel Core 7 240H with 16 logical processors.
The measurements used Windows release builds and requested 16 decode and encode threads.
Source dimensions were 960×432, 1280×576 and 720×1280.
Background activity caused absolute speed to vary.
These results describe the tested samples, rather than a guarantee for all videos.

The resize comparison ran the old and new code in the same process.
It alternated their order on each frame and compared all Y, U and V bytes outside the timed sections.
All 1,447 tested frames matched exactly.
The old code included its final buffer copy, as the previous application did.

| Input | Frames | Previous resize time | New resize time | Resize speed ratio |
|---|---:|---:|---:|---:|
| H.264, 960×432 | 600 | 7.611 s | 5.217 s | 1.46× |
| HEVC, 1280×576 | 600 | 6.907 s | 5.068 s | 1.36× |
| HEVC, 720×1280 | 247 | 2.163 s | 1.550 s | 1.40× |

The pipeline comparison used the same fixed xvid library for both versions.
It ran the old, new, new and old versions in that order for each input.
The table gives the median of the two runs for each version.
Timing includes source indexing, setup, decoding, resizing and encoding, but excludes writing the small keyframe file.
All keyframe lists matched between runs.

| Input | Frames per run | Previous pipeline | New pipeline | Speed increase |
|---|---:|---:|---:|---:|
| H.264, 960×432 | 600 | 42.67 fps | 48.23 fps | 13% |
| HEVC, 1280×576 | 600 | 53.36 fps | 59.72 fps | 12% |
| HEVC, 720×1280 | 247 | 63.40 fps | 80.66 fps | 27% |

## Validation

Release tests passed on Windows, Windows native ASan and Linux with Rust and native ASan.
Each run passed 18 tests: three resize tests and 15 native wrapper tests.
The encoder noise test includes the production size of 800×360, a request for 16 threads and multiple strides.
The resize tests cover scalar pixel equivalence, repeated buffer use, fractional crops, odd dimensions, padding and two-pass rounding.
The Linux ASan media corpus also passed.
Strict Clippy checks passed without warnings.

The optimized application matched the fixed encoder baseline for all three complete local videos.
These local video fixtures are not part of the committed test suite.
ASan remains a separate manual workflow.
Handwritten assembly is not instrumented in that workflow.
