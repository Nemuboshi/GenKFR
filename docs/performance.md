# Resize and Windows encoder performance

GenKFR uses a fixed height of 360 pixels for scene detection.
Each frame passes through decoding, three plane resizes and xvid encoding in that order.
A preparation worker decodes and resizes upcoming frames while the main thread encodes prepared frames.
The encoder receives every frame in source order and keeps one continuous history.
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
The sequential version measured below passes the borrowed output directly to xvid, without a final `clone_into` copy.

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

## Overlapping preparation and encoding

The preparation worker owns the decoder and all three resizers.
The main thread owns the encoder.
Three reusable sets of resized planes pass between these threads.
The queue holds at most two prepared frames.
This limit lets preparation run ahead without storing a whole video in memory.
The resizers exchange their output buffers with recycled frame buffers before preparing the next frame.

The encoder receives frames in source order with the same thread count and slice count as before.
If either thread fails, the application disconnects the queues and joins the worker before returning the error.
The application writes the keyframe file only after all frames succeed.
The overlap change added no dependency or unsafe code.

Each run prints processing speed, stage times and queue waits.
Processing speed includes indexing, setup, frame processing and worker cleanup.
It excludes writing the keyframe file.
Setup time covers pool creation, decoder opening, source indexing and resizer creation.
Decode time covers frame retrieval and copying decoded planes.
Resize time includes exchanging completed output buffers with the reusable frame buffers.
The historical comparisons below included a copy at this handoff.
Encode time covers the encoder call for each frame.
These times measure elapsed time, not CPU time, and exclude waits for queues.

The preparation wait measures time waiting for a free buffer or space in the prepared frame queue.
The encoder wait measures time waiting for a prepared frame after encoder creation.
A large preparation wait with a small encoder wait points to encoding as the limiting stage.
A large encoder wait points to preparation as the limiting stage.
Compare decode and resize times to locate the slower part of preparation.
Concurrent stages share CPU and memory resources, so their times can change when they overlap.
Stage times overlap and must not be added to calculate total runtime.

The local comparison used the existing sequential release executable and the new pipeline release executable.
Both ran on the same host with 16 logical processors.
The first two inputs contained 600 frames each, copied from the local videos without re-encoding.
The third input contained all 247 frames.
Each executable ran once per input, with the pipeline running first for the second input.
These samples establish a local improvement, but repeated measurements are needed for a stable speed estimate.

| Input | Sequential speed | Pipeline speed | Speed increase | Sequential CPU use | Pipeline CPU use |
|---|---:|---:|---:|---:|---:|
| H.264, 960×432 | 62.97 fps | 65.71 fps | 4% | 12.4% | 18.3% |
| HEVC, 1280×576 | 51.55 fps | 64.25 fps | 25% | 12.2% | 16.8% |
| HEVC, 720×1280 | 62.03 fps | 75.63 fps | 22% | 13.3% | 21.1% |

Speed includes process startup and output writing in this comparison.
CPU use is the process CPU time divided by elapsed time and 16 logical processors.
It is an average across the whole run, including native worker threads.

The pipeline spent 8.603, 8.937 and 2.913 seconds resizing the three samples.
Decode time was 0.178, 0.210 and 0.125 seconds.
Encode time was 6.408, 5.100 and 1.902 seconds.
The encoder waited for prepared frames for 2.515, 4.066 and 1.142 seconds.
Preparation waited for buffers for only 0.159, 0.011 and 0.005 seconds.
These measurements identify the serial resizer as the limiting stage on these inputs.
The pipeline improves overlap but does not reach full CPU utilization.

All sample keyframe files matched byte for byte between executables.
The pipeline also processed the complete local videos with 5,745, 965 and 247 frames.
Their keyframe files matched the saved baseline byte for byte, with 57, 4 and 1 keyframes.
The complete runs served as output checks and are not part of the speed comparison.

