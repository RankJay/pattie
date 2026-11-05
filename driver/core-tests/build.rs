//! Compiles the pure C driver core for the host and generates Rust bindings
//! for its interface and the shared ABI header (ADR 0002).

use std::{env, fs, path::PathBuf};

fn main() {
    let driver = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap()).join("..");
    let include = driver.join("include");
    let core = driver.join("core");
    let probe = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap()).join("probe");

    let mut build = cc::Build::new();
    build
        .include(&include)
        .include(&core)
        .include(&probe)
        .std("c17")
        .warnings(true)
        .warnings_into_errors(true);
    for dir in [&include, &core, &probe] {
        // Watching the folder too picks up newly added source files.
        println!("cargo:rerun-if-changed={}", dir.display());
        for entry in fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            println!("cargo:rerun-if-changed={}", path.display());
            if dir != &include && path.extension().is_some_and(|ext| ext == "c") {
                build.file(&path);
            }
        }
    }
    build.compile("viaduct_core");

    bindgen::Builder::default()
        .header(core.join("viaduct_core.h").to_string_lossy())
        .header(probe.join("abi_probe.h").to_string_lossy())
        .clang_arg(format!("-I{}", include.display()))
        .allowlist_item("(vd|VD|VIADUCT)_.*")
        .rust_edition(bindgen::RustEdition::Edition2024)
        .generate()
        .expect("generate bindings for the driver core")
        .write_to_file(PathBuf::from(env::var("OUT_DIR").unwrap()).join("bindings.rs"))
        .unwrap();
}
