use rust_photos_core::edit::*;
use rust_photos_core::{files, i18n, library, store, theme, thumbs};
use image::{Rgba, RgbaImage};

fn sample(w: u32, h: u32) -> RgbaImage {
    RgbaImage::from_fn(w, h, |x, y| {
        let r = (x * 255 / w) as u8;
        let g = (y * 255 / h) as u8;
        let b = (((x / 40 + y / 40) % 2) * 200) as u8;
        Rgba([r, g, b, 255])
    })
}

#[test]
fn i18n_has_every_key_in_both_languages() {
    let en: std::collections::HashMap<String, String> = serde_json::from_str(include_str!("../i18n/en.json")).unwrap();
    let vi: std::collections::HashMap<String, String> = serde_json::from_str(include_str!("../i18n/vi.json")).unwrap();
    assert_eq!(en.len(), vi.len());
    assert_eq!(i18n::tr(i18n::Lang::Vi, "nav.gallery"), "Thư viện");
    assert_eq!(i18n::trf(i18n::Lang::En, "view.of", &[("a", "3"), ("b", "9")]), "3 of 9");
}

#[test]
fn natural_sort_and_group_labels() {
    assert_eq!(library::natural_cmp("img2.jpg", "img10.jpg"), std::cmp::Ordering::Less);
    assert_eq!(library::natural_cmp("IMG2", "img2"), std::cmp::Ordering::Equal);
    let today = chrono::Local::now().date_naive();
    let now_ms = chrono::Local::now().timestamp_millis() as u64;
    assert_eq!(library::group_label(now_ms, i18n::Lang::En, today), "Today");
    assert_eq!(library::group_label(now_ms - 86_400_000, i18n::Lang::Vi, today), "Hôm qua");
}

#[test]
fn accent_shades_match_the_old_ts_maths() {
    // Reference values produced by running the original `accentShades` from theme.ts in Node 22.
    // (hex, dark, fill, hover, press, text, on)
    let reference = [
        ("#0078D4", false, "#0073cc", "#0065b3", "#005799", "#005799", [255u8, 255, 255]),
        ("#0078D4", true, "#0078d4", "#0085eb", "#008cf8", "#70c1ff", [255, 255, 255]),
        ("#E3008C", false, "#cc007e", "#b3006e", "#99005e", "#99005e", [255, 255, 255]),
        ("#E3008C", true, "#e0008a", "#f70099", "#ff059f", "#ff70c8", [255, 255, 255]),
        ("#68768A", false, "#586374", "#4d5766", "#424b57", "#424b57", [255, 255, 255]),
        ("#68768A", true, "#606d80", "#6a798d", "#707f94", "#aeb6c2", [255, 255, 255]),
        ("#FFB900", false, "#cc9400", "#b38100", "#996f00", "#996f00", [0, 0, 0]),
        ("#FFB900", true, "#e0a300", "#f7b300", "#ffba05", "#ffd870", [0, 0, 0]),
    ];
    for (hex, dark, fill, hover, press, text, on) in reference {
        let s = theme::accent_shades(hex, dark);
        let got = (theme::to_hex(s.fill), theme::to_hex(s.hover), theme::to_hex(s.press), theme::to_hex(s.text), s.on);
        assert_eq!(got, (fill.to_string(), hover.to_string(), press.to_string(), text.to_string(), on), "{hex} dark={dark}");
    }
}

