// XG Editor — Rust + egui(原生入口)
// wasm 入口在 lib.rs 的 WebHandle。
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use eframe::egui;
use xg_editor::XgApp;

#[cfg(not(target_arch = "wasm32"))]
fn main() -> eframe::Result {
    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1280.0, 820.0])
            .with_title("XG Editor — egui"),
        ..Default::default()
    };
    eframe::run_native(
        "xg-editor-app",
        native_options,
        Box::new(|_cc| Ok(Box::new(XgApp::default()))),
    )
}

// wasm 下 bin 也需要一个 main(实际逻辑走 lib.rs 的 WebHandle)
#[cfg(target_arch = "wasm32")]
fn main() {}
