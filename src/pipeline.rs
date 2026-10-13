//! Decode and resize ahead of a single, ordered encoder.

use std::path::Path;
use std::sync::mpsc::{Receiver, SyncSender, sync_channel};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow};
use genkfr_media::{Decoder, Xvid};

use crate::{TARGET_HEIGHT, align_width_4, scale};
use scale::Crop;

#[derive(Default)]
struct PreparedFrame {
    y: Vec<u8>,
    u: Vec<u8>,
    v: Vec<u8>,
}

struct VideoInfo {
    width: i32,
    frames: i32,
    fps: f64,
}

#[derive(Default)]
pub struct Timings {
    pub setup: Duration,
    pub decode: Duration,
    pub resize: Duration,
    pub buffer_wait: Duration,
    pub encode: Duration,
    pub frame_wait: Duration,
    pub total: Duration,
}

pub struct Analysis {
    pub frames: i32,
    pub fps: f64,
    pub keyframes: Vec<i32>,
    pub timings: Timings,
}

pub fn analyze(input: &Path, crop: Crop, threads: i32) -> Result<Analysis> {
    let start = Instant::now();
    let (info_tx, info_rx) = sync_channel(1);
    let (ready_tx, ready_rx) = sync_channel(2);
    let (free_tx, free_rx) = sync_channel(3);
    for _ in 0..3 {
        free_tx.send(PreparedFrame::default())?;
    }

    std::thread::scope(|scope| {
        // Native decoder handles stay on the thread that creates them.
        let worker = scope.spawn(move || prepare(input, crop, threads, info_tx, ready_tx, free_rx));
        let consumed = (|| -> Result<Option<Analysis>> {
            let Ok(info) = info_rx.recv() else {
                return Ok(None);
            };
            let mut encoder = Xvid::new(info.width, TARGET_HEIGHT, threads)?;
            let mut keyframes = Vec::new();
            let mut timings = Timings::default();
            for number in 0..info.frames {
                if number % 1000 == 0 {
                    eprintln!("processing frame {number}/{}", info.frames);
                }
                let wait = Instant::now();
                let Ok(frame) = ready_rx.recv() else {
                    return Ok(None);
                };
                timings.frame_wait += wait.elapsed();
                let encode = Instant::now();
                if encoder.is_keyframe(&frame.y, &frame.u, &frame.v, info.width, info.width / 2)? {
                    keyframes.push(number);
                }
                timings.encode += encode.elapsed();
                // The producer may already have sent its last frame and exited.
                let _ = free_tx.send(frame);
            }
            Ok(Some(Analysis {
                frames: info.frames,
                fps: info.fps,
                keyframes,
                timings,
            }))
        })();

        // Disconnect both queues before joining, including on encoder failure.
        drop(info_rx);
        drop(ready_rx);
        drop(free_tx);
        let produced = worker
            .join()
            .map_err(|_| anyhow!("frame preparation panicked"));
        let consumed = consumed?;
        let producer_timings = produced??;
        let mut analysis = consumed.context("frame preparation ended before all frames arrived")?;
        analysis.timings.setup = producer_timings.setup;
        analysis.timings.decode = producer_timings.decode;
        analysis.timings.resize = producer_timings.resize;
        analysis.timings.buffer_wait = producer_timings.buffer_wait;
        analysis.timings.total = start.elapsed();
        Ok(analysis)
    })
}