The new pipeline test compares sequential and overlapping encoding on a generated video.
It covers one and 16 encoder threads, fractional crops and repeated buffer use.
It also makes sure that preparation exits when the consumer disconnects and returns errors for invalid crops and input data.
The current Windows release suite passes 19 tests, and strict Clippy checks pass.
The earlier ASan results below predate this pipeline change.

## Parallel resize passes

Each video creates one persistent Rayon pool for all three plane resizers.
The pool uses at most four workers, capped by the thread count that xvid detects.
The horizontal pass divides the intermediate buffer into groups of complete output rows.
Workers read the source plane and shared filter coefficients, then write separate groups.
The horizontal pass finishes before any vertical task reads its results.
The vertical pass also divides its output into groups of complete rows.
Each vertical group uses its own reusable float accumulation row.

Each pixel keeps its previous tap order and 8-bit rounding after each pass.
The pool borrows the source plane, coefficients and output buffers during each call.
The implementation adds no input copies and no unsafe code.
The pipeline exchanges completed output buffers with its three reusable frame buffers instead of copying the pixels.
The pool exits with the preparation worker, including when the encoder disconnects.

Rayon supplies the persistent workers and safe scheduling of borrowed row groups.
The application creates the pool once per video and reuses scratch memory across frames.
The resizer accepts a pool so that byte comparisons can exercise different worker counts.
No command-line configuration is added.
The native decoder and encoder keep their existing thread counts and slice counts.

The scalar comparison tests run with one, two, four and eight resize workers.
They cover production dimensions, uneven row groups, more workers than rows, fractional crops, source padding and repeated buffer use.
The rounding test also runs at every worker count.
The pipeline test compares ordered encoding with a one-worker resize reference.
All 19 Windows release tests and strict Clippy checks pass.
The earlier ASan results below predate the parallel resize change.

A resize-only comparison used 120 decoded frames from each local video.
It alternated worker counts between frames and compared all Y, U and V bytes outside the timed sections.
Every output byte matched the one-worker result.
The table gives the combined time for all three planes.

| Input | One worker | Two workers | Four workers | Eight workers |
|---|---:|---:|---:|---:|
| H.264, 960×432 | 2.280 s | 1.545 s | 1.132 s | 0.948 s |
| HEVC, 1280×576 | 3.200 s | 2.025 s | 1.226 s | 0.912 s |
| HEVC, 720×1280 | 1.585 s | 1.032 s | 0.619 s | 0.408 s |

The pipeline comparison used the previous executable with serial resizing as its baseline.
The parallel executable used the same decoder, encoder, thread counts and three frame buffers.
Only the resize worker count varied between parallel runs.
Each input ran with serial resizing, two workers, four workers and eight workers, then repeated this order in reverse.
The table reports the median processing speed from two runs per configuration.
These measurements include process startup and output writing.
Background activity caused substantial variation between runs.
These results describe local samples and do not guarantee the same speed increase on other workloads.

| Input | Serial resize pipeline | Two workers | Four workers | Eight workers |
|---|---:|---:|---:|---:|
| H.264, 960×432, 600 frames | 42.68 fps | 57.44 fps | 80.86 fps | 77.80 fps |
| HEVC, 1280×576, 600 frames | 40.95 fps | 58.15 fps | 50.63 fps | 64.09 fps |
| HEVC, 720×1280, 247 frames | 61.39 fps | 79.87 fps | 98.10 fps | 84.55 fps |

Four workers won on two inputs, while eight workers won on the second input.
The four-worker cap balances resize speed with CPU capacity for decoding and encoding.
Four workers increased median CPU use from 14.7%, 14.1% and 19.7% to 32.3%, 22.8% and 37.1%.
Encoding took longer than resizing in all six four-worker runs.
The encoder waited for prepared frames for 0.011–0.707 seconds per run.
Encoding is now the main limiting stage on these tested samples.
The pipeline still does not reach full CPU utilization.

