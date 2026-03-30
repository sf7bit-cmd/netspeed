#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod measure;
mod scan;
mod ui;
mod history;
mod wifi;
mod iperf;

use eframe::{egui, NativeOptions};
use egui::ViewportBuilder;

fn main() -> eframe::Result<()> {
    let options = NativeOptions {
        viewport: ViewportBuilder::default()
            .with_title("NetSpeed Analyzer")
            .with_inner_size([1060.0, 740.0])
            .with_min_inner_size([700.0, 520.0]),
        ..Default::default()
    };
    eframe::run_native(
        "NetSpeed Analyzer",
        options,
        Box::new(|cc| Box::new(ui::App::new(cc))),
    )
}
