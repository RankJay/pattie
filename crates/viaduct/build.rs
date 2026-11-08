//! On Linux the SDK plays the driver's part in the app's own process, so it
//! compiles the C core, the same one the Windows driver runs (ADR 0018). Nothing
//! is built for other targets.

use std::{env, fs, path::PathBuf};

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("linux") {
        return;
    }
    let manifest = PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("set by cargo"));
    let include = manifest.join("../../core/include");
    let core = manifest.join("../../core/src");
    let shim = manifest.join("src/linux/shim.c");

    let mut build = cc::Build::new();
    build
        .include(&include)
        .include(&core)
        .std("c17")
        .warnings(true)
        .warnings_into_errors(true)
        // Narrowing and sign changes are errors in code that reads hostile input,
        // as in the core's other builds.
        .flag("-Wconversion")
        .flag("-Wsign-conversion")
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
    build.compile("viaduct_core_linux");
}