All 24 pipeline comparison runs produced matching keyframe files for their input.
The production executable also matched the saved baselines for the complete videos with 5,745, 965 and 247 frames.
The complete H.264 and portrait runs spent more time encoding than resizing.
The complete HEVC run remained slightly limited by preparation, with 11.478 seconds resizing and 11.057 seconds encoding.
The largest stage can therefore still depend on the input and system activity.

## Resize dispatch and buffer handoff

The preparation thread enters the resize pool once per frame for all three planes.
Nested calls to the same pool run directly on the current worker.
Previously each plane required a separate external dispatch and completion wait.
Each plane still completes its horizontal pass before its vertical pass.
The pass boundaries protect the shared intermediate buffer.
The ready-frame and recycled-buffer queues still bound preparation and preserve frame order.

After resizing, the resizer exchanges its output vector with the recycled frame vector.
The application transfers buffer ownership without copying pixel bytes.
On first use, each empty recycled vector grows to the fixed output size.
After warmup, these allocations circulate between the resizers and frame buffers.
The exchange does not affect other frames held by the encoder or queue.
This change adds no dependency or unsafe code.

The buffer exchange test covers one, four and eight resize workers.
It compares every held frame against scalar output and makes sure that allocation addresses remain stable after warmup.
It also makes sure that invalid input leaves the destination unchanged.
All 20 Windows release tests and strict Clippy checks pass.

CPU utilization alone cannot measure efficiency.
Measure frames per second together with process CPU time per frame.
For a fixed processor count, utilization equals throughput multiplied by CPU time per frame, divided by the processor count.
A higher CPU cost per frame can increase utilization without a proportional increase in throughput.
Queue wait measures elapsed idle time and does not establish the CPU cost of synchronization.
Use a CPU profile to separate scheduler overhead, native encoding and cache or memory contention before adding more workers.
The local xvidcore 1.3.7 source creates and joins workers inside its frame encoding paths.
On Windows these operations map to `CreateThread` and `WaitForSingleObject`.
A persistent native worker pool is a candidate for further profiling and optimization.
It must preserve task completion, slice configuration and frame order to maintain reference output.

The focused resize-and-handoff comparison alternated the previous and current paths for each decoded frame.
Both paths used four workers, the same input frame and the same arithmetic.
It compared all held Y, U and V outputs outside timed sections.
All 720 tested frames matched byte for byte.

| Input | Frames | Previous resize and handoff | Current resize and handoff |
|---|---:|---:|---:|
| H.264, 960×432 | 240 | 1.396 s | 1.261 s |
| HEVC, 1280×576 | 240 | 1.805 s | 1.718 s |
| HEVC, 720×1280 | 240 | 1.519 s | 1.464 s |

The complete pipeline comparison ran previous, current, current and previous executables for each sample.
It produced matching keyframe files in all 12 runs.
The table gives the median of two runs per executable.
Process CPU time includes all native and Rust worker threads.

| Input | Previous speed | Current speed | Previous CPU time per frame | Current CPU time per frame |
|---|---:|---:|---:|---:|
| H.264, 600 frames | 98.39 fps | 100.68 fps | 54.78 ms | 56.38 ms |
| HEVC, 600 frames | 51.96 fps | 36.46 fps | 73.95 ms | 72.68 ms |
| HEVC portrait, 247 frames | 82.09 fps | 84.45 fps | 62.94 ms | 62.91 ms |

The focused test shows lower elapsed time for resize and handoff.
The complete pipeline results remain mixed and do not establish an overall speed or CPU efficiency improvement.
Background activity and contention affect these runs, and the measurements do not isolate their contribution.
The implementation retains the dispatch reduction and buffer exchange because the focused comparison improves all three inputs.
The current executable also matches the saved baseline keyframe files for all three complete videos, totaling 6,957 frames.
All three complete runs spend more time encoding than resizing.
These complete runs establish output equivalence and are not a controlled speed comparison.

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
