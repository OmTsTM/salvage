fn main() {
    // The embedded manifest asks Windows for elevation. Raw sector access needs
    // Administrator, and without the UAC prompt the program would open blind,
    // unable to see a single disk.
    let windows = tauri_build::WindowsAttributes::new().app_manifest(include_str!("app.manifest"));

    tauri_build::try_build(tauri_build::Attributes::new().windows_attributes(windows))
        .expect("failed to prepare the Tauri build");
}
