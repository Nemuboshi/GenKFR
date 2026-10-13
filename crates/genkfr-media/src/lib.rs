//! Safe decoding and scene detection for GenKFR.
//! Native declarations are generated from the headers and stay private.

#![deny(unsafe_op_in_unsafe_fn, clippy::undocumented_unsafe_blocks)]

mod decode;
mod plane;
mod xvid;

#[allow(
    non_camel_case_types,
    non_snake_case,
    non_upper_case_globals,
    dead_code
)]
mod ffi {
    include!(concat!(env!("OUT_DIR"), "/bindings.rs"));
}

pub use decode::{Decoder, Frame};
pub use xvid::{Xvid, actual_version, probe_threads};
