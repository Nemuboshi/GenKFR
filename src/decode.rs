//! Video decode through FFMS2, with no VapourSynth.
//!
//! Only the functions that GenKFR needs are declared. Their C layouts are
//! checked against the vendored header by `src/ffms-layout.cpp`.
//! No bindgen or libclang is required.
//!
//! The `ffms2.Source` defaults are reproduced: the first indexed video track,
//! one decode thread per logical CPU. Normal seeking is used; see `seek_mode`.

use anyhow::{Context, Result, bail};
use std::ffi::{CString, c_char, c_int};
use std::path::Path;
use std::ptr::NonNull;

// FFMS_TYPE_UNKNOWN is -1, so video is 0. See ffms.h:156.
const FFMS_TYPE_VIDEO: c_int = 0;
const FFMS_SEEK_NORMAL: c_int = 1;

/// GenKFR reads every frame once, in order, so linear access is enough.
///
/// It cannot be used. `FFMS_VideoSource` starts with `CurrentFrame` at 1
/// (`videosource.h:120`), and only a seek sets it to 0. In linear mode no seek
/// happens, so the read of frame 0 fails with "Non-linear access attempted".
fn seek_mode() -> c_int {
    FFMS_SEEK_NORMAL
}
const FFMS_IEH_IGNORE: c_int = 3;

/// FFMS2 truncates error text to fit this buffer.
const ERROR_BUFFER_SIZE: usize = 1024;

#[repr(C)]
struct FfmsErrorInfo {
    error_type: c_int,
    sub_type: c_int,
    buffer_size: c_int,
    buffer: *mut c_char,
}

/// The leading fields of `FFMS_Frame`. FFMS2 writes the whole structure, but
/// only these are read. The fields after `repeat_pict` are left out.
#[repr(C)]
struct FfmsFrame {
    data: [*const u8; 4],
    linesize: [c_int; 4],
    encoded_width: c_int,
    encoded_height: c_int,
    encoded_pixel_format: c_int,
    scaled_width: c_int,
    scaled_height: c_int,
    converted_pixel_format: c_int,
    key_frame: c_int,
    repeat_pict: c_int,
}

/// The leading fields of `FFMS_VideoProperties`.
#[repr(C)]
struct FfmsVideoProperties {
    fps_denominator: c_int,
    fps_numerator: c_int,
    rff_denominator: c_int,
    rff_numerator: c_int,
    num_frames: c_int,
    sar_num: c_int,
    sar_den: c_int,
    crop_top: c_int,
    crop_bottom: c_int,
    crop_left: c_int,
    crop_right: c_int,
    top_field_first: c_int,
}

#[repr(C)]
struct FfmsVideoSource {
    _private: [u8; 0],
}
#[repr(C)]
struct FfmsIndex {
    _private: [u8; 0],
}
#[repr(C)]
struct FfmsIndexer {
    _private: [u8; 0],
}

unsafe extern "C" {
    fn FFMS_Init(argument_count: c_int, argument_values: c_int);
    fn FFMS_GetPixFmt(name: *const c_char) -> c_int;
    fn FFMS_CreateIndexer(source: *const c_char, error: *mut FfmsErrorInfo) -> *mut FfmsIndexer;
    fn FFMS_DoIndexing2(
        indexer: *mut FfmsIndexer,
        error_handling: c_int,
        error: *mut FfmsErrorInfo,
    ) -> *mut FfmsIndex;
    fn FFMS_DestroyIndex(index: *mut FfmsIndex);
    fn FFMS_GetFirstTrackOfType(
        index: *mut FfmsIndex,
        track_type: c_int,
        error: *mut FfmsErrorInfo,
    ) -> c_int;
    fn FFMS_CreateVideoSource(
        source: *const c_char,
        track: c_int,
        index: *mut FfmsIndex,
        threads: c_int,
        seek_mode: c_int,
        error: *mut FfmsErrorInfo,
    ) -> *mut FfmsVideoSource;
    fn FFMS_DestroyVideoSource(source: *mut FfmsVideoSource);
    fn FFMS_GetVideoProperties(source: *mut FfmsVideoSource) -> *const FfmsVideoProperties;
    fn FFMS_GetFrame(
        source: *mut FfmsVideoSource,
        frame: c_int,
        error: *mut FfmsErrorInfo,
    ) -> *const FfmsFrame;
    fn FFMS_SetOutputFormatV2(
        source: *mut FfmsVideoSource,
        formats: *const c_int,
        width: c_int,
        height: c_int,
        resizer: c_int,
        error: *mut FfmsErrorInfo,
    ) -> c_int;
}

