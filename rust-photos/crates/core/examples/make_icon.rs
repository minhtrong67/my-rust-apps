//! Builds the Windows icon (.ico with 16..256 px frames) and a 512 px PNG from the SVG.
//!   make_icon <icon.svg> <out.ico> <out.png>
use image::codecs::ico::{IcoEncoder, IcoFrame};
use image::ExtendedColorType;
use rust_photos_core::decode::render_svg;

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let svg = std::path::Path::new(&a[1]);
    let mut frames = Vec::new();
    for size in [16u32, 24, 32, 48, 64, 128, 256] {
        let img = render_svg(svg, size).expect("render");
        assert_eq!((img.width(), img.height()), (size, size));
        frames.push(IcoFrame::as_png(img.as_raw(), size, size, ExtendedColorType::Rgba8).expect("frame"));
    }
    IcoEncoder::new(std::fs::File::create(&a[2]).unwrap()).encode_images(&frames).expect("ico");
    render_svg(svg, 512).unwrap().save(&a[3]).unwrap();
    println!("wrote {} and {}", a[2], a[3]);
}
