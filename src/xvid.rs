//! Minimal FFI to xvidcore, enough to reproduce SCXvid's frame type decisions.
//!
//! GenKFR previously reached this code through the VapourSynth `scxvid` plugin.
//! The plugin adds no algorithm: it builds an encoder, feeds frames, and reads
//! back `stats.type`. Calling `xvid_encore` directly gives the same result.
//!
//! GenKFR passes no `log` argument to `scxvid.Scxvid`. The plugin therefore
//! fails its own create step, because `rc_2pass1_create` rejects a null
//! filename (`plugin_2pass1.c:53`). `xvidcore` then disables it
//! (`encoder.c:210`), and `call_plugins` skips it. The plugin never runs, so
//! its `FAST1PASS` flag rewrite never runs either.
//!
//! The values below are thus exactly the ones `scxvid.cpp` writes into the
//! frame. Do not "optimise" them.
//!
//! One value is still substituted: `call_plugins` runs even with zero plugins,
//! and `encoder.c:904` replaces a `quant` of zero with 2.

use anyhow::{Result, anyhow, bail};
use std::ffi::c_void;
use std::os::raw::{c_char, c_int, c_uint};

// ---- constants -----------------------------------------------------------

/// `XVID_MAKE_VERSION(1,3,7)`. Only the major byte is checked
/// (`xvid.c:203,699`, `encoder.c:143,1064`), but keep the whole value.
const XVID_VERSION: c_int = (1 << 16) | (3 << 8) | 7;

const XVID_GBL_INIT: c_int = 0;
const XVID_GBL_INFO: c_int = 1;

const XVID_ENC_CREATE: c_int = 0;
const XVID_ENC_DESTROY: c_int = 1;
const XVID_ENC_ENCODE: c_int = 2;

const XVID_TYPE_AUTO: c_int = 0;
pub const XVID_TYPE_IVOP: c_int = 1;

const XVID_CSP_PLANAR: c_int = 1 << 0;

// The values from scxvid.cpp:203-216. See the module comment.
pub const VOL_FLAGS: c_int = 0;
pub const VOP_FLAGS: c_int = (1 << 8)  // MODEDECISION_RD
    | (1 << 1)                     // HALFPEL
    | (1 << 7)                     // HQACPRED
    | (1 << 3)                     // TRELLISQUANT
    | (1 << 2); // INTER4V
pub const MOTION_FLAGS: c_int = (1 << 12) // CHROMA_PVOP
    | (1 << 13)                       // CHROMA_BVOP
    | (1 << 4)                        // HALFPELREFINE16
    | (1 << 10)                       // EXTSEARCH16
    | (1 << 6)                        // HALFPELREFINE8
    | (1 << 2); // USESQUARES16

// scxvid.cpp writes 0, and xvidcore substitutes 2. Send 2 directly.
pub const QUANT: c_int = 2;
const MAX_KEY_INTERVAL: c_int = 10_000_000;
const FINCR: c_int = 1;
const FBASE: c_int = 1;

/// Same value as SCXVID_BUFFER_SIZE. xvid writes the real bitstream here.
const BUFFER_SIZE: usize = 4 * 1024 * 1024;

// ---- structures ----------------------------------------------------------
// Field order copied from xvid.h. A mismatch is silent memory corruption, so
// the size assertions at the bottom of this file guard the layout.

#[repr(C)]
struct XvidImage {
    csp: c_int,
    plane: [*mut c_void; 4],
    stride: [c_int; 4],
}

#[repr(C)]
struct XvidGblInit {
    version: c_int,
    cpu_flags: c_uint,
    debug: c_int,
}

#[repr(C)]
struct XvidGblInfo {
    version: c_int,
    actual_version: c_int,
    build: *const c_char,
    cpu_flags: c_uint,
    num_threads: c_int,
}

#[repr(C)]
struct XvidEncCreate {
    version: c_int,
    profile: c_int,
    width: c_int,
    height: c_int,
    num_zones: c_int,
    zones: *mut c_void,
    num_plugins: c_int,
    plugins: *mut c_void,
    num_threads: c_int,
    max_bframes: c_int,
    global: c_int,
    fincr: c_int,
    fbase: c_int,
    max_key_interval: c_int,
    frame_drop_ratio: c_int,
    bquant_ratio: c_int,
    bquant_offset: c_int,
    min_quant: [c_int; 3],
    max_quant: [c_int; 3],
    handle: *mut c_void,
    start_frame_num: c_int,
    num_slices: c_int,
}

