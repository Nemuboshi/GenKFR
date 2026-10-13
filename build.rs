//! Builds the vendored FFMS2 core.
//!
//! FFMS2 is a git submodule in `vendor/ffms2`, pinned to a commit. GenKFR
//! links it statically, so its core sources are compiled here. The submodule
//! also holds AviSynth and VapourSynth plugins, which GenKFR does not use, so
//! only `src/core` is built.
//!
//! FFmpeg must be available. FFMS2 is a wrapper around it, so its headers and
//! static libraries are required. Set `FFMPEG_DIR` to the prefix that holds
//! `include/` and `lib/`, or install FFmpeg with vcpkg.

use std::env;
use std::path::{Path, PathBuf};

fn main() {
    println!("cargo:rerun-if-changed=vendor/ffms2/src/core");
    println!("cargo:rerun-if-changed=vendor/ffms2/include");
    println!("cargo:rerun-if-env-changed=FFMPEG_DIR");

    let ffmpeg = find_ffmpeg();
    let include = ffmpeg.join("include");
    let lib = ffmpeg.join("lib");

    let mut build = cc::Build::new();
    build
        .cpp(true)
        .std("c++17")
        .opt_level(2)
        .warnings(false)
        // FFMS_STATIC turns FFMS_API into a plain declaration. Without it the
        // header declares the API as __declspec(dllimport) and defining those
        // functions is an error (C2491).
        .define("FFMS_STATIC", None)
        .define("NDEBUG", None)
        .define("_CRT_SECURE_NO_WARNINGS", None)
        .include("vendor/ffms2/include")
        .include("vendor/ffms2/src/core")
        .include(&include)
        .file("vendor/ffms2/src/core/audiosource.cpp")
        .file("vendor/ffms2/src/core/ffms.cpp")
        .file("vendor/ffms2/src/core/filehandle.cpp")
        .file("vendor/ffms2/src/core/indexing.cpp")
        .file("vendor/ffms2/src/core/track.cpp")
        .file("vendor/ffms2/src/core/utils.cpp")
        .file("vendor/ffms2/src/core/videosource.cpp")
        .file("vendor/ffms2/src/core/videoutils.cpp")
        .file("vendor/ffms2/src/core/zipfile.cpp");

    if let Some(target) = env::var_os("CARGO_CFG_TARGET_ENV") {
        if target == "msvc" {
            build.flag_if_supported("/EHsc");
        }
    }

    build.compile("ffms2");

    println!("cargo:rustc-link-search=native={}", lib.display());

    // FFmpeg's own libraries. The list follows FFMS2's Visual Studio project.
    //
    // The zlib archive has no stable file name. zlib 1.3.2 renamed it: the
    // static library is `zs.lib` and `z.lib` is the DLL import library, so a
    // static triplet has neither `zlib.lib` nor `z.lib`. Older ports ship
    // `zlib.lib`. Ask the prefix what it has, or the link fails with LNK1181.
    let zlib = pick_zlib(&lib);
    for name in [
        "avformat",
        "avcodec",
        "swscale",
        "swresample",
        "avutil",
        zlib,
    ] {
        println!("cargo:rustc-link-lib=static={name}");
    }

    // System libraries that FFmpeg needs on Windows.
    for name in [
        "bcrypt", "ncrypt", "ws2_32", "secur32", "mfplat", "mfuuid", "strmiids", "ole32", "user32",
        "crypt32", "psapi", "uuid", "oleaut32", "shlwapi", "gdi32", "vfw32",
    ] {
        println!("cargo:rustc-link-lib={name}");
    }
}

/// Name the zlib library to link, from the files the prefix actually has.
///
/// `zs` first: that is what vcpkg's static triplets ship since zlib 1.3.2.
/// Falls back to `z` on non-MSVC prefixes, where the archive is `libz.a`.
fn pick_zlib(lib: &Path) -> &'static str {
    for name in ["zs", "zlib", "z"] {
        if lib.join(format!("{name}.lib")).is_file() {
            return name;
        }
    }
    "z"
}

/// Find the FFmpeg prefix.
///
/// `FFMPEG_DIR` wins. Otherwise look for a vcpkg tree, which is what the
/// release workflow uses.
fn find_ffmpeg() -> PathBuf {
    if let Some(dir) = env::var_os("FFMPEG_DIR") {
        let path = PathBuf::from(dir);
        if path.join("include").is_dir() {
            return path;
        }
        panic!(
            "FFMPEG_DIR is set but has no include directory: {}",
            path.display()
        );
    }

    for candidate in vcpkg_candidates() {
        if candidate.join("include").is_dir() {
            return candidate;
        }
    }

    panic!(
        "cannot find FFmpeg. Set FFMPEG_DIR to the prefix that holds include/ and lib/, \
         or install FFmpeg with vcpkg. Looked in: {}",
        vcpkg_candidates()
            .iter()
            .map(|p| p.display().to_string())
            .collect::<Vec<_>>()
            .join(", ")
    );
}

fn vcpkg_candidates() -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Some(root) = env::var_os("VCPKG_ROOT") {
        for triplet in ["x64-windows-static-md-release", "x64-windows-static-md"] {
            out.push(Path::new(&root).join("installed").join(triplet));
        }
    }
    out
}
