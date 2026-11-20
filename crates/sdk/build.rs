//! The In-process core (`src/backend/in_process/`) runs the C core in the app's own process,
//! the same core the Windows driver runs, so this compiles it: on Linux, where the
//! host is the backend (ADR 0018), and elsewhere only with the hidden
//! `fake-backend` feature (tests), so Windows app builds compile no C (ADR 0012).

use std::{env, fs, path::PathBuf};

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    let linux = env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("linux");
    if !linux && env::var_os("CARGO_FEATURE_FAKE_BACKEND").is_none() {
        return;
    }
    let manifest = PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("set by cargo"));
    // The published crate carries its own copy of the C core (publish.yml);
    // in the repo it is the shared one two folders up.
    let core_root = if manifest.join("core/src").is_dir() {
        manifest.join("core")
    } else {
        manifest.join("../../core")
    };
    let include = core_root.join("include");
    let core = core_root.join("src");
    let alloc = manifest.join("src/sys/alloc.c");

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
        .file(&alloc);
    println!("cargo:rerun-if-changed={}", alloc.display());
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
    build.compile("pattie_in_process_core");
}