// Mirrored by src/ffms-layout.cpp, compiled against the vendored header.
const _: () = {
    use std::mem::{align_of, offset_of, size_of};
    assert!(
        size_of::<usize>() == 8,
        "FFI layouts require a 64-bit target"
    );
    assert!(size_of::<FfmsErrorInfo>() == 24 && align_of::<FfmsErrorInfo>() == 8);
    assert!(offset_of!(FfmsErrorInfo, buffer) == 16);
    assert!(size_of::<FfmsFrame>() == 80 && align_of::<FfmsFrame>() == 8);
    assert!(offset_of!(FfmsFrame, encoded_width) == 48);
    assert!(offset_of!(FfmsFrame, repeat_pict) == 76);
    assert!(size_of::<FfmsVideoProperties>() == 48 && align_of::<FfmsVideoProperties>() == 4);
    assert!(offset_of!(FfmsVideoProperties, top_field_first) == 44);
};

/// Collect error text from our own buffer, without trusting a returned pointer.
fn error_text(buffer: &[c_char]) -> String {
    let bytes: Vec<u8> = buffer
        .iter()
        .take_while(|&&byte| byte != 0)
        .map(|&byte| byte as u8)
        .collect();
    String::from_utf8_lossy(&bytes).into_owned()
}

/// A decoded frame. The planes stay valid until the next call to
/// [`Decoder::frame`].
pub struct Frame<'a> {
    pub y: &'a [u8],
    pub u: &'a [u8],
    pub v: &'a [u8],
    pub y_stride: usize,
    pub uv_stride: usize,
}

struct VideoSource(NonNull<FfmsVideoSource>);
struct Index(NonNull<FfmsIndex>);

impl Drop for VideoSource {
    fn drop(&mut self) {
        // SAFETY: this wrapper uniquely owns a successfully created source.
        unsafe { FFMS_DestroyVideoSource(self.0.as_ptr()) };
    }
}

impl Drop for Index {
    fn drop(&mut self) {
        // SAFETY: all sources borrowing this index have been dropped first.
        unsafe { FFMS_DestroyIndex(self.0.as_ptr()) };
    }
}

pub struct Decoder {
    // Fields drop in declaration order: the source borrows its index.
    source: VideoSource,
    _index: Index,
    planes: [Vec<u8>; 3],
    output_format: c_int,
    width: i32,
    height: i32,
    num_frames: i32,
    fps_num: i64,
    fps_den: i64,
}

