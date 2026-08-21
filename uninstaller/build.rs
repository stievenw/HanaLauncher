fn main() {
    println!("cargo:rerun-if-changed=assets/icon.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("../assets/icon.ico");
        res.set_language(0x0409);
        // Distinct metadata so Task Manager / Explorer never show the
        // uninstaller as the main "Hana Launcher" process.
        res.set("ProductName", "Hana Launcher - Uninstaller");
        res.set("FileDescription", "Hana Launcher - Uninstaller");
        res.set("OriginalFilename", "Uninstall.exe");
        res.set("LegalCopyright", "Copyright (c) 2026 Hanakama");
        res.set_version_info(winresource::VersionInfo::FILEVERSION, 0x00010000000A0000);
        res.set_version_info(winresource::VersionInfo::PRODUCTVERSION, 0x00010000000A0000);
        if let Err(e) = res.compile() {
            println!("cargo:warning=winresource failed: {e}");
        }
    }
}