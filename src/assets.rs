#![allow(dead_code)]
//! Runtime resources (icon, background artwork, Monogram fonts). They live in
//! a `resources/` folder next to the executable (installed there by the setup,
//! see setup/HanaLauncher.wxs) instead of being embedded, which keeps the
//! launcher exe itself small. The files are XOR-packed so the artwork is not
//! trivially extractable. Repack after editing the originals with
//! `tools\obfuscate-assets.ps1`.

use std::path::PathBuf;

/// Repeating XOR key. Must match the `$key` in tools\obfuscate-assets.ps1.
const KEY: &[u8] = &[0x5A, 0x3C, 0xA7];

/// The folder holding the packed resources, next to the executable (installed
/// layout). Falls back to `./resources` for running from a build directory or
/// a dev checkout.
pub fn resources_dir() -> PathBuf {
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            let p = dir.join("resources");
            if p.is_dir() {
                return p;
            }
        }
    }
    std::env::current_dir()
        .unwrap_or_else(|_| PathBuf::from("."))
        .join("resources")
}

/// De-obfuscate a packed asset (XOR with the repeating key).
pub fn unpack(data: &[u8]) -> Vec<u8> {
    data.iter()
        .enumerate()
        .map(|(i, b)| b ^ KEY[i % KEY.len()])
        .collect()
}

/// Read a packed asset from disk. Returns `None` when the resources folder is
/// missing (the app then degrades gracefully: system font, no artwork).
pub fn load_packed(name: &str) -> Option<Vec<u8>> {
    let path = resources_dir().join(name);
    let bytes = std::fs::read(&path).ok()?;
    Some(unpack(&bytes))
}

/// The launcher icon (PNG, window icon + in-app logo).
pub fn icon_png() -> Option<Vec<u8>> {
    load_packed("icon.png.x")
}

/// The window background artwork (JPEG).
pub fn background_jpg() -> Option<Vec<u8>> {
    load_packed("bg.jpg.x")
}

/// The Monogram pixel font (regular).
pub fn monogram_ttf() -> Option<Vec<u8>> {
    load_packed("monogram.ttf.x")
}

/// The Monogram pixel font (italic).
pub fn monogram_italic_ttf() -> Option<Vec<u8>> {
    load_packed("monogram-italic.ttf.x")
}