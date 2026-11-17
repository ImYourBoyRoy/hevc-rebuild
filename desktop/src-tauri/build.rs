//! Build script: Tauri's generated context, plus the Windows application manifest.
//!
//! The dialog plugin imports `TaskDialogIndirect`, which only exists in Common
//! Controls v6, and Windows loads v6 only for an executable whose manifest
//! asks for it. `tauri-build` embeds that manifest into the application binary
//! alone, so the library's own unit-test executable failed to start on Windows
//! (`STATUS_ENTRYPOINT_NOT_FOUND`) and none of the 330+ tests could run there.
//! We hand the same manifest to every target ourselves and tell `tauri-build`
//! not to add its copy, so the application binary still carries it exactly once.

fn main() {
    // A build script sees the package's features only as environment variables.
    if std::env::var_os("CARGO_FEATURE_DESKTOP").is_some() {
        let windows = tauri_build::WindowsAttributes::new_without_app_manifest();
        tauri_build::try_build(tauri_build::Attributes::new().windows_attributes(windows))
            .expect("failed to run tauri-build");
    }

    println!("cargo:rerun-if-changed=windows-app.rc");
    println!("cargo:rerun-if-changed=windows-app.manifest");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        embed_resource::compile_for_everything("windows-app.rc", embed_resource::NONE)
            .manifest_required()
            .expect("failed to embed the Windows application manifest");
    }
}
