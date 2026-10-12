//! Video decode through FFMS2, with no VapourSynth.
//!
//! Only the functions that GenKFR needs are declared. The official Windows
//! build of FFMS2 exports these with the C names, so no bindgen, no libclang
//! and no FFmpeg development files are necessary.
//!
//! The `ffms2.Source` defaults are reproduced: the first indexed video track,
//! one decode thread per logical CPU, and linear access.

use anyhow::{Context, Result, bail};
use std::ffi::{CStr, CString, c_char, c_int};
use std::path::Path;

// FFMS_TYPE_UNKNOWN is -1, so video is 0. See ffms.h:156.
const FFMS_TYPE_VIDEO: c_int = 0;
/// Sequential access without rewind. GenKFR reads every frame once, in order.
const FFMS_SEEK_LINEAR_NO_RW: c_int = -1;
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

/// Enough for any FFMS2 message.
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

#[link(name = "ffms2", kind = "static")]
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

/// Collect the FFMS2 error text.
fn error_text(error: &FfmsErrorInfo) -> String {
    if error.buffer.is_null() || error.buffer_size <= 0 {
        return String::from("no message");
    }
    unsafe { CStr::from_ptr(error.buffer) }
        .to_string_lossy()
        .into_owned()
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

pub struct Decoder {
    source: *mut FfmsVideoSource,
    index: *mut FfmsIndex,
    pub width: i32,
    #[allow(dead_code)]
    pub height: i32,
    pub num_frames: i32,
    pub fps_num: i64,
    pub fps_den: i64,
}

impl Decoder {
    /// Open a video and index it when no index is present.
    ///
    /// `threads` follows the `ffms2.Source` default of one thread per logical
    /// CPU.
    pub fn open(path: &Path, threads: i32) -> Result<Self> {
        let source =
            CString::new(path.to_string_lossy().as_bytes()).context("path contains a NUL byte")?;

        unsafe { FFMS_Init(0, 0) };

        let mut buffer = vec![0 as c_char; ERROR_BUFFER_SIZE];
        let mut error = FfmsErrorInfo {
            error_type: 0,
            sub_type: 0,
            buffer_size: ERROR_BUFFER_SIZE as c_int,
            buffer: buffer.as_mut_ptr(),
        };

        let indexer = unsafe { FFMS_CreateIndexer(source.as_ptr(), &raw mut error) };
        if indexer.is_null() {
            bail!("cannot index {}: {}", path.display(), error_text(&error));
        }

        // The indexer is destroyed by FFMS_DoIndexing2, so query it first.
        let index = unsafe { FFMS_DoIndexing2(indexer, FFMS_IEH_IGNORE, &raw mut error) };
        if index.is_null() {
            bail!(
                "indexing failed for {}: {}",
                path.display(),
                error_text(&error)
            );
        }

        // ffms2.Source uses track -1 for "the first video track".
        let track = unsafe { FFMS_GetFirstTrackOfType(index, FFMS_TYPE_VIDEO, &raw mut error) };
        // The output format must be set before frames are used, but the frame
        // size is only known from a frame. Probe with one video source, then
        // open a second one for the real pass. Reading frame 0 twice on a
        // single source would break linear access.
        let probe = unsafe {
            FFMS_CreateVideoSource(
                source.as_ptr(),
                track,
                index,
                threads,
                seek_mode(),
                &raw mut error,
            )
        };
        if probe.is_null() {
            unsafe { FFMS_DestroyIndex(index) };
            bail!(
                "cannot open video in {}: {}",
                path.display(),
                error_text(&error)
            );
        }

        let properties = unsafe { FFMS_GetVideoProperties(probe) };
        if properties.is_null() {
            unsafe {
                FFMS_DestroyVideoSource(probe);
                FFMS_DestroyIndex(index);
            }
            bail!("FFMS2 returned no video properties for {}", path.display());
        }
        let properties = unsafe { &*properties };
        let num_frames = properties.num_frames;
        let fps_num = properties.fps_numerator;
        let fps_den = properties.fps_denominator;

        let first = unsafe { FFMS_GetFrame(probe, 0, &raw mut error) };
        if first.is_null() {
            unsafe {
                FFMS_DestroyVideoSource(probe);
                FFMS_DestroyIndex(index);
            }
            bail!(
                "cannot read frame 0 of {}: {}",
                path.display(),
                error_text(&error)
            );
        }
        let first = unsafe { &*first };
        let width = first.encoded_width;
        let height = first.encoded_height;
        unsafe { FFMS_DestroyVideoSource(probe) };

        if width <= 0 || height <= 0 {
            unsafe { FFMS_DestroyIndex(index) };
            bail!("invalid frame size {width}x{height} in {}", path.display());
        }

        let video = unsafe {
            FFMS_CreateVideoSource(
                source.as_ptr(),
                track,
                index,
                threads,
                seek_mode(),
                &raw mut error,
            )
        };
        if video.is_null() {
            unsafe { FFMS_DestroyIndex(index) };
            bail!(
                "cannot reopen video in {}: {}",
                path.display(),
                error_text(&error)
            );
        }

        // Ask FFMS2 for yuv420p at the source size. The source may be 10-bit or
        // another layout, and the encoder needs 8-bit 4:2:0. The size is left
        // alone so that the resize stays in this program.
        let format = unsafe { FFMS_GetPixFmt(c"yuv420p".as_ptr()) };
        if format < 0 {
            unsafe {
                FFMS_DestroyVideoSource(video);
                FFMS_DestroyIndex(index);
            }
            bail!("this FFMS2 build has no yuv420p format");
        }
        let formats = [format, -1];
        let rc = unsafe {
            FFMS_SetOutputFormatV2(video, formats.as_ptr(), width, height, 0, &raw mut error)
        };
        if rc != 0 {
            unsafe {
                FFMS_DestroyVideoSource(video);
                FFMS_DestroyIndex(index);
            }
            bail!(
                "cannot set the output format for {}: {}",
                path.display(),
                error_text(&error)
            );
        }

        if num_frames <= 0 {
            unsafe {
                FFMS_DestroyVideoSource(video);
                FFMS_DestroyIndex(index);
            }
            bail!("{} reports {num_frames} frames", path.display());
        }
        if fps_num <= 0 || fps_den <= 0 {
            unsafe {
                FFMS_DestroyVideoSource(video);
                FFMS_DestroyIndex(index);
            }
            bail!("{} reports the frame rate {fps_num}/{fps_den}", path.display());
        }

        Ok(Self {
            source: video,
            index,
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

        let frame = unsafe { FFMS_GetFrame(self.source, number, &raw mut error) };
        if frame.is_null() {
            bail!("cannot read frame {number}: {}", error_text(&error));
        }
        let frame = unsafe { &*frame };

        let y_stride = frame.linesize[0] as usize;
        let uv_stride = frame.linesize[1] as usize;
        let h = frame.scaled_height as usize;
        let ch = h.div_ceil(2);

        // The plane buffers are `stride * rows` bytes. Build the slices from
        // the raw pointers, which FFMS2 keeps valid until the next call.
        let y = unsafe { std::slice::from_raw_parts(frame.data[0], y_stride * h) };
        let u = unsafe { std::slice::from_raw_parts(frame.data[1], uv_stride * ch) };
        let v = unsafe { std::slice::from_raw_parts(frame.data[2], uv_stride * ch) };

        Ok(Frame {
            y,
            u,
            v,
            y_stride,
            uv_stride,
        })
    }

    /// The frame rate as a floating point value.
    pub fn fps(&self) -> f64 {
        self.fps_num as f64 / self.fps_den as f64
    }
}

impl Drop for Decoder {
    fn drop(&mut self) {
        unsafe {
            FFMS_DestroyVideoSource(self.source);
            FFMS_DestroyIndex(self.index);
        }
    }
}
