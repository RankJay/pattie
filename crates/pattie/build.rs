//! The Core host (`src/core_host.rs`) runs the C core in the app's own process,
//! the same core the Windows driver runs, so this compiles it: on Linux, where the
//! host is the backend (ADR 0018), and elsewhere only with the hidden
//! `test-backend` feature (tests), so Windows app builds compile no C (ADR 0012).

use std::{env, fs, path::PathBuf};

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    let linux = env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("linux");
    if !linux && env::var_os("CARGO_FEATURE_TEST_BACKEND").is_none() {
        return;
    }
    let manifest = PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("set by cargo"));
    let include = manifest.join("../../core/include");
    let core = manifest.join("../../core/src");
    let shim = manifest.join("src/core_host/shim.c");

    let mut build = cc::Build::new();
    build
        .include(&include)
        .include(&core)
        .std("c17")
        .warnings(true)
        .warnings_into_errors(true)
        // Narrowing and sign changes are errors in code that reads hostile input,
        // as in the core's other builds (GCC and Clang; MSVC has no equivalent).
        .flag_if_supported("-Wconversion")
        .flag_if_supported("-Wsign-conversion")
        .file(&shim);
    println!("cargo:rerun-if-changed={}", shim.display());
    for dir in [&include, &core] {
        // Watching the folder too picks up newly added source files.
        println!("cargo:rerun-if-changed={}", dir.display());
        for entry in fs::read_dir(dir).expect("core sources") {
            let path = entry.expect("core source entry").path();
            println!("cargo:rerun-if-changed={}", path.display());
            if dir == &core && path.extension().is_some_and(|ext| ext == "c") {
                build.file(&path);
            }
        }
    }
    build.compile("pattie_core_host");
}
