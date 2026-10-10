//! Compiles the C hosts (OpenFX and CLAP need C: OpenFX's parameter suite is
//! variadic) and, for the tests, two small plugins in the layouts real ones
//! ship in: an OpenFX bundle and a `.clap` library.

use std::env;
use std::path::{Path, PathBuf};
use std::process::Command;

fn main() {
    println!("cargo:rerun-if-changed=csrc");
    println!("cargo:rerun-if-changed=vendor");
    if env::var("CARGO_CFG_UNIX").is_err() {
        return; // the helper loads plugins with dlopen
    }
    let ofx = Path::new("vendor/openfx/include");
    let clap = Path::new("vendor/clap/include");
    cc::Build::new()
        .files([
            "csrc/props.c",
            "csrc/ofx_host.c",
            "csrc/clap_host.c",
            "csrc/stdio.c",
        ])
        .include(ofx)
        .include(clap)
        .define("_GNU_SOURCE", None)
        .flag_if_supported("-std=c11")
        .warnings(true)
        .compile("debut_plugin_host");
    if env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("linux") {
        println!("cargo:rustc-link-lib=dl");
        println!("cargo:rustc-link-lib=pthread");
    }

    // Test plugins, built as shared libraries.
    let out = PathBuf::from(env::var("OUT_DIR").unwrap()).join("plugins");
    let arch = match env::var("CARGO_CFG_TARGET_OS").as_deref() {
        Ok("macos") => "MacOS",
        _ if env::var("CARGO_CFG_TARGET_ARCH").as_deref() == Ok("aarch64") => "Linux-arm64",
        _ => "Linux-x86-64",
    };
    let bundle = out.join("TestGain.ofx.bundle/Contents").join(arch);
    std::fs::create_dir_all(&bundle).unwrap();
    shared("csrc/test_ofx_gain.c", ofx, &bundle.join("TestGain.ofx"));
    shared("csrc/test_clap_gain.c", clap, &out.join("test-gain.clap"));
    println!("cargo:rustc-env=DEBUT_TEST_PLUGINS={}", out.display());
}

fn shared(src: &str, include: &Path, dst: &Path) {
    let compiler = cc::Build::new().get_compiler();
    let mut cmd: Command = compiler.to_command();
    cmd.args(["-shared", "-fPIC", "-std=c11", "-O2"])
        .arg("-I")
        .arg(include)
        .arg(src)
        .arg("-o")
        .arg(dst);
    if env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos") {
        cmd.arg("-undefined").arg("dynamic_lookup");
    }
    let status = cmd.status().expect("run the C compiler");
    assert!(status.success(), "building {src} failed");
}
