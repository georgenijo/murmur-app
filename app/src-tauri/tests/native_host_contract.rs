//! Windowless executable: identity validation must perform no filesystem I/O,
//! create no NSApplication, request no TCC, and register no OS service.
use std::path::PathBuf;
use ui_lib::native_services::{validate_paths, NativePaths};
fn main() {
    let home = PathBuf::from("/synthetic-home");
    for identifier in [
        "com.localdictation",
        "com.localdictation.dev",
        "com.localdictation.bench",
        "future.id",
    ] {
        let paths = NativePaths {
            home: home.clone(),
            app: home.join("Library/Application Support").join(identifier),
            resources: home.join("test-resources"),
            cache: home.join("test-cache"),
        };
        assert!(validate_paths(&paths, identifier).is_ok());
        let mut wrong = paths.clone();
        wrong.app = home.join("Library/Application Support/com.localdictation");
        if identifier != "com.localdictation" {
            assert!(validate_paths(&wrong, identifier).is_err());
        }
    }
    for identifier in [
        "",
        "..",
        "../com.localdictation",
        "/com.localdictation",
        "a/b",
        "a\\b",
    ] {
        let paths = NativePaths {
            home: home.clone(),
            app: home.join("Library/Application Support").join(identifier),
            resources: home.clone(),
            cache: home.clone(),
        };
        assert!(validate_paths(&paths, identifier).is_err());
    }
    println!("Native host path/identity contracts passed (calculation only)");
}
