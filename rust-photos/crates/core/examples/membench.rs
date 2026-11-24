//! Thumbnail pipeline benchmark: peak memory and speed over a folder of 12 MP JPEGs.
//!   membench gen <dir>            create the test pictures
//!   membench run <dir> <threads>  generate thumbnails with that many workers
use rust_photos_core::thumbs;
use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

fn status(key: &str) -> f64 {
    std::fs::read_to_string("/proc/self/status").ok().and_then(|s| s.lines().find(|l| l.starts_with(key)).and_then(|l| l.split_whitespace().nth(1).and_then(|v| v.parse::<f64>().ok()))).unwrap_or(0.0) / 1024.0
}

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let dir = PathBuf::from(&a[2]);
    if a[1] == "gen" {
        std::fs::create_dir_all(&dir).unwrap();
        for n in 0..24u32 {
            let mut seed = 12345u32.wrapping_mul(n + 1);
            let img = image::RgbImage::from_fn(4000, 3000, |x, y| {
                seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
                let noise = ((seed >> 24) as i32 - 128) / 10;
                let v = |f: f32, p: f32| ((((x as f32 * f + y as f32 * p + n as f32 * 40.0) / 90.0).sin() * 0.5 + 0.5) * 220.0) as i32;
                image::Rgb([(v(0.9, 0.3) + noise).clamp(0, 255) as u8, (v(0.2, 0.8) + noise).clamp(0, 255) as u8, (v(0.5, 0.5) + noise).clamp(0, 255) as u8])
            });
            let mut f = std::fs::File::create(dir.join(format!("img_{n:02}.jpg"))).unwrap();
            image::codecs::jpeg::JpegEncoder::new_with_quality(&mut f, 90).encode_image(&img).unwrap();
        }
        println!("generated 24 pictures, {:.1} MB total", std::fs::read_dir(&dir).unwrap().map(|e| e.unwrap().metadata().unwrap().len()).sum::<u64>() as f64 / 1e6);
        return;
    }
    if a[1] == "view-old" || a[1] == "view-new" {
        // one 12 MP picture opened for the viewer; `keep` stands in for the pixel buffer Slint holds on to
        let p = dir.join("img_00.jpg");
        let keep: Vec<u8> = if a[1] == "view-old" {
            let img = rust_photos_core::decode::open_for_view(&p, 6144).unwrap(); // RGB -> RGBA copy
            img.as_raw().clone()                                                   // clone into the UI buffer
        } else {
            let img = rust_photos_core::decode::open_upright(&p).unwrap().into_rgb8();
            let mut out = vec![0u8; img.width() as usize * img.height() as usize * 4];
            for (d, s) in out.chunks_exact_mut(4).zip(img.pixels()) { d[..3].copy_from_slice(&s.0); d[3] = 255; }
            out
        };
        println!("{}: peak memory {:.0} MB, kept afterwards {:.0} MB (buffer {:.0} MB)", a[1], status("VmHWM"), status("VmRSS"), keep.len() as f64 / 1048576.0);
        return;
    }
    let threads: usize = a[3].parse().unwrap();
    let mut files: Vec<PathBuf> = std::fs::read_dir(&dir).unwrap().map(|e| e.unwrap().path()).filter(|p| p.extension().map(|e| e == "jpg").unwrap_or(false)).collect();
    files.sort();
    let cache = std::env::temp_dir().join("membench-cache");
    let _ = std::fs::remove_dir_all(&cache);
    let base = status("VmRSS");
    let q = Arc::new(Mutex::new(files.iter().cloned().collect::<VecDeque<_>>()));
    let t = std::time::Instant::now();
    let hs: Vec<_> = (0..threads).map(|_| { let q = q.clone(); let c = cache.clone(); std::thread::spawn(move || loop { let Some(p) = q.lock().unwrap().pop_front() else { break }; thumbs::load_thumb(&c, &p, thumbs::THUMB_SIZE).unwrap(); }) }).collect();
    for h in hs { h.join().unwrap(); }
    let s = t.elapsed().as_secs_f64();
    println!("{} pictures, {} workers: {:.1}s ({:.1} pictures/s) | peak memory {:.0} MB (baseline {:.0} MB) | memory after {:.0} MB", files.len(), threads, s, files.len() as f64 / s, status("VmHWM"), base, status("VmRSS"));
}
