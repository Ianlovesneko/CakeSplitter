use std::{env, fs, path::Path};

fn copy(source_root: &Path, staging_root: &Path, relative: &str) {
    let source = source_root.join(relative);
    let destination = staging_root.join(relative);
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent).expect("failed to create Tauri build staging directory");
    }
    fs::copy(&source, &destination).unwrap_or_else(|error| {
        panic!(
            "failed to stage {} for the Tauri build: {error}",
            source.display()
        )
    });
}

fn main() {
    for relative in [
        "tauri.conf.json",
        "capabilities/main.json",
        "permissions/cakesplitter.toml",
        "icons/icon.ico",
    ] {
        println!("cargo:rerun-if-changed={relative}");
    }

    let source_root = env::current_dir().expect("Tauri source directory is unavailable");
    let out_dir = env::var_os("OUT_DIR").expect("Cargo did not provide OUT_DIR");
    let staging_root = Path::new(&out_dir).join("tauri-build-input");
    for relative in [
        "tauri.conf.json",
        "capabilities/main.json",
        "permissions/cakesplitter.toml",
        "icons/icon.ico",
    ] {
        copy(&source_root, &staging_root, relative);
    }
    fs::write(
        staging_root.join("Cargo.toml"),
        format!(
            "[package]\nname = \"cakesplitter-desktop\"\nversion = \"{}\"\nedition = \"2024\"\n\n[dependencies]\ntauri = \"2\"\ntauri-plugin-dialog = \"2\"\n",
            env!("CARGO_PKG_VERSION")
        ),
    )
    .expect("failed to stage standalone Tauri Cargo metadata");

    env::set_current_dir(&staging_root).expect("failed to enter Tauri build staging directory");
    tauri_build::try_build(tauri_build::Attributes::new())
        .expect("failed to build CakeSplitter Desktop resources");
    env::set_current_dir(source_root).expect("failed to restore Tauri source directory");
}
