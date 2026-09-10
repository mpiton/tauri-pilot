const COMMANDS: &[&str] = &["callback", "__callback"];

fn main() {
    tauri_plugin::Builder::new(COMMANDS).build();
    embed_common_controls_manifest();
}

/// Link the Common Controls v6 manifest into this crate's test binaries.
///
/// `tauri::test::mock_app` pulls in `TaskDialogIndirect`, which only comctl32
/// v6 exports. An app gets the manifest from `tauri-build`, but the lib test
/// harness has none, so Windows refuses to load it with
/// `STATUS_ENTRYPOINT_NOT_FOUND` (tauri-apps/tauri#13419). Cargo applies link
/// args only to this package's own targets, never to the app using the plugin.
fn embed_common_controls_manifest() {
    let cfg = |key| std::env::var(key).ok();
    if cfg("CARGO_CFG_TARGET_OS").as_deref() == Some("windows")
        && cfg("CARGO_CFG_TARGET_ENV").as_deref() == Some("msvc")
    {
        println!("cargo:rustc-link-arg=/MANIFEST:EMBED");
        println!(
            "cargo:rustc-link-arg=/MANIFESTDEPENDENCY:type='win32' \
             name='Microsoft.Windows.Common-Controls' version='6.0.0.0' \
             processorArchitecture='*' publicKeyToken='6595b64144ccf1df' language='*'"
        );
    }
}
