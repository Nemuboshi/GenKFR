//! Bilinear resize that matches VapourSynth's `resize.Bilinear`.
//!
//! VapourSynth's resize filter is the zimg library. A Rust bilinear kernel is
//! not automatically the same as zimg's, because zimg derives its filter
//! weights from a scale-dependent support and normalizes them per output
//! sample. This module reproduces that arithmetic, so the pixels match.
//!
//! Crop is part of the same operation, as in VapourSynth. The arguments are
//! `src_left`, `src_top`, `src_width` and `src_height`, and they select the
//! region of the input that the filter reads.

use anyhow::{Result, bail};

/// One axis of the filter: the source index and weight of each tap.
struct Axis {
    /// `start[i]` is the first source index for output sample `i`.
    start: Vec<u32>,
    /// `offset[i]` is where the taps of output sample `i` begin in `weights`.
    offset: Vec<u32>,
    weights: Vec<f32>,
    count: Vec<u32>,
}

/// The active region of the source, in pixels. `None` means the full frame.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Crop {
    pub left: Option<f64>,
    pub top: Option<f64>,
    pub width: Option<f64>,
    pub height: Option<f64>,
}

/// Round a position on the pixel grid. Copied from zimg's `round_halfup`
/// (`filter.cpp`), which deliberately avoids half-to-even so that
/// `round(x - 1) == round(x) - 1` holds.
fn round_halfup(x: f64) -> f64 {
    if x < 0.0 {
        (x + 0.5).floor()
    } else {
        (x + 0.499_999_999_999_999_94).floor()
    }
}

/// zimg's `BilinearFilter`: a triangle of support 1.
fn bilinear(x: f64) -> f64 {
    (1.0 - x.abs()).max(0.0)
}

/// Build the filter for one axis. This is zimg's `compute_filter`
/// (`filter.cpp:246`).
///
/// `src_dim` is the size of the active source region, `dst_dim` the output
/// size, and `shift` the crop offset expressed in source pixels.
fn build_axis(src_dim: u32, dst_dim: u32, shift: f64) -> Result<Axis> {
    let scale = f64::from(dst_dim) / f64::from(src_dim);
    let step = scale.min(1.0);
    let support = 1.0 / step; // BilinearFilter::support() == 1
    let filter_size = ((support.ceil() as u32) * 2).max(1) as usize;

    let mut start = Vec::with_capacity(dst_dim as usize);
    let mut count = Vec::with_capacity(dst_dim as usize);
    let mut offset = Vec::with_capacity(dst_dim as usize);
    let mut weights = Vec::with_capacity(dst_dim as usize * filter_size);

    for i in 0..dst_dim {
        // Position of the output sample on the input grid.
        let pos = (f64::from(i) + 0.5) / scale + shift;
        let begin_pos = round_halfup(pos - filter_size as f64 / 2.0) + 0.5;

        let mut total = 0.0;
        for j in 0..filter_size {
            total += bilinear((begin_pos + j as f64 - pos) * step);
        }
        if total == 0.0 {
            bail!("resize filter has zero weight at output sample {i}");
        }

        // Collect the taps, mirroring out-of-bounds positions as zimg does.
        let mut taps: Vec<(u32, f64)> = Vec::with_capacity(filter_size);
        let mut first = u32::MAX;
        for j in 0..filter_size {
            let xpos = begin_pos + j as f64;
            let real_pos = if xpos < 0.0 {
                -xpos
            } else if xpos >= f64::from(src_dim) {
                2.0 * f64::from(src_dim) - xpos
            } else {
                xpos
            };
            // Clamp to the last representable position inside the image.
            let limit = f64::from(src_dim) - f64::EPSILON / 2.0;
            let real_pos = real_pos.clamp(0.0, limit);
            let idx = real_pos.floor() as u32;
            first = first.min(idx);
            let w = bilinear((xpos - pos) * step) / total;
            taps.push((idx, w));
        }

        // Merge taps that mirror onto the same index, as zimg's matrix does.
        taps.sort_by_key(|t| t.0);
        let mut merged: Vec<(u32, f32)> = Vec::with_capacity(taps.len());
        for (idx, w) in taps {
            match merged.last_mut() {
                Some(last) if last.0 == idx => last.1 += w as f32,
                _ => merged.push((idx, w as f32)),
            }
        }

        start.push(first);
        count.push((merged.last().unwrap().0 - first + 1) as u32);
        offset.push(weights.len() as u32);
        for t in &merged {
            weights.push(t.1);
        }
    }

    Ok(Axis {
        start,
        offset,
        weights,
        count,
    })
}

