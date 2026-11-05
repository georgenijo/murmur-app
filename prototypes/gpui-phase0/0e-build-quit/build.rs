use std::{env, path::PathBuf, process::Command};
fn main() {
    let out = PathBuf::from(env::var_os("OUT_DIR").unwrap());
    assert!(
        Command::new("xcrun")
            .args([
                "clang",
                "-fobjc-arc",
                "-fblocks",
                "-c",
                "src/quit_hook.m",
                "-o"
            ])
            .arg(out.join("quit_hook.o"))
            .status()
            .unwrap()
            .success()
    );
    assert!(
        Command::new("ar")
            .arg("crs")
            .arg(out.join("libquit_hook.a"))
            .arg(out.join("quit_hook.o"))
            .status()
            .unwrap()
            .success()
    );
    println!("cargo:rustc-link-search=native={}", out.display());
    println!("cargo:rustc-link-lib=static=quit_hook");
    println!("cargo:rustc-link-lib=framework=AppKit");
    println!("cargo:rerun-if-changed=src/quit_hook.m");
}
