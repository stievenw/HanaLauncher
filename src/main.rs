#![windows_subsystem = "windows"]

mod app;
mod assets;
mod auth;
mod config;
mod download;
mod install;
mod java;
mod lang;
mod launch;
mod minecraft;
mod tasks;
mod util;
mod worker;

use app::HanaApp;
use std::sync::Arc;

/// Ensure only one HanaLauncher instance runs at a time. Uses a named mutex;
/// the handle is intentionally leaked so the lock lives for the whole process.
/// Returns `true` when another instance is already running.
fn single_instance_already_running() -> bool {
    const ERROR_ALREADY_EXISTS: u32 = 183;
    let name: Vec<u16> = "Global\\HanaLauncher_SingleInstance"
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();
    unsafe {
        let handle = win32::CreateMutexW(std::ptr::null(), 0, name.as_ptr());
        let already = win32::GetLastError() == ERROR_ALREADY_EXISTS;
        // Keep the handle alive for the whole process (never CloseHandle) so
        // the named mutex stays locked while this instance is running.
        let _ = handle;
        already
    }
}

#[allow(non_snake_case, non_camel_case_types)]
mod win32 {
    use std::os::raw::c_int;
    use std::os::raw::c_void;

    #[link(name = "kernel32")]
    unsafe extern "system" {
        pub fn CreateMutexW(
            lpMutexAttributes: *const c_void,
            bInitialOwner: c_int,
            lpName: *const u16,
        ) -> *mut c_void;
        pub fn GetLastError() -> u32;
    }
}

fn main() -> eframe::Result {
    let mut brand = crate::util::DEFAULT_BRAND.to_string();
    let mut channel = crate::util::DEFAULT_CHANNEL.to_string();
    let args: Vec<String> = std::env::args().collect();
    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--brand" => {
                i += 1;
                if let Some(v) = args.get(i) {
                    brand = v.clone();
                }
            }
            "--channel" => {
                i += 1;
                if let Some(v) = args.get(i) {
                    channel = v.clone();
                }
            }
            _ => {}
        }
        i += 1;
    }

    let warn_existing = single_instance_already_running();

    let mut viewport = eframe::egui::ViewportBuilder::default()
        .with_decorations(false)
        .with_inner_size([1024.0, 660.0])
        .with_min_inner_size([860.0, 540.0])
        .with_title("HanaLauncher — Minecraft Launcher");

    // Window icon (also shows in the taskbar). Falls back silently if the
    // resource is missing or cannot be decoded.
    if let Some(bytes) = crate::assets::icon_png() {
        if let Ok(icon) = image::load_from_memory(&bytes) {
            let rgba = icon.to_rgba8();
            let (w, h) = rgba.dimensions();
            viewport = viewport.with_icon(Arc::new(eframe::egui::IconData {
                rgba: rgba.into_raw(),
                width: w,
                height: h,
            }));
        }
    }

    let options = eframe::NativeOptions { viewport, ..Default::default() };
    eframe::run_native(
        "HanaLauncher",
        options,
        Box::new(move |cc| {
            Ok(Box::new(HanaApp::new(
                cc,
                brand.clone(),
                channel.clone(),
                warn_existing,
            )))
        }),
    )
}