/// Apply one axis of the filter to a plane of `src_w` by `src_h` samples.
///
/// `horizontal` selects the axis. `stride` is the row length of `src` in
/// bytes; the output is tightly packed.
#[allow(clippy::too_many_arguments)]
fn apply_axis(
    src: &[u8],
    src_w: u32,
    src_h: u32,
    stride: usize,
    axis: &Axis,
    dst_len: u32,
    horizontal: bool,
) -> Vec<u8> {
    let out_len = if horizontal {
        src_h as usize
    } else {
        src_w as usize
    } * dst_len as usize;
    let mut out = vec![0u8; out_len];

    if horizontal {
        for y in 0..src_h as usize {
            let row = &src[y * stride..];
            for i in 0..dst_len as usize {
                let s = axis.start[i] as usize;
                let n = axis.count[i] as usize;
                let mut acc = 0.0f32;
                for k in 0..n {
                    let w = axis.weights[axis.offset[i] as usize + k];
                    acc += w * f32::from(row[s + k]);
                }
                // zimg rounds the accumulator half away from zero.
                out[y * dst_len as usize + i] = (acc + 0.5).floor().clamp(0.0, 255.0) as u8;
            }
        }
    } else {
        for i in 0..dst_len as usize {
            let s = axis.start[i] as usize;
            let n = axis.count[i] as usize;
            for x in 0..src_w as usize {
                let mut acc = 0.0f32;
                for k in 0..n {
                    let w = axis.weights[axis.offset[i] as usize + k];
                    acc += w * f32::from(src[(s + k) * stride + x]);
                }
                out[i * src_w as usize + x] = (acc + 0.5).floor().clamp(0.0, 255.0) as u8;
            }
        }
    }

    out
}

/// A plane of 8-bit samples.
pub struct Plane<'a> {
    pub data: &'a [u8],
    pub stride: usize,
}

/// Resize and crop one plane.
///
/// `dst_w` and `dst_h` are the output size. `crop` selects the source region.
/// The returned buffer is tightly packed at `dst_w` bytes per row.
pub fn resize_plane(
    plane: Plane<'_>,
    src_w: u32,
    src_h: u32,
    dst_w: u32,
    dst_h: u32,
    crop: Crop,
) -> Result<Vec<u8>> {
    if dst_w == 0 || dst_h == 0 {
        bail!("resize needs a non-zero output size");
    }

    // Default the crop to the full frame, as VapourSynth does.
    let left = crop.left.unwrap_or(0.0);
    let top = crop.top.unwrap_or(0.0);
    let width = crop.width.unwrap_or(f64::from(src_w) - left);
    let height = crop.height.unwrap_or(f64::from(src_h) - top);

    if width <= 0.0 || height <= 0.0 {
        bail!("crop leaves an empty region: {width} x {height}");
    }
    if left < 0.0 || top < 0.0 || left + width > f64::from(src_w) || top + height > f64::from(src_h)
    {
        bail!(
            "crop {left},{top} {width}x{height} is outside the {src_w}x{src_h} frame"
        );
    }

    let hx = build_axis(width.round() as u32, dst_w, left)?;
    let vy = build_axis(height.round() as u32, dst_h, top)?;

    // The filter indexes the active region, so offset the source first.
    let left_i = left.floor() as usize;
    let top_i = top.floor() as usize;
    let active_w = width.round() as u32;
    let active_h = height.round() as u32;

    let mut region = vec![0u8; active_w as usize * active_h as usize];
    for y in 0..active_h as usize {
        let src_row = (top_i + y) * plane.stride + left_i;
        region[y * active_w as usize..(y + 1) * active_w as usize]
            .copy_from_slice(&plane.data[src_row..src_row + active_w as usize]);
    }

    let horiz = apply_axis(&region, active_w, active_h, active_w as usize, &hx, dst_w, true);
    Ok(apply_axis(&horiz, dst_w, active_h, dst_w as usize, &vy, dst_h, false))
}
