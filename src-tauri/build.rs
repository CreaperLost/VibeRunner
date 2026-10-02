fn main() {
    // On Windows, embed the common-controls application manifest through
    // the linker for *every* target instead of letting tauri-build put it
    // in the app's resource file. The content is identical to
    // tauri-build's default (see windows-app-manifest.xml), but this way
    // the unit-test binary gets it too: Tauri's mock runtime (used by
    // lifecycle tests) needs comctl32 v6, and without the manifest the
    // test exe dies with STATUS_ENTRYPOINT_NOT_FOUND before running.
    let mut attrs = tauri_build::Attributes::new();
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows")
        && std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc")
    {
        let manifest = std::path::Path::new(&std::env::var("CARGO_MANIFEST_DIR").unwrap())
            .join("windows-app-manifest.xml");
        println!("cargo:rerun-if-changed=windows-app-manifest.xml");
        println!("cargo:rustc-link-arg=/MANIFEST:EMBED");
        println!("cargo:rustc-link-arg=/MANIFESTINPUT:{}", manifest.display());
        attrs = attrs.windows_attributes(tauri_build::WindowsAttributes::new_without_app_manifest());
    }
    tauri_build::try_build(attrs).expect("failed to run tauri-build");
}
