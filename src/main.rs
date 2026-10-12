//! GenKFR: make an Aegisub keyframe file for each input video.
//!
//! The scene detection is SCXvid's: xvidcore's `MEanalysis` frame type
//! decision. This program calls `xvid_encore` directly, so it needs no
//! VapourSynth and no plugin.

mod decode;
mod scale;
mod xvid;

use anyhow::{Context, Result, bail};
use std::env;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use scale::Crop;

/// The height that the detection runs at. SCXvid is slow at full resolution.
const TARGET_HEIGHT: i32 = 360;

fn main() -> Result<()> {
    let arguments: Vec<std::ffi::OsString> = env::args_os().skip(1).collect();
    if arguments.is_empty() {
        bail!("usage: genkfr [--crop L,T,W,H] <input-video> [input-video ...]");
    }

    let (crop, inputs) = parse_arguments(&arguments)?;
    let threads = xvid::probe_threads()?;
    let (major, minor, patch) = xvid::actual_version()?;
    println!("xvidcore {major}.{minor}.{patch}");

    for input in inputs {
        process_input(&input, crop, threads)
            .with_context(|| format!("failed to process {}", input.display()))?;
    }

    Ok(())
}

/// Split `--crop L,T,W,H` from the file names.
///
/// The four values are the VapourSynth `resize` crop arguments: `src_left`,
/// `src_top`, `src_width` and `src_height`. A value of `-` keeps the default.
fn parse_arguments(arguments: &[std::ffi::OsString]) -> Result<(Crop, Vec<PathBuf>)> {
    let mut crop = Crop::default();
    let mut inputs = Vec::new();
    let mut rest = arguments.iter();

    while let Some(argument) = rest.next() {
        if argument == "--crop" {
            let value = rest
                .next()
                .context("--crop needs four values: left,top,width,height")?;
            let value = value.to_string_lossy();
            let parts: Vec<&str> = value.split(',').map(str::trim).collect();
            if parts.len() != 4 {
                bail!("--crop needs four values, got {}", parts.len());
            }
            let number = |text: &str, name: &str| -> Result<Option<f64>> {
                if text == "-" {
                    return Ok(None);
                }
                let parsed: f64 = text
                    .parse()
                    .with_context(|| format!("--crop {name} is not a number: {text}"))?;
                if !parsed.is_finite() || parsed < 0.0 {
                    bail!("--crop {name} must be zero or more, got {text}");
                }
                Ok(Some(parsed))
            };
            crop = Crop {
                left: number(parts[0], "left")?,
                top: number(parts[1], "top")?,
                width: number(parts[2], "width")?,
                height: number(parts[3], "height")?,
            };
        } else if argument.to_string_lossy().starts_with("--") {
            bail!("unknown option {}", argument.to_string_lossy());
        } else {
            inputs.push(PathBuf::from(argument));
        }
    }

    if inputs.is_empty() {
        bail!("usage: genkfr [--crop L,T,W,H] <input-video> [input-video ...]");
    }
    Ok((crop, inputs))
}

fn process_input(input: &Path, crop: Crop, threads: i32) -> Result<()> {
    let output = default_output_path(input);
    let resolved = canonicalize_lossy(input)?;

    let mut decoder = decode::Decoder::open(&resolved, threads)?;
    let target_width = align_width_4(decoder.width, decoder.height, TARGET_HEIGHT);

    // The crop is in source pixels. As in VapourSynth, it selects the region
    // that the filter samples; it does not change the output size.
    let source_w = decoder.width as u32;
    let source_h = decoder.height as u32;

    let target_w = target_width;
    let mut encoder = xvid::Xvid::new(target_w, TARGET_HEIGHT, threads)?;
    let mut keyframes: Vec<i32> = Vec::new();

    let mut luma = Vec::new();
    let mut chroma_u = Vec::new();
    let mut chroma_v = Vec::new();

    for number in 0..decoder.num_frames {
        if number % 1000 == 0 {
            eprintln!("processing frame {number}/{}", decoder.num_frames);
        }

        let frame = decoder.frame(number)?;
        scale::resize_plane(
            scale::Plane {
                data: frame.y,
                stride: frame.y_stride,
            },
            source_w,
            source_h,
            target_w as u32,
            TARGET_HEIGHT as u32,
            crop,
        )?
        .clone_into(&mut luma);

        // Chroma uses the same crop and scale, in half resolution.
        let half_crop = Crop {
            left: crop.left.map(|v| v / 2.0),
            top: crop.top.map(|v| v / 2.0),
            width: crop.width.map(|v| v / 2.0),
            height: crop.height.map(|v| v / 2.0),
        };
        scale::resize_plane(
            scale::Plane {
                data: frame.u,
                stride: frame.uv_stride,
            },
            source_w.div_ceil(2),
            source_h.div_ceil(2),
            (target_w as u32).div_ceil(2),
            (TARGET_HEIGHT as u32).div_ceil(2),
            half_crop,
        )?
        .clone_into(&mut chroma_u);
        scale::resize_plane(
            scale::Plane {
                data: frame.v,
                stride: frame.uv_stride,
            },
            source_w.div_ceil(2),
            source_h.div_ceil(2),
            (target_w as u32).div_ceil(2),
            (TARGET_HEIGHT as u32).div_ceil(2),
            half_crop,
        )?
        .clone_into(&mut chroma_v);

        if encoder.is_keyframe(
            &luma,
            &chroma_u,
            &chroma_v,
            target_w,
            target_w / 2,
        )? {
            keyframes.push(number);
        }
    }

    write_kf_file(&output, decoder.fps(), &keyframes)?;

    println!("input: {}", resolved.display());
    println!("output: {}", output.display());
    println!("frames: {}", decoder.num_frames);
    println!("fps: {:.6}", decoder.fps());
    println!("keyframes: {}", keyframes.len());
    println!();

    Ok(())
}

fn default_output_path(input: &Path) -> PathBuf {
    PathBuf::from(format!("{}.kf.txt", input.display()))
}

fn write_kf_file(path: &Path, fps: f64, keyframes: &[i32]) -> Result<()> {
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    let mut file =
        fs::File::create(path).with_context(|| format!("failed to create {}", path.display()))?;
    writeln!(file, "# keyframe format v1")?;
    writeln!(file, "fps {:.6}", fps)?;
    for frame in keyframes {
        writeln!(file, "{frame}")?;
    }
    Ok(())
}

/// The width for a 360-pixel-high frame, rounded to a multiple of 4.
fn align_width_4(src_w: i32, src_h: i32, target_h: i32) -> i32 {
    let scaled = (src_w as f64 * target_h as f64) / src_h as f64;
    let rounded = (scaled / 4.0).round() as i32 * 4;
    rounded.max(16)
}

fn canonicalize_lossy(path: &Path) -> Result<PathBuf> {
    fs::canonicalize(path).with_context(|| format!("failed to resolve {}", path.display()))
}
