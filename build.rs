fn main() {
    #[cfg(feature = "desktop")]
    {
        tauri_build::build();
        // Tauri embeds its Windows manifest in the app binary, not Cargo examples.
        // The native fixture also needs Common Controls v6 for TaskDialogIndirect.
        if std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc") {
            println!("cargo:rerun-if-changed=examples/windows.manifest");
            println!("cargo:rustc-link-arg-examples=/MANIFEST:EMBED");
            println!("cargo:rustc-link-arg-examples=/MANIFESTINPUT:examples/windows.manifest");
        }
    }
}
