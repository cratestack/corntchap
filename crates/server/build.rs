//! Picks the folder the web console is embedded from: `console/dist` when it was built (`just console-build`),
//! otherwise a one-page "console not built" placeholder, so `cargo build` never needs node.

use std::{env, fs, path::PathBuf};

fn main() {
    let manifest = PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
    let dist = manifest.join("../../console/dist");
    let placeholder = manifest.join("assets/console-missing.html");
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed={}", placeholder.display());
    println!("cargo:rerun-if-env-changed=CORNTCHAP_CONSOLE_DIST");

    let chosen = if let Ok(dir) = env::var("CORNTCHAP_CONSOLE_DIST") {
        PathBuf::from(dir)
    } else if dist.join("index.html").is_file() {
        // not `rerun-if-changed` on a missing dist: that would re-run this script on every build. `just
        // console-build` touches this file instead, which is enough to pick up a fresh dist.
        println!("cargo:rerun-if-changed={}", dist.display());
        dist.canonicalize().expect("console/dist")
    } else {
        let out = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR")).join("console-missing");
        fs::create_dir_all(&out).expect("create placeholder dir");
        fs::copy(&placeholder, out.join("index.html")).expect("copy placeholder");
        println!("cargo:rustc-env=CORNTCHAP_CONSOLE_MISSING=1");
        out
    };
    println!("cargo:rustc-env=CONSOLE_DIST={}", chosen.display());
}
