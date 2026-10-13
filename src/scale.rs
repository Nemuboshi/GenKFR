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
        count.push(merged.last().unwrap().0 - first + 1);
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

/// A plane of 8-bit samples.
pub struct Plane<'a> {
    pub data: &'a [u8],
    pub stride: usize,
}

/// Fixed filter coefficients and reusable buffers for one video plane.
pub struct Resizer {
    src_w: usize,
    src_h: usize,
    left: usize,
    top: usize,
    active_w: usize,
    dst_w: usize,
    hx: Axis,
    vy: Axis,
    horizontal: Vec<u8>,
    output: Vec<u8>,
    row: Vec<f32>,
}

impl Resizer {
    pub fn new(src_w: u32, src_h: u32, dst_w: u32, dst_h: u32, crop: Crop) -> Result<Self> {
        if src_w == 0 || src_h == 0 || dst_w == 0 || dst_h == 0 {
            bail!("resize needs non-zero input and output sizes");
        }
        let left = crop.left.unwrap_or(0.0);
        let top = crop.top.unwrap_or(0.0);
        let width = crop.width.unwrap_or(f64::from(src_w) - left);
        let height = crop.height.unwrap_or(f64::from(src_h) - top);
        if ![left, top, width, height].iter().all(|v| v.is_finite()) {
            bail!("crop values must be finite");
        }
        if width.round() <= 0.0 || height.round() <= 0.0 {
            bail!("crop leaves an empty region: {width} x {height}");
        }
        if left < 0.0
            || top < 0.0
            || left + width > f64::from(src_w)
            || top + height > f64::from(src_h)
        {
            bail!("crop {left},{top} {width}x{height} is outside the {src_w}x{src_h} frame");
        }
        let active_w = width.round() as u32;
        let active_h = height.round() as u32;
        let horizontal_len = (active_h as usize)
            .checked_mul(dst_w as usize)
            .filter(|&len| len <= isize::MAX as usize)
            .ok_or_else(|| anyhow::anyhow!("resize buffer exceeds the addressable range"))?;
        let output_len = (dst_h as usize)
            .checked_mul(dst_w as usize)
            .filter(|&len| len <= isize::MAX as usize)
            .ok_or_else(|| anyhow::anyhow!("resize buffer exceeds the addressable range"))?;
        Ok(Self {
            src_w: src_w as usize,
            src_h: src_h as usize,
            left: left.floor() as usize,
            top: top.floor() as usize,
            active_w: active_w as usize,
            dst_w: dst_w as usize,
            hx: build_axis(active_w, dst_w, left)?,
            vy: build_axis(active_h, dst_h, top)?,
            horizontal: vec![0; horizontal_len],
            output: vec![0; output_len],
            row: vec![0.0; dst_w as usize],
        })
    }

