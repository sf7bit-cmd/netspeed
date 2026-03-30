#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod measure;
mod scan;
mod ui;

use eframe::{egui, NativeOptions};
use egui::ViewportBuilder;

fn main() -> eframe::Result<()> {
    let options = NativeOptions {
        viewport: ViewportBuilder::default()
            .with_title("NetSpeed Analyzer")
            .with_inner_size([980.0, 680.0])
            .with_min_inner_size([640.0, 480.0]),
        ..Default::default()
    };
    eframe::run_native(
        "NetSpeed Analyzer",
        options,
        Box::new(|cc| Box::new(ui::App::new(cc))),
    )
}
