#![windows_subsystem = "windows"]
//! Uninstall.exe - full, clean uninstaller for Hana Launcher.
//!
//! Shipped in the install root. Shows the standard Windows Installer uninstall
//! wizard (the default "uninstall" appearance every app uses), and afterwards
//! performs a full cleanup:
//!   1. kill a running HanaLauncher.exe
//!   2. wipe the user data folder (%APPDATA%\Hana\HanaLauncher) - accounts,
//!      settings and cache - plus any launcher data in %LOCALAPPDATA%\Hana
//!   3. remove the HKCU\Software\Hana registry keys
//!   4. run `msiexec /x` WITH the standard UI (confirmation + progress bar)
//!   5. remove only the launcher's own program files
//!
//! SAFETY: uninstall removes the launcher's program files and its own config
//! data only. The install folder is the user's global .minecraft folder (the
//! setup default) and is preserved completely - saves, worlds, mods, versions,
//! libraries, assets, runtime and everything else (including data written by
//! the official Mojang launcher) survive uninstall.

use std::env;
use std::os::windows::process::CommandExt;
use std::path::Path;
use std::process::Command;
use std::time::Duration;

const CREATE_NO_WINDOW: u32 = 0x08000000;

/// ProductCode of the Hana Launcher MSI (must match HanaLauncher.wxs).
const PRODUCT_CODE: &str = "{B4A9D1C8-6E3F-4A2B-9C7E-5F0D8A1B2C3D}";

fn run_quiet(prog: &str, args: &[&str]) {
    let _ = Command::new(prog).args(args).creation_flags(CREATE_NO_WINDOW).status();
}

fn remove_dir(path: &Path) {
    let _ = std::fs::remove_dir_all(path);
}

fn main() {
    let install_dir = match env::current_exe() {
        Ok(p) => match p.parent() {
            Some(d) => d.to_path_buf(),
            None => return,
        },
        Err(_) => return,
    };

    // 1. close a running launcher
    run_quiet("taskkill", &["/IM", "HanaLauncher.exe", "/F"]);
    std::thread::sleep(Duration::from_millis(800));

    // 2. wipe user data (accounts, settings, cache)
    if let Ok(dir) = env::var("APPDATA") {
        remove_dir(&Path::new(&dir).join("Hana").join("HanaLauncher"));
    }
    if let Ok(dir) = env::var("LOCALAPPDATA") {
        remove_dir(&Path::new(&dir).join("Hana").join("HanaLauncher"));
        remove_dir(&Path::new(&dir).join("Hana"));
    }

    // 3. remove registry keys
    run_quiet("reg", &["delete", "HKCU\\Software\\Hana\\HanaLauncher", "/f"]);
    run_quiet("reg", &["delete", "HKCU\\Software\\Hana", "/f"]);

    // 4. run the MSI uninstall WITH the standard Windows Installer UI
    //    (confirmation dialog + progress). Runs synchronously so the user
    //    sees the wizard.
    let _ = Command::new("msiexec")
        .args(["/x", PRODUCT_CODE, "/norestart"])
        .status();

    // 5. remove only the launcher's own program files. The install folder is
    //    the user's global .minecraft folder (the setup default) and must be
    //    preserved completely - saves, worlds, mods, versions, libraries,
    //    assets, runtime and everything else inside it survive uninstall.
    for name in ["HanaLauncher.exe", "Uninstall.exe"] {
        let _ = std::fs::remove_file(install_dir.join(name));
    }
    if let Ok(rd) = std::fs::read_dir(install_dir.join("resources")) {
        for e in rd.flatten() {
            let _ = std::fs::remove_file(e.path());
        }
        let _ = std::fs::remove_dir(install_dir.join("resources"));
    }
}