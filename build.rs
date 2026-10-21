//! Turns `brand/jpdfw logo.png` into the exe's icon (shown by Explorer, the taskbar and on
//! associated PDF files) plus a raw RGBA window icon, so startup does no image decoding.

use image::codecs::ico::{IcoEncoder, IcoFrame};
use image::imageops::{self, FilterType};
use image::{ExtendedColorType, RgbaImage};
use std::path::PathBuf;

const LOGO: &str = "brand/jpdfw logo.png";

fn main() {
    println!("cargo:rerun-if-changed={LOGO}");
    let out = PathBuf::from(std::env::var("OUT_DIR").unwrap());

    // The logo isn't square; center it on a transparent square so it isn't stretched.
    let logo = image::open(LOGO).expect("brand logo").to_rgba8();
    let side = logo.width().max(logo.height());
    let mut square = RgbaImage::new(side, side);
    let x = (side - logo.width()) / 2;
    let y = (side - logo.height()) / 2;
    imageops::overlay(&mut square, &logo, x.into(), y.into());

    let window_icon = imageops::resize(&square, 256, 256, FilterType::Lanczos3);
    std::fs::write(out.join("icon_256.rgba"), window_icon.as_raw()).unwrap();

    let frames: Vec<IcoFrame> = [16, 20, 24, 32, 40, 48, 64, 96, 128, 256]
        .into_iter()
        .map(|size| {
            let img = imageops::resize(&square, size, size, FilterType::Lanczos3);
            IcoFrame::as_png(img.as_raw(), size, size, ExtendedColorType::Rgba8).unwrap()
        })
        .collect();
    let ico_path = out.join("app.ico");
    IcoEncoder::new(std::fs::File::create(&ico_path).unwrap())
        .encode_images(&frames)
        .unwrap();

    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        winresource::WindowsResource::new()
            .set_icon(ico_path.to_str().unwrap())
            .set("FileDescription", "Just a PDF viewer")
            .set("ProductName", "justafuckingpdfviewer")
            .compile()
            .expect("embedding the icon needs rc.exe from the Windows SDK");
    }
}
