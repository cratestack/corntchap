fn main() {
    // Only the bundled picker page may call these. Remote origins get no IPC at all (see capabilities/).
    tauri_build::try_build(
        tauri_build::Attributes::new()
            .app_manifest(tauri_build::AppManifest::new().commands(&["get_server", "set_server"])),
    )
    .expect("failed to run tauri build script");
}
