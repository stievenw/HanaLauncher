fn main() {
    println!("cargo:rerun-if-changed=assets/icon.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("assets/icon.ico");
        res.set_language(0x0409);
        // Metadata shown in Explorer -> Properties -> Details. The uninstaller
        // is built as a separate crate (uninstaller/) with its own metadata.
        res.set("CompanyName", "Hanakama");
        res.set("ProductName", "Hana Launcher");
        res.set("FileDescription", "Hana Launcher - Minecraft Launcher");
        res.set("OriginalFilename", "HanaLauncher.exe");
        res.set("LegalCopyright", "Copyright (c) 2026 Hanakama");
        res.set_version_info(winresource::VersionInfo::FILEVERSION, 0x00010000000A0000);
        res.set_version_info(winresource::VersionInfo::PRODUCTVERSION, 0x00010000000A0000);
        if let Err(e) = res.compile() {
            println!("cargo:warning=winresource failed: {e}");
        }
    }
}