impl Decoder {
    /// Open a video and index it when no index is present.
    ///
    /// `threads` follows the `ffms2.Source` default of one thread per logical
    /// CPU.
    pub fn open(path: &Path, threads: i32) -> Result<Self> {
        let source =
            CString::new(path.to_string_lossy().as_bytes()).context("path contains a NUL byte")?;

        static INIT: std::sync::Once = std::sync::Once::new();
        INIT.call_once(|| {
            // SAFETY: FFMS_Init requires zero arguments and runs once per process.
            unsafe { FFMS_Init(0, 0) };
        });

        let mut buffer = vec![0 as c_char; ERROR_BUFFER_SIZE];
        let mut error = FfmsErrorInfo {
            error_type: 0,
            sub_type: 0,
            buffer_size: ERROR_BUFFER_SIZE as c_int,
            buffer: buffer.as_mut_ptr(),
        };

        // SAFETY: source is NUL-terminated; error points to our writable buffer.
        let indexer = unsafe { FFMS_CreateIndexer(source.as_ptr(), &raw mut error) };
        if indexer.is_null() {
            bail!("cannot index {}: {}", path.display(), error_text(&buffer));
        }

        // FFMS_DoIndexing2 consumes the indexer, including on failure.
        // SAFETY: consumes the live indexer; error and its buffer remain valid.
        let index = unsafe { FFMS_DoIndexing2(indexer, FFMS_IEH_IGNORE, &raw mut error) };
        let index = Index(NonNull::new(index).with_context(|| {
            format!(
                "indexing failed for {}: {}",
                path.display(),
                error_text(&buffer)
            )
        })?);

        // ffms2.Source uses track -1 for "the first video track".
        // SAFETY: index is live; the result is checked before creating sources.
        let track =
            unsafe { FFMS_GetFirstTrackOfType(index.0.as_ptr(), FFMS_TYPE_VIDEO, &raw mut error) };
        if track < 0 {
            bail!(
                "cannot find a video track in {}: {}",
                path.display(),
                error_text(&buffer)
            );
        }
        // Probe the source dimensions before configuring yuv420p output.
        // SAFETY: valid path/track/index and error buffer; index outlives probe.
        let probe = unsafe {
            FFMS_CreateVideoSource(
                source.as_ptr(),
                track,
                index.0.as_ptr(),
                threads,
                seek_mode(),
                &raw mut error,
            )
        };
        let probe = VideoSource(NonNull::new(probe).with_context(|| {
            format!(
                "cannot open video in {}: {}",
                path.display(),
                error_text(&buffer)
            )
        })?);

        // SAFETY: probe owns a live source, with no frame reference borrowed.
        let properties = unsafe { FFMS_GetVideoProperties(probe.0.as_ptr()) };
        if properties.is_null() {
            bail!("FFMS2 returned no video properties for {}", path.display());
        }
        // SAFETY: source-owned properties are aligned and valid until drop.
        let properties = unsafe { &*properties };
        let num_frames = properties.num_frames;
        let fps_num = properties.fps_numerator;
        let fps_den = properties.fps_denominator;
        if num_frames <= 0 {
            bail!("{} reports {num_frames} frames", path.display());
        }
        if fps_num <= 0 || fps_den <= 0 {
            bail!(
                "{} reports the frame rate {fps_num}/{fps_den}",
                path.display()
            );
        }

        // SAFETY: probe is live and num_frames > 0; error buffer is writable.
        let first = unsafe { FFMS_GetFrame(probe.0.as_ptr(), 0, &raw mut error) };
        if first.is_null() {
            bail!(
                "cannot read frame 0 of {}: {}",
                path.display(),
                error_text(&buffer)
            );
        }
        // SAFETY: the non-null frame prefix is source-owned and valid here.
        let first = unsafe { &*first };
        let width = first.encoded_width;
        let height = first.encoded_height;
        drop(probe);

        if width <= 0 || height <= 0 {
            bail!("invalid frame size {width}x{height} in {}", path.display());
        }

        // SAFETY: checked path/track/index; index will outlive the video source.
        let video = unsafe {
            FFMS_CreateVideoSource(
                source.as_ptr(),
                track,
                index.0.as_ptr(),
                threads,
                seek_mode(),
                &raw mut error,
            )
        };
        let video = VideoSource(NonNull::new(video).with_context(|| {
            format!(
                "cannot reopen video in {}: {}",
                path.display(),
                error_text(&buffer)
            )
        })?);

        // Ask FFMS2 for yuv420p at the source size. The source may be 10-bit or
        // another layout, and the encoder needs 8-bit 4:2:0. The size is left
        // alone so that the resize stays in this program.
        // SAFETY: the static pixel format name is NUL-terminated.
        let format = unsafe { FFMS_GetPixFmt(c"yuv420p".as_ptr()) };
        if format < 0 {
            bail!("this FFMS2 build has no yuv420p format");
        }
        let formats = [format, -1];
        // SAFETY: dimensions are positive, formats ends in -1; no frame is borrowed.
        let rc = unsafe {
            FFMS_SetOutputFormatV2(
                video.0.as_ptr(),
                formats.as_ptr(),
                width,
                height,
                0,
                &raw mut error,
            )
        };
        if rc != 0 {
            bail!(
                "cannot set the output format for {}: {}",
                path.display(),
                error_text(&buffer)
            );
        }

        Ok(Self {
            source: video,
            _index: index,
            planes: std::array::from_fn(|_| Vec::new()),
            output_format: format,
            width,
            height,
            num_frames,
            fps_num: i64::from(fps_num),
            fps_den: i64::from(fps_den),
        })
    }

