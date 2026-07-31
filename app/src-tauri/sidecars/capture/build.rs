use std::path::PathBuf;

fn main() {
    println!("cargo:rerun-if-changed=Info.plist");
    println!("cargo:rerun-if-changed=WorkerInfo.plist");
    println!("cargo:rerun-if-env-changed=MURMUR_CAPTURE_ROLE");
    println!("cargo:rerun-if-env-changed=MURMUR_APP_VERSION");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos") {
        let manifest_dir = PathBuf::from(
            std::env::var_os("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR is set"),
        );
        let role = std::env::var("MURMUR_CAPTURE_ROLE").unwrap_or_else(|_| "helper".to_string());
        let template = match role.as_str() {
            "helper" => manifest_dir.join("Info.plist"),
            "worker" => manifest_dir.join("WorkerInfo.plist"),
            _ => panic!("MURMUR_CAPTURE_ROLE must be helper or worker"),
        };
        let version = std::env::var("MURMUR_APP_VERSION")
            .unwrap_or_else(|_| std::env::var("CARGO_PKG_VERSION").unwrap());
        let payload = std::fs::read_to_string(template)
            .expect("capture executable Info.plist template is readable")
            .replace("__MURMUR_VERSION__", &version);
        let output = PathBuf::from(std::env::var_os("OUT_DIR").expect("OUT_DIR is set"))
            .join(format!("capture-{role}-Info.plist"));
        std::fs::write(&output, payload)
            .expect("generated capture executable Info.plist is writable");
        println!(
            "cargo:rustc-link-arg=-Wl,-sectcreate,__TEXT,__info_plist,{}",
            output.display()
        );
    }
}