#[repr(C)]
struct XvidEncFrame {
    version: c_int,
    vol_flags: c_int,
    quant_intra_matrix: *mut u8,
    quant_inter_matrix: *mut u8,
    par: c_int,
    par_width: c_int,
    par_height: c_int,
    fincr: c_int,
    vop_flags: c_int,
    motion: c_int,
    input: XvidImage,
    type_: c_int,
    quant: c_int,
    bframe_threshold: c_int,
    bitstream: *mut c_void,
    length: c_int,
    out_flags: c_int,
}

#[repr(C)]
struct XvidEncStats {
    version: c_int,
    type_: c_int,
    quant: c_int,
    vol_flags: c_int,
    vop_flags: c_int,
    length: c_int,
    hlength: c_int,
    kblks: c_int,
    mblks: c_int,
    ublks: c_int,
    sse_y: c_int,
    sse_u: c_int,
    sse_v: c_int,
}

// Linked statically: GenKFR builds xvidcore from source, so no DLL is needed.
#[link(name = "xvidcore", kind = "static")]
unsafe extern "C" {
    fn xvid_global(handle: *mut c_void, opt: c_int, p1: *mut c_void, p2: *mut c_void) -> c_int;
    fn xvid_encore(handle: *mut c_void, opt: c_int, p1: *mut c_void, p2: *mut c_void) -> c_int;
}

// ---- public API ----------------------------------------------------------

/// Read the thread count that xvidcore detects.
///
/// SCXvid gives this value to both `num_threads` and `num_slices`. Use it
/// instead of `available_parallelism`, or the output differs from the
/// reference on the same machine.
pub fn probe_threads() -> Result<c_int> {
    let mut info = XvidGblInfo {
        version: XVID_VERSION,
        actual_version: 0,
        build: std::ptr::null(),
        cpu_flags: 0,
        num_threads: 0,
    };
    let rc = unsafe {
        xvid_global(
            std::ptr::null_mut(),
            XVID_GBL_INFO,
            (&raw mut info).cast(),
            std::ptr::null_mut(),
        )
    };
    if rc != 0 {
        bail!("xvid_global(XVID_GBL_INFO) returned {rc}");
    }
    if info.num_threads <= 0 {
        bail!("xvidcore reported {} threads", info.num_threads);
    }
    Ok(info.num_threads)
}

/// The version string that xvidcore reports, for bug reports.
pub fn actual_version() -> Result<(u8, u8, u8)> {
    let mut info = XvidGblInfo {
        version: XVID_VERSION,
        actual_version: 0,
        build: std::ptr::null(),
        cpu_flags: 0,
        num_threads: 0,
    };
    let rc = unsafe {
        xvid_global(
            std::ptr::null_mut(),
            XVID_GBL_INFO,
            (&raw mut info).cast(),
            std::ptr::null_mut(),
        )
    };
    if rc != 0 {
        bail!("xvid_global(XVID_GBL_INFO) returned {rc}");
    }
    let v = info.actual_version;
    Ok((
        ((v >> 16) & 0xff) as u8,
        ((v >> 8) & 0xff) as u8,
        (v & 0xff) as u8,
    ))
}

pub struct Xvid {
    handle: *mut c_void,
    buffer: Vec<u8>,
    height: c_int,
}

impl Xvid {
    /// Make an encoder for one clip.
    ///
    /// `num_threads` comes from [`probe_threads`]. Width and height must be
    /// even: `xvid_enc_create` rejects odd values (`encoder.c:146-150`).
    pub fn new(width: c_int, height: c_int, num_threads: c_int) -> Result<Self> {
        if width <= 0 || height <= 0 || width % 2 != 0 || height % 2 != 0 {
            bail!("xvidcore needs a positive even size, got {width}x{height}");
        }

        let mut init = XvidGblInit {
            version: XVID_VERSION,
            cpu_flags: 0,
            debug: 0,
        };
        let rc = unsafe {
            xvid_global(
                std::ptr::null_mut(),
                XVID_GBL_INIT,
                (&raw mut init).cast(),
                std::ptr::null_mut(),
            )
        };
        if rc != 0 {
            bail!("xvid_global(XVID_GBL_INIT) returned {rc}");
        }

        let mut create = XvidEncCreate {
            version: XVID_VERSION,
            profile: 0,
            width,
            height,
            num_zones: 0,
            zones: std::ptr::null_mut(),
            // No plugin. See the module comment: GenKFR's call makes the
            // 2-pass-1 plugin fail to create, so it never runs.
            num_plugins: 0,
            plugins: std::ptr::null_mut(),
            num_threads,
            max_bframes: 0,
            global: 0,
            fincr: FINCR,
            fbase: FBASE,
            max_key_interval: MAX_KEY_INTERVAL,
            frame_drop_ratio: 0,
            bquant_ratio: 0,
            bquant_offset: 0,
            min_quant: [0; 3],
            max_quant: [0; 3],
            handle: std::ptr::null_mut(),
            start_frame_num: 0,
            num_slices: num_threads,
        };

        let rc = unsafe {
            xvid_encore(
                std::ptr::null_mut(),
                XVID_ENC_CREATE,
                (&raw mut create).cast(),
                std::ptr::null_mut(),
            )
        };
        if rc != 0 {
            // -4 is XVID_ERR_VERSION, the usual cause of a silent init failure.
            bail!("xvid_encore(XVID_ENC_CREATE) returned {rc}");
        }
        if create.handle.is_null() {
            bail!("xvid_encore(XVID_ENC_CREATE) returned a null handle");
        }

        Ok(Self {
            handle: create.handle,
            buffer: vec![0u8; BUFFER_SIZE],
            height,
        })
    }