    /// Read one frame. The returned planes are valid until the next call.
    pub fn frame(&mut self, number: i32) -> Result<Frame<'_>> {
        if number < 0 || number >= self.num_frames {
            bail!("frame {number} is outside 0..{}", self.num_frames);
        }

        let mut buffer = vec![0 as c_char; ERROR_BUFFER_SIZE];
        let mut error = FfmsErrorInfo {
            error_type: 0,
            sub_type: 0,
            buffer_size: ERROR_BUFFER_SIZE as c_int,
            buffer: buffer.as_mut_ptr(),
        };

        // SAFETY: source is live, number is in range, and &mut self excludes
        // other frame reads or destruction while the returned data is copied.
        let frame = unsafe { FFMS_GetFrame(self.source.0.as_ptr(), number, &raw mut error) };
        if frame.is_null() {
            bail!("cannot read frame {number}: {}", error_text(&buffer));
        }
        // SAFETY: FFMS_GetFrame returns an aligned, source-owned frame prefix.
        let frame = unsafe { &*frame };
        if frame.scaled_width != self.width
            || frame.scaled_height != self.height
            || frame.converted_pixel_format != self.output_format
        {
            bail!("FFMS2 returned an unexpected frame size or pixel format");
        }

        for (i, (w, h)) in [
            (self.width, self.height),
            (
                self.width / 2 + self.width % 2,
                self.height / 2 + self.height % 2,
            ),
            (
                self.width / 2 + self.width % 2,
                self.height / 2 + self.height % 2,
            ),
        ]
        .into_iter()
        .enumerate()
        {
            // SAFETY: OutputFrame supplies source-owned AVFrame or av_image_alloc
            // planes in yuv420p. No source call or destruction occurs during copy.
            unsafe { copy_plane(frame.data[i], frame.linesize[i], w, h, &mut self.planes[i])? };
        }

        Ok(Frame {
            y: &self.planes[0],
            u: &self.planes[1],
            v: &self.planes[2],
            y_stride: self.width as usize,
            uv_stride: (self.width as usize).div_ceil(2),
        })
    }

    pub fn width(&self) -> i32 {
        self.width
    }
    pub fn height(&self) -> i32 {
        self.height
    }
    pub fn num_frames(&self) -> i32 {
        self.num_frames
    }

    /// The frame rate as a floating point value.
    pub fn fps(&self) -> f64 {
        self.fps_num as f64 / self.fps_den as f64
    }
}