    /// The tightly packed result stays valid until the next resize call.
    pub fn resize(&mut self, plane: Plane<'_>) -> Result<&[u8]> {
        let needed = plane
            .stride
            .checked_mul(self.src_h - 1)
            .and_then(|len| len.checked_add(self.src_w));
        if plane.stride < self.src_w || needed.is_none_or(|len| len > plane.data.len()) {
            bail!("resize input is too small for its size and stride");
        }
        for (y, destination) in self.horizontal.chunks_exact_mut(self.dst_w).enumerate() {
            let offset = (self.top + y) * plane.stride + self.left;
            let source = &plane.data[offset..offset + self.active_w];
            for (i, pixel) in destination.iter_mut().enumerate() {
                let start = self.hx.start[i] as usize;
                let count = self.hx.count[i] as usize;
                let offset = self.hx.offset[i] as usize;
                let weights = &self.hx.weights[offset..offset + count];
                let pixels = &source[start..start + count];
                let mut acc = 0.0f32;
                for (&weight, &value) in weights.iter().zip(pixels) {
                    acc += weight * f32::from(value);
                }
                *pixel = (acc + 0.5).floor().clamp(0.0, 255.0) as u8;
            }
        }
        for (i, destination) in self.output.chunks_exact_mut(self.dst_w).enumerate() {
            self.row.fill(0.0);
            let start = self.vy.start[i] as usize * self.dst_w;
            let count = self.vy.count[i] as usize;
            let offset = self.vy.offset[i] as usize;
            let weights = &self.vy.weights[offset..offset + count];
            let rows = &self.horizontal[start..start + count * self.dst_w];
            // Keep each pixel's tap order while processing contiguous rows for vectorization.
            for (&weight, source) in weights.iter().zip(rows.chunks_exact(self.dst_w)) {
                for (acc, &value) in self.row.iter_mut().zip(source) {
                    *acc += weight * f32::from(value);
                }
            }
            for (pixel, &acc) in destination.iter_mut().zip(&self.row) {
                *pixel = (acc + 0.5).floor().clamp(0.0, 255.0) as u8;
            }
        }
        Ok(&self.output)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Scalar indexing and accumulation from the previous implementation.
    fn reference(resizer: &Resizer, data: &[u8], stride: usize) -> Vec<u8> {
        let mut horizontal = vec![0; resizer.horizontal.len()];
        for y in 0..horizontal.len() / resizer.dst_w {
            for x in 0..resizer.dst_w {
                let mut acc = 0.0f32;
                for k in 0..resizer.hx.count[x] as usize {
                    let weight = resizer.hx.weights[resizer.hx.offset[x] as usize + k];
                    let index = (resizer.top + y) * stride
                        + resizer.left
                        + resizer.hx.start[x] as usize
                        + k;
                    acc += weight * f32::from(data[index]);
                }
                horizontal[y * resizer.dst_w + x] = (acc + 0.5).floor().clamp(0.0, 255.0) as u8;
            }
        }
        let mut output = vec![0; resizer.output.len()];
        for y in 0..resizer.vy.start.len() {
            for x in 0..resizer.dst_w {
                let mut acc = 0.0f32;
                for k in 0..resizer.vy.count[y] as usize {
                    let weight = resizer.vy.weights[resizer.vy.offset[y] as usize + k];
                    let index = (resizer.vy.start[y] as usize + k) * resizer.dst_w + x;
                    acc += weight * f32::from(horizontal[index]);
                }
                output[y * resizer.dst_w + x] = (acc + 0.5).floor().clamp(0.0, 255.0) as u8;
            }
        }
        output
    }

    #[test]
    fn keeps_rounding_between_the_two_filter_passes() {
        let mut resizer = Resizer::new(2, 2, 1, 1, Crop::default()).unwrap();
        assert_eq!(
            resizer
                .resize(Plane {
                    data: &[0, 40, 80, 120],
                    stride: 2
                })
                .unwrap(),
            [60]
        );
        // Horizontal rows round to 1 and 2, then the vertical result rounds to 2.
        assert_eq!(
            resizer
                .resize(Plane {
                    data: &[0, 1, 1, 2],
                    stride: 2
                })
                .unwrap(),
            [2]
        );
    }

    #[test]
    fn preserves_scalar_pixels_and_reuses_buffers() {
        let mut state = 42u32;
        for (sw, sh, dw, dh, crop) in [
            (1, 1, 3, 5, Crop::default()),
            (5, 7, 2, 3, Crop::default()),
            (6, 4, 11, 9, Crop::default()),
            (65, 49, 32, 24, Crop::default()),
            (
                17,
                19,
                6,
                4,
                Crop {
                    left: Some(2.25),
                    top: Some(1.25),
                    width: Some(11.5),
                    height: Some(12.5),
                },
            ),
        ] {
            let mut resizer = Resizer::new(sw, sh, dw, dh, crop).unwrap();
            let pointers = (
                resizer.horizontal.as_ptr(),
                resizer.output.as_ptr(),
                resizer.row.as_ptr(),
            );
            for padding in [0, 7] {
                let stride = sw as usize + padding;
                for _ in 0..3 {
                    let data: Vec<_> = (0..stride * (sh as usize - 1) + sw as usize)
                        .map(|_| {
                            state = state.wrapping_mul(1664525).wrapping_add(1013904223);
                            (state >> 24) as u8
                        })
                        .collect();
                    let expected = reference(&resizer, &data, stride);
                    assert_eq!(
                        resizer
                            .resize(Plane {
                                data: &data,
                                stride
                            })
                            .unwrap(),
                        expected
                    );
                    assert_eq!(
                        pointers,
                        (
                            resizer.horizontal.as_ptr(),
                            resizer.output.as_ptr(),
                            resizer.row.as_ptr()
                        )
                    );
                }
            }
        }
    }

    #[test]
    fn rejects_invalid_dimensions_crops_and_planes() {
        for size in [(0, 4, 2, 2), (4, 0, 2, 2), (4, 4, 0, 2), (4, 4, 2, 0)] {
            assert!(Resizer::new(size.0, size.1, size.2, size.3, Crop::default()).is_err());
        }
        for left in [-1.0, f64::NAN, f64::INFINITY, 4.0] {
            assert!(
                Resizer::new(
                    4,
                    4,
                    2,
                    2,
                    Crop {
                        left: Some(left),
                        ..Crop::default()
                    }
                )
                .is_err()
            );
        }
        assert!(
            Resizer::new(
                4,
                4,
                2,
                2,
                Crop {
                    width: Some(0.4),
                    ..Crop::default()
                }
            )
            .is_err()
        );
        let mut resizer = Resizer::new(4, 4, 2, 2, Crop::default()).unwrap();
        for (data, stride) in [(&[0; 16][..], 3), (&[0; 15][..], 4), (&[][..], usize::MAX)] {
            assert!(resizer.resize(Plane { data, stride }).is_err());
        }
    }
}
