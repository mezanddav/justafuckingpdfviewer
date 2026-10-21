//! Times loading a PDF and rendering its first pages: `cargo run --release --example bench -- file.pdf [scale]`
use hayro::hayro_interpret::InterpreterSettings;
use hayro::hayro_syntax::Pdf;
use hayro::vello_cpu::color::palette::css::WHITE;
use hayro::{PixmapSettings, RenderCache, RenderSettings, render};
use std::time::Instant;

fn main() {
    let path = std::env::args().nth(1).expect("pdf path");
    let scale: f32 = std::env::args()
        .nth(2)
        .and_then(|s| s.parse().ok())
        .unwrap_or(2.0);
    let t = Instant::now();
    let data = std::fs::read(&path).unwrap();
    let read = t.elapsed();
    let pdf = Pdf::new(data).unwrap();
    let parsed = t.elapsed();
    let pages = pdf.pages();
    println!("read {read:?}, parsed {parsed:?}, {} pages", pages.len());
    let cache = RenderCache::new();
    let settings = InterpreterSettings::default();
    for (i, page) in pages.iter().take(3).enumerate() {
        let t = Instant::now();
        let pix = render(
            page,
            &cache,
            &settings,
            &RenderSettings::default(),
            &PixmapSettings {
                x_scale: scale,
                y_scale: scale,
                bg_color: WHITE,
            },
        );
        println!(
            "page {i}: {}x{} in {:?}",
            pix.width(),
            pix.height(),
            t.elapsed()
        );
    }
}