/// Copy only initialized pixels, not the possibly uninitialized row padding.
///
/// # Safety
/// If the layout and pointer checks succeed, `data` must cover each row's
/// `width` initialized bytes at the given stride in one allocation, and remain
/// valid and unmodified throughout this call.
unsafe fn copy_plane(
    data: *const u8,
    stride: i32,
    width: i32,
    height: i32,
    output: &mut Vec<u8>,
) -> Result<()> {
    let len = crate::plane::plane_len(stride, width, height)?;
    if data.is_null() || (data as usize).checked_add(len).is_none() {
        bail!("FFMS2 returned an invalid plane pointer");
    }
    output.resize(width as usize * height as usize, 0);
    for (row, destination) in output.chunks_exact_mut(width as usize).enumerate() {
        // SAFETY: checked layout bounds offsets; caller guarantees initialized
        // pixels and allocation validity. Padding is never made into a slice.
        let pixels =
            unsafe { std::slice::from_raw_parts(data.add(row * stride as usize), width as usize) };
        destination.copy_from_slice(pixels);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opening_audio_only_input_fails_cleanly() {
        let path =
            std::env::temp_dir().join(format!("genkfr-audio-only-{}.wav", std::process::id()));
        let mut wav = Vec::new();
        wav.extend_from_slice(b"RIFF");
        wav.extend_from_slice(&68u32.to_le_bytes());
        wav.extend_from_slice(b"WAVEfmt ");
        wav.extend_from_slice(&16u32.to_le_bytes());
        wav.extend_from_slice(&1u16.to_le_bytes()); // PCM
        wav.extend_from_slice(&1u16.to_le_bytes()); // mono
        wav.extend_from_slice(&8000u32.to_le_bytes());
        wav.extend_from_slice(&16000u32.to_le_bytes());
        wav.extend_from_slice(&2u16.to_le_bytes());
        wav.extend_from_slice(&16u16.to_le_bytes());
        wav.extend_from_slice(b"data");
        wav.extend_from_slice(&32u32.to_le_bytes());
        wav.extend_from_slice(&[0; 32]);
        std::fs::write(&path, wav).unwrap();
        for _ in 0..2 {
            let result = Decoder::open(&path, 1);
            assert!(result.is_err());
            assert!(result.err().unwrap().to_string().contains("video track"));
        }
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn opening_invalid_input_fails_cleanly() {
        let path =
            std::env::temp_dir().join(format!("genkfr-invalid-video-{}.bin", std::process::id()));
        std::fs::write(&path, b"not a video").unwrap();
        for _ in 0..2 {
            assert!(Decoder::open(&path, 1).is_err());
        }
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn error_messages_are_bounded_by_the_buffer() {
        assert_eq!(error_text(&[65, 66, 0, 67]), "AB");
        assert_eq!(error_text(&[65, 66]), "AB");
        assert_eq!(error_text(&[]), "");
    }

    #[test]
    fn copies_pixels_without_row_padding() {
        let source = [1, 2, 3, 99, 99, 4, 5, 6];
        let mut output = Vec::new();
        // SAFETY: both three-byte rows fit in source; final padding is absent.
        unsafe { copy_plane(source.as_ptr(), 5, 3, 2, &mut output).unwrap() };
        assert_eq!(output, [1, 2, 3, 4, 5, 6]);
        // SAFETY: the first row fits in source, and output can be reused.
        unsafe { copy_plane(source.as_ptr(), 3, 3, 1, &mut output).unwrap() };
        assert_eq!(output, [1, 2, 3]);
    }

    #[test]
    fn rejects_invalid_planes_before_dereferencing() {
        let mut output = Vec::new();
        for (stride, width, height) in [
            (3, 3, 2),
            (0, 3, 2),
            (-3, 3, 2),
            (2, 3, 2),
            (3, 3, 0),
            (3, -1, 2),
        ] {
            // SAFETY: invalid layouts/null pointers are rejected before access.
            assert!(
                unsafe { copy_plane(std::ptr::null(), stride, width, height, &mut output) }
                    .is_err()
            );
        }
        // SAFETY: address overflow is rejected before accessing this pointer.
        assert!(
            unsafe {
                copy_plane(
                    std::ptr::without_provenance(usize::MAX),
                    3,
                    3,
                    2,
                    &mut output,
                )
            }
            .is_err()
        );
    }
}
