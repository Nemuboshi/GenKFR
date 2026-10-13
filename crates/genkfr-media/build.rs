//! Builds the vendored FFMS2 core and generates private native bindings.
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
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let ffms = root.join("vendor/ffms2");
    println!("cargo:rerun-if-changed={}", ffms.join("src/core").display());
    println!("cargo:rerun-if-changed={}", ffms.join("include").display());
    println!("cargo:rerun-if-env-changed=FFMPEG_DIR");
    println!("cargo:rerun-if-env-changed=XVIDCORE_DIR");
    println!("cargo:rerun-if-env-changed=LIBCLANG_PATH");
    println!("cargo:rerun-if-env-changed=GENKFR_ASAN");
    println!("cargo:rustc-check-cfg=cfg(native_asan)");

    let target = env::var("TARGET").unwrap();
    let native_asan = env::var("GENKFR_ASAN").as_deref() == Ok("1");
    let windows = target == "x86_64-pc-windows-msvc";
    assert!(
        windows || (target == "x86_64-unknown-linux-gnu" && native_asan),
        "native builds support Windows x64 MSVC, or Linux x64 for ASan tests"
    );
    let xvid = PathBuf::from(
        env::var_os("XVIDCORE_DIR")
            .expect("set XVIDCORE_DIR to the directory containing xvid.h and the static library"),
    );
    let xvid_header = xvid.join("xvid.h");
    let xvid_library = xvid.join(if windows {
        "xvidcore.lib"
    } else {
        "libxvidcore.a"
    });
    assert!(
        xvid_header.is_file() && xvid_library.is_file(),
        "XVIDCORE_DIR must contain matching xvid.h and the static xvidcore library"
    );
    println!("cargo:rerun-if-changed={}", xvid_header.display());
    println!("cargo:rerun-if-changed={}", xvid_library.display());

    bindgen::Builder::default()
        .header(ffms.join("include/ffms.h").to_str().unwrap())
        .header(xvid_header.to_str().unwrap())
        .clang_arg(format!("--target={target}"))
        .clang_arg("-DFFMS_STATIC")
        .allowlist_function("FFMS_(Init|GetPixFmt|CreateIndexer|DoIndexing2|DestroyIndex|GetFirstTrackOfType|CreateVideoSource|DestroyVideoSource|GetVideoProperties|GetFrame|SetOutputFormatV2)|xvid_(global|encore)")
        .allowlist_type("FFMS_(ErrorInfo|Frame|VideoProperties|VideoSource|Index|Indexer)|xvid_(image|gbl_init|gbl_info|enc_create|enc_frame|enc_stats)_t")
        .layout_tests(true)
        .parse_callbacks(Box::new(bindgen::CargoCallbacks::new()))
        .generate()
        .expect("cannot generate native bindings; install libclang and set LIBCLANG_PATH")
        .write_to_file(PathBuf::from(env::var_os("OUT_DIR").unwrap()).join("bindings.rs"))
        .expect("cannot write generated bindings");

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
        .include(ffms.join("include"))
        .include(ffms.join("src/core"))
        .include(&include);
    for name in [
        "audiosource",
        "ffms",
        "filehandle",
        "indexing",
        "track",
        "utils",
        "videosource",
        "videoutils",
        "zipfile",
    ] {
        build.file(ffms.join(format!("src/core/{name}.cpp")));
    }
    if windows {
        build.flag_if_supported("/EHsc");
    }
    if native_asan {
        if windows {
            build.flag("/fsanitize=address");
        } else {
            build
                .flag("-fsanitize=address")
                .flag("-fno-omit-frame-pointer");
        }
        build.debug(true);
        println!("cargo:rustc-cfg=native_asan");
    }

    build.compile("ffms2");
    println!("cargo:rustc-link-search=native={}", xvid.display());
    println!("cargo:rustc-link-lib=static=xvidcore");

    println!("cargo:rustc-link-search=native={}", lib.display());

    // FFmpeg's own libraries. The list follows FFMS2's Visual Studio project.
    //
    // The zlib archive has no stable file name. zlib 1.3.2 renamed it: the
    // static library is `zs.lib` and `z.lib` is the DLL import library, so a
    // static triplet has neither `zlib.lib` nor `z.lib`. Older ports ship
    // `zlib.lib`. Ask the prefix what it has, or the link fails with LNK1181.
    let zlib = if windows { pick_zlib(&lib) } else { "z" };
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

    if !windows {
        for name in ["m", "pthread", "dl"] {
            println!("cargo:rustc-link-lib={name}");
        }
        return;
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
