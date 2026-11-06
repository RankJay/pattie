//! Compiles the pure C driver core for the host and generates its bindings, the
//! same way the core tests do (ADR 0002).

use std::{env, fs, path::PathBuf};

fn main() {
    let repo = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap()).join("../..");
    let include = repo.join("core/include");
    let core = repo.join("core/src");

    let mut build = cc::Build::new();
    build
        .include(&include)
        .include(&core)
        .std("c17")
        .warnings(true)
        .warnings_into_errors(true);
    for dir in [&include, &core] {
        println!("cargo:rerun-if-changed={}", dir.display());
        for entry in fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            println!("cargo:rerun-if-changed={}", path.display());
            if dir == &core && path.extension().is_some_and(|ext| ext == "c") {
                build.file(&path);
            }
        }
    }
    build.compile("viaduct_core_fake");

    bindgen::Builder::default()
        .header(core.join("viaduct_core.h").to_string_lossy())
        .clang_arg(format!("-I{}", include.display()))
        .allowlist_item("(vd|VD|VIADUCT)_.*")
        .rust_edition(bindgen::RustEdition::Edition2024)
        .generate()
        .expect("generate bindings for the driver core")
        .write_to_file(PathBuf::from(env::var("OUT_DIR").unwrap()).join("bindings.rs"))
        .unwrap();
}
