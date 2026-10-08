use std::{env, path::PathBuf, process::Command};
fn main() {
    let root = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").unwrap()).join("../..");
    println!("cargo:rerun-if-changed=../../VERSION");
    for path in [
        "src",
        "../../Cargo.toml",
        "../../desktop/src-tauri/src",
        "../../desktop/frontend/src",
        "../../python",
        "../../extension",
    ] {
        println!("cargo:rerun-if-changed={path}");
    }
    println!("cargo:rerun-if-changed=../../.git/HEAD");
    println!("cargo:rerun-if-changed=../../.git/index");
    println!("cargo:rerun-if-env-changed=GITHUB_SHA");
    let output = Command::new("git")
        .arg("-C")
        .arg(&root)
        .args(["rev-parse", "HEAD"])
        .output()
        .ok();
    let git = output
        .filter(|o| o.status.success())
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_owned());
    let commit = env::var("GITHUB_SHA")
        .ok()
        .or(git)
        .filter(|s| (s.len() == 40 || s.len() == 64) && s.bytes().all(|b| b.is_ascii_hexdigit()))
        .unwrap_or_else(|| "unknown".into());
    let dirty = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["status", "--porcelain", "--untracked-files=no"])
        .output()
        .map(|o| !o.status.success() || !o.stdout.is_empty())
        .unwrap_or(true);
    println!("cargo:rustc-env=RECALLCARD_BUILD_COMMIT={commit}");
    println!("cargo:rustc-env=RECALLCARD_BUILD_DIRTY={dirty}");
}