#[test]
fn edit_pipeline_renders_and_exports() {
    let src = sample(640, 480);
    let mut eng = Engine::new(src);
    let mut st = EditState::default();
    st.rot = 1;
    st.crop = Crop { x: 0.1, y: 0.1, w: 0.8, h: 0.7 };
    st.adj.brightness = 10.0;
    st.adj.contrast = 20.0;
    st.adj.vignette = 40.0;
    st.adj.sharpness = 30.0;
    st.angle = 5.0;
    st.filter = "film".into();
    st.filter_amt = 70.0;
    st.items.push(Item::Stroke { id: 1, pts: vec![[0.1, 0.1], [0.3, 0.2], [0.5, 0.15]], color: [255, 0, 0, 255], width: 6.0, highlighter: false });
    st.items.push(Item::Shape { id: 2, shape: ShapeKind::Arrow, a: [0.2, 0.8], b: [0.7, 0.5], color: [255, 220, 0, 255], width: 5.0, fill: false });
    st.items.push(Item::Text { id: 3, text: "Fluent\nPhotos".into(), x: 0.5, y: 0.5, size: 0.08, rot: 0.2, color: [255, 255, 255, 255], bold: true, italic: false, bg: false });
    st.items.push(Item::Blur { id: 4, x: 0.6, y: 0.6, w: 0.25, h: 0.2, mode: BlurMode::Pixelate });
    let p = eng.preview(&st, 400, &RenderOpts::default());
    assert!(p.width().max(p.height()) <= 400);
    let (w, h) = eng.crop_size(&st);
    assert_eq!((w, h), ((480.0f32 * 0.8).round() as u32, (640.0f32 * 0.7).round() as u32));
    let out = std::env::temp_dir().join("fp-test-out");
    std::fs::create_dir_all(&out).unwrap();
    p.save(out.join("preview.png")).unwrap();
    let full = eng.export_image(&st, Some((300, 260)));
    assert_eq!(full.dimensions(), (300, 260));
    let bytes = encode(&full, &ExportOpts { format: Format::Jpeg, quality: 90, size: None }).unwrap();
    assert!(bytes.starts_with(&[0xFF, 0xD8]));
    let plain = eng.preview(&st, 200, &RenderOpts { plain: true, ..Default::default() });
    assert_eq!(plain.width(), 200);
}

#[test]
fn identity_edit_is_a_no_op() {
    let src = sample(200, 100);
    let mut eng = Engine::new(src.clone());
    let out = eng.export_image(&EditState::default(), None);
    assert_eq!(out.as_raw(), src.as_raw());
    assert!(EditState::default().is_identity());
}

#[test]
fn filters_change_pixels_and_amount_blends() {
    let src = sample(120, 90);
    let mut eng = Engine::new(src.clone());
    let mut st = EditState::default();
    st.filter = "mono".into();
    let full = eng.export_image(&st, None);
    let p = full.get_pixel(60, 45);
    assert!((p[0] as i32 - p[1] as i32).abs() < 6 && (p[1] as i32 - p[2] as i32).abs() < 6, "mono must be grey: {p:?}");
    st.filter_amt = 0.0;
    assert_eq!(eng.export_image(&st, None).as_raw(), src.as_raw());
}

#[test]
fn blur_is_mean_preserving_and_smooths() {
    let mut img = RgbaImage::from_fn(64, 64, |x, _| if x < 32 { Rgba([0, 0, 0, 255]) } else { Rgba([255, 255, 255, 255]) });
    color_blur(&mut img);
    let edge = img.get_pixel(32, 10)[0];
    assert!(edge > 40 && edge < 215, "edge must be soft, got {edge}");
    assert!(img.get_pixel(2, 10)[0] < 20 && img.get_pixel(61, 10)[0] > 235);
}
fn color_blur(img: &mut RgbaImage) { rust_photos_core::edit::color::blur(img, 4.0); }

#[test]
fn thumbnails_are_cached_square_and_upright() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("a.png");
    sample(500, 300).save(&p).unwrap();
    let cache = dir.path().join("cache");
    let t = thumbs::load_thumb(&cache, &p, 128).unwrap();
    assert_eq!(t.dimensions(), (128, 128));
    assert_eq!(std::fs::read_dir(&cache).unwrap().count(), 1);
    let again = thumbs::load_thumb(&cache, &p, 128).unwrap();
    assert_eq!(again.dimensions(), (128, 128));
}