    /// Give one frame to the encoder.
    ///
    /// Returns true when xvid codes the frame as an I-VOP, which is a scene
    /// cut. The encoder copies the plane data, so the caller can reuse the
    /// buffers after this returns.
    pub fn is_keyframe(
        &mut self,
        y: &[u8],
        u: &[u8],
        v: &[u8],
        y_stride: c_int,
        uv_stride: c_int,
    ) -> Result<bool> {
        let need_y = (self.height as usize) * (y_stride as usize);
        if y.len() < need_y {
            bail!("luma plane too small: {} < {need_y}", y.len());
        }

        let mut frame = XvidEncFrame {
            version: XVID_VERSION,
            vol_flags: VOL_FLAGS,
            quant_intra_matrix: std::ptr::null_mut(),
            quant_inter_matrix: std::ptr::null_mut(),
            par: 0,
            par_width: 0,
            par_height: 0,
            fincr: 0,
            vop_flags: VOP_FLAGS,
            motion: MOTION_FLAGS,
            input: XvidImage {
                csp: XVID_CSP_PLANAR,
                plane: [
                    y.as_ptr().cast_mut().cast(),
                    u.as_ptr().cast_mut().cast(),
                    v.as_ptr().cast_mut().cast(),
                    std::ptr::null_mut(),
                ],
                stride: [y_stride, uv_stride, uv_stride, 0],
            },
            type_: XVID_TYPE_AUTO,
            quant: QUANT,
            bframe_threshold: 0,
            bitstream: self.buffer.as_mut_ptr().cast(),
            length: BUFFER_SIZE as c_int,
            out_flags: 0,
        };

        let mut stats = XvidEncStats {
            version: XVID_VERSION,
            type_: 0,
            quant: 0,
            vol_flags: 0,
            vop_flags: 0,
            length: 0,
            hlength: 0,
            kblks: 0,
            mblks: 0,
            ublks: 0,
            sse_y: 0,
            sse_u: 0,
            sse_v: 0,
        };

        let rc = unsafe {
            xvid_encore(
                self.handle,
                XVID_ENC_ENCODE,
                (&raw mut frame).cast(),
                (&raw mut stats).cast(),
            )
        };
        if rc < 0 {
            return Err(anyhow!("xvid_encore(XVID_ENC_ENCODE) returned {rc}"));
        }

        if std::env::var("XVID_TRACE").is_ok() {
            eprintln!(
                "xvid type={} quant={} len={} kblks={} mblks={}",
                stats.type_, stats.quant, stats.length, stats.kblks, stats.mblks
            );
        }
        Ok(stats.type_ == XVID_TYPE_IVOP)
    }
}

impl Drop for Xvid {
    fn drop(&mut self) {
        unsafe {
            xvid_encore(
                self.handle,
                XVID_ENC_DESTROY,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            );
        }
    }
}

// ---- layout guards -------------------------------------------------------
// These fail the build, not the run, if a struct drifts from xvid.h.

const _: () = {
    assert!(std::mem::size_of::<XvidImage>() == 8 + 32 + 16);
    assert!(std::mem::size_of::<XvidGblInit>() == 12);
    assert!(std::mem::size_of::<XvidGblInfo>() == 8 + 8 + 8);
    assert!(std::mem::size_of::<XvidEncStats>() == 13 * 4);
    // 64-bit pointer, no padding surprises.
    assert!(std::mem::size_of::<XvidEncCreate>() % 8 == 0);
    assert!(std::mem::size_of::<XvidEncFrame>() % 8 == 0);
};
