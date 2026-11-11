//! Compiles the Box3D library (a git submodule in `box3d/`) and our C
//! interface over it (`shim/physics.c`) into static libraries.

use std::path::Path;

fn main() {
    let root = Path::new("box3d");
    let src = root.join("src");
    if !src.join("core.c").exists() {
        panic!("box3d/ is empty: run `git submodule update --init`");
    }
    println!("cargo::rerun-if-changed=shim");
    println!("cargo::rerun-if-changed=box3d/src");
    println!("cargo::rerun-if-changed=box3d/include");

    let msvc = std::env::var("CARGO_CFG_TARGET_ENV").is_ok_and(|env| env == "msvc");
    let x86 = std::env::var("CARGO_CFG_TARGET_ARCH").is_ok_and(|arch| arch == "x86_64" || arch == "x86");
    let release = std::env::var("PROFILE").is_ok_and(|profile| profile == "release");

    let mut sources: Vec<_> = std::fs::read_dir(&src)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "c"))
        .collect();
    sources.sort();
    // The 8-wide SIMD file needs AVX2 enabled for it alone: the library
    // checks at run time that the CPU has it before calling into it.
    let (wide, sources): (Vec<_>, Vec<_>) = sources.into_iter().partition(|path| path.file_name().unwrap() == "wide8.c");

    let base = || {
        let mut build = cc::Build::new();
        build.include(root.join("include")).include(&src).warnings(false).opt_level(3);
        if msvc {
            build.flag("/std:c17");
        } else {
            build.std("c17").flag_if_supported("-ffp-contract=off");
        }
        if release {
            build.define("NDEBUG", None);
        }
        build
    };
    let mut library = base();
    library.files(&sources).file("shim/physics.c");
    library.compile("box3d");

    let mut avx = base();
    avx.files(&wide);
    if msvc && x86 {
        avx.flag("/arch:AVX2");
    }
    avx.compile("box3d_wide");
}
