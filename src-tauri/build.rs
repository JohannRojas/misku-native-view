fn main() {
    tauri_build::try_build(tauri_build::Attributes::new().app_manifest(
        tauri_build::AppManifest::new().commands(&[
            "manager_snapshot",
            "manager_create",
            "manager_update",
            "manager_open",
            "manager_remove",
        ]),
    ))
    .expect("failed to build Tauri application")
}