fn prepare(
    input: &Path,
    crop: Crop,
    threads: i32,
    info_tx: SyncSender<VideoInfo>,
    ready_tx: SyncSender<PreparedFrame>,
    free_rx: Receiver<PreparedFrame>,
) -> Result<Timings> {
    let setup = Instant::now();
    // Leave CPU capacity for the overlapping native decoder and encoder.
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(threads.clamp(1, 4) as usize)
        .build()
        .context("cannot create resize workers")?;
    let mut decoder = Decoder::open(input, threads)?;
    let target_w = align_width_4(decoder.width(), decoder.height(), TARGET_HEIGHT);
    let source_w = decoder.width() as u32;
    let source_h = decoder.height() as u32;
    let mut luma = scale::Resizer::new(
        source_w,
        source_h,
        target_w as u32,
        TARGET_HEIGHT as u32,
        crop,
    )?;
    let half_crop = Crop {
        left: crop.left.map(|v| v / 2.0),
        top: crop.top.map(|v| v / 2.0),
        width: crop.width.map(|v| v / 2.0),
        height: crop.height.map(|v| v / 2.0),
    };
    let mut chroma_u = scale::Resizer::new(
        source_w.div_ceil(2),
        source_h.div_ceil(2),
        (target_w as u32).div_ceil(2),
        (TARGET_HEIGHT as u32).div_ceil(2),
        half_crop,
    )?;
    let mut chroma_v = scale::Resizer::new(
        source_w.div_ceil(2),
        source_h.div_ceil(2),
        (target_w as u32).div_ceil(2),
        (TARGET_HEIGHT as u32).div_ceil(2),
        half_crop,
    )?;
    let mut timings = Timings {
        setup: setup.elapsed(),
        ..Timings::default()
    };
    info_tx.send(VideoInfo {
        width: target_w,
        frames: decoder.num_frames(),
        fps: decoder.fps(),
    })?;

    for number in 0..decoder.num_frames() {
        let wait = Instant::now();
        let mut prepared = free_rx.recv().context("encoder stopped")?;
        timings.buffer_wait += wait.elapsed();
        let decode = Instant::now();
        let frame = decoder.frame(number)?;
        timings.decode += decode.elapsed();
        let resize = Instant::now();
        // Enter the pool once; each plane's nested install runs on this same pool.
        pool.install(|| -> Result<()> {
            luma.resize_into(
                scale::Plane {
                    data: frame.y,
                    stride: frame.y_stride,
                },
                &pool,
                &mut prepared.y,
            )?;
            chroma_u.resize_into(
                scale::Plane {
                    data: frame.u,
                    stride: frame.uv_stride,
                },
                &pool,
                &mut prepared.u,
            )?;
            chroma_v.resize_into(
                scale::Plane {
                    data: frame.v,
                    stride: frame.uv_stride,
                },
                &pool,
                &mut prepared.v,
            )?;
            Ok(())
        })?;
        timings.resize += resize.elapsed();
        let wait = Instant::now();
        ready_tx.send(prepared).context("encoder stopped")?;
        timings.buffer_wait += wait.elapsed();
    }
    Ok(timings)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pipeline_matches_sequential_encoding_with_reused_buffers() {
        let path = std::env::temp_dir().join(format!("genkfr-pipeline-{}.y4m", std::process::id()));
        let mut input = b"YUV4MPEG2 W16 H16 F24:1 Ip A1:1 C420jpeg\n".to_vec();
        for number in 0..12u8 {
            input.extend_from_slice(b"FRAME\n");
            for pixel in 0..256 {
                input.push(if number < 4 {
                    16
                } else if number < 8 {
                    235
                } else {
                    (pixel as u8).wrapping_mul(37).wrapping_add(number)
                });
            }
            input.extend_from_slice(&[128; 128]);
        }
        std::fs::write(&path, input).unwrap();
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(1)
            .build()
            .unwrap();
        for threads in [1, 16] {
            for crop in [
                Crop::default(),
                Crop {
                    left: Some(0.5),
                    top: Some(0.5),
                    width: Some(14.0),
                    height: Some(14.0),
                },
            ] {
                let mut decoder = Decoder::open(&path, threads).unwrap();
                let mut encoder = Xvid::new(360, 360, threads).unwrap();
                let half_crop = Crop {
                    left: crop.left.map(|v| v / 2.0),
                    top: crop.top.map(|v| v / 2.0),
                    width: crop.width.map(|v| v / 2.0),
                    height: crop.height.map(|v| v / 2.0),
                };
                let mut y = scale::Resizer::new(16, 16, 360, 360, crop).unwrap();
                let mut u = scale::Resizer::new(8, 8, 180, 180, half_crop).unwrap();
                let mut v = scale::Resizer::new(8, 8, 180, 180, half_crop).unwrap();
                let mut expected = Vec::new();
                for number in 0..decoder.num_frames() {
                    let frame = decoder.frame(number).unwrap();
                    let y = y
                        .resize(
                            scale::Plane {
                                data: frame.y,
                                stride: frame.y_stride,
                            },
                            &pool,
                        )
                        .unwrap();
                    let u = u
                        .resize(
                            scale::Plane {
                                data: frame.u,
                                stride: frame.uv_stride,
                            },
                            &pool,
                        )
                        .unwrap();
                    let v = v
                        .resize(
                            scale::Plane {
                                data: frame.v,
                                stride: frame.uv_stride,
                            },
                            &pool,
                        )
                        .unwrap();
                    if encoder.is_keyframe(y, u, v, 360, 180).unwrap() {
                        expected.push(number);
                    }
                }
                let actual = analyze(&path, crop, threads).unwrap();
                assert_eq!(actual.frames, 12);
                assert_eq!(actual.fps, 24.0);
                assert_eq!(actual.keyframes, expected);
                assert!(actual.timings.total >= actual.timings.encode);
            }
        }
        let (info_tx, info_rx) = sync_channel(1);
        let (ready_tx, ready_rx) = sync_channel(2);
        let (free_tx, free_rx) = sync_channel(3);
        for _ in 0..3 {
            free_tx.send(PreparedFrame::default()).unwrap();
        }
        std::thread::scope(|scope| {
            let worker =
                scope.spawn(|| prepare(&path, Crop::default(), 1, info_tx, ready_tx, free_rx));
            info_rx.recv().unwrap();
            ready_rx.recv().unwrap();
            drop(info_rx);
            drop(ready_rx);
            drop(free_tx);
            assert!(worker.join().unwrap().is_err());
        });
        let invalid_crop = Crop {
            width: Some(17.0),
            ..Crop::default()
        };
        assert!(
            analyze(&path, invalid_crop, 1)
                .err()
                .unwrap()
                .to_string()
                .contains("outside")
        );
        std::fs::write(&path, b"not a video").unwrap();
        assert!(analyze(&path, Crop::default(), 1).is_err());
        std::fs::remove_file(path).unwrap();
    }
}