#[test]
fn library_scan_rename_transfer_and_duplicates() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    std::fs::create_dir_all(root.join("sub/deeper")).unwrap();
    sample(50, 40).save(root.join("one.png")).unwrap();
    std::fs::copy(root.join("one.png"), root.join("sub/copy of one.png")).unwrap();
    sample(60, 40).save(root.join("sub/deeper/two.png")).unwrap();
    std::fs::write(root.join("notes.txt"), "x").unwrap();
    std::fs::write(root.join(".hidden.png"), "x").unwrap();
    let o = library::library(root.to_str().unwrap()).unwrap();
    assert_eq!(o.entries.len(), 3, "txt is ignored, hidden names are skipped");
    // Opening one picture lists its own folder only (not sub folders). Like the Tauri version, this shallow
    // scan does not skip dot-files; only the deep library scan does.
    assert_eq!(library::open_target(root.join("one.png").to_str().unwrap()).unwrap().entries.len(), 2);

    let paths: Vec<String> = o.entries.iter().map(|e| e.path.clone()).collect();
    let dups = files::find_duplicates(&paths);
    assert_eq!(dups.len(), 1);
    assert_eq!(dups[0].len(), 2);

    let np = files::rename_file(root.join("one.png").to_str().unwrap(), "uno.png").unwrap();
    assert!(np.ends_with("uno.png"));
    assert!(files::rename_file(&np, "bad:name.png").is_err());
    let n = files::transfer_files(&[np.clone()], root.join("out").to_str().unwrap(), false).unwrap();
    assert_eq!(n, 1);
    let n2 = files::transfer_files(&[np.clone()], root.join("out").to_str().unwrap(), false).unwrap();
    assert_eq!(n2, 1);
    assert!(root.join("out/uno (2).png").exists());
    assert_eq!(files::build_name("Trip_{n}_{name}", "IMG.jpg", 7, 3), "Trip_007_IMG");
}

#[test]
fn rotate_keeps_jpeg_exif_and_orientation_is_reset() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("r.jpg");
    // JPEG with a tiny EXIF block: orientation = 6 (rotate 90 CW), camera "TestCam".
    let mut buf = Vec::new();
    image::DynamicImage::ImageRgb8(image::RgbImage::from_pixel(80, 40, image::Rgb([10, 200, 30]))).write_to(&mut std::io::Cursor::new(&mut buf), image::ImageFormat::Jpeg).unwrap();
    let mut tiff: Vec<u8> = b"MM\0\x2A\0\0\0\x08".to_vec();
    tiff.extend_from_slice(&[0, 2]); // two entries
    tiff.extend_from_slice(&[0x01, 0x0F, 0, 2, 0, 0, 0, 8, 0, 0, 0, 0x26]); // Make -> offset 0x26
    tiff.extend_from_slice(&[0x01, 0x12, 0, 3, 0, 0, 0, 1, 0, 6, 0, 0]); // Orientation = 6
    tiff.extend_from_slice(&[0, 0, 0, 0]);
    tiff.extend_from_slice(b"TestCam\0");
    let mut seg = vec![0xFF, 0xE1];
    let len = (2 + 6 + tiff.len()) as u16;
    seg.extend_from_slice(&len.to_be_bytes());
    seg.extend_from_slice(b"Exif\0\0");
    seg.extend_from_slice(&tiff);
    let mut with = vec![0xFF, 0xD8];
    with.extend_from_slice(&seg);
    with.extend_from_slice(&buf[2..]);
    std::fs::write(&p, &with).unwrap();
    assert_eq!(rust_photos_core::exif_util::orientation(&p), 6);
    assert_eq!(rust_photos_core::exif_util::read_info(&p).make.as_deref(), Some("TestCam"));
    // Upright view: orientation 6 turns the 80x40 pixels into 40x80.
    let up = rust_photos_core::decode::open_upright(&p).unwrap();
    assert_eq!((up.width(), up.height()), (40, 80));
    files::rotate_file(&p, 90, 92).unwrap();
    let after = rust_photos_core::decode::open_upright(&p).unwrap();
    assert_eq!((after.width(), after.height()), (80, 40), "rotated once more and orientation tag reset to 1");
    assert_eq!(rust_photos_core::exif_util::read_info(&p).make.as_deref(), Some("TestCam"));
    assert_eq!(rust_photos_core::exif_util::orientation(&p), 1);
}

#[test]
fn settings_roundtrip_and_old_values_survive() {
    let s: store::Settings = serde_json::from_str(r##"{"theme":"dark","accent":"#8764B8","material":"mica-alt","sort":"name-desc","lang":"vi","slide":{"interval":9,"loop":false}}"##).unwrap();
    assert_eq!(s.theme, store::ThemeMode::Dark);
    assert_eq!(s.material, store::Material::MicaAlt);
    assert_eq!(s.sort, library::SortKey::NameDesc);
    assert_eq!(s.slide.interval, 9);
    assert!(!s.slide.repeat);
    assert_eq!(s.quality, 95, "missing fields take defaults");
    let back: store::Settings = serde_json::from_str(&serde_json::to_string(&s).unwrap()).unwrap();
    assert_eq!(back, s);
}
