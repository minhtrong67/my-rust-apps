# Rust Photos 3.0 (pure Rust rewrite)

A photo viewer and small editor in the style of Windows 11 Photos.
This is a rewrite of the Tauri + React version: **no WebView2, no JavaScript, one native executable.**

```
crates/core   pure Rust engine: library, EXIF, thumbnails, editing, settings, i18n   (no UI code)
crates/app    Slint UI (Fluent style) + glue
```

## Build (Windows 10/11)

Needs Rust **1.95 or newer** (Slint 1.19) with the MSVC toolchain.

```
cargo run --release -p rust-photos
cargo test -p rust-photos-core
```

## Honest status

Checked on Linux while writing this (not on Windows):

| What | How it was checked |
|---|---|
| `core` engine: scan, sort, thumbnails, EXIF-preserving rotate, rename/copy/move, duplicates, settings, the whole edit pipeline | 11 automated tests, plus a rendered preview image inspected by eye |
| Accent colour maths | compared with the original `accentShades` from `theme.ts`, run in Node |
| EN/VI strings | converted from the TypeScript dictionaries; both have all 386 keys |
| UI layout (gallery, viewer, settings, editor tabs) | rendered with Slint's software renderer (`cargo run -p rust-photos --example shot`) and viewed |

**Not checked by a compiler or by running the real app:** everything behind `#[cfg(windows)]`
(Mica / Mica Alt / Acrylic via `window-vibrancy`, Recycle Bin restore, wallpaper, print),
file drop, single-instance hand-off, native file dialogs, clipboard, the editor's pointer tools
(pen, shapes, text drag, crop handles) and save flow, slideshow, and real-world speed.
No performance numbers were measured, so "faster than the Tauri build" is a goal, not a result.

## Memory

What was measured (Linux, 24 JPEGs of 12 MP, `cargo run --release -p rust-photos-core --example membench`):

| Thumbnail generation, 8 workers | peak memory | speed |
|---|---|---|
| decode whole picture, then shrink (first version) | 755 MB | 6.9 pictures/s |
| JPEG decoded at 1/2-1/8 size (current) | 62 MB | 12.2 pictures/s |

Opening one 12 MP picture in the viewer: peak 97 MB before, 85 MB now. The picture itself needs 46 MB as RGBA and the viewer keeps up to
3 pictures; they are freed when you go back to the gallery. This was not measured on Windows or for the whole app: the graphics driver,
GPU textures and DLLs are on top of it. To see the split, run with `FP_STATS=1`
(PowerShell: `$env:FP_STATS=1; cargo run --release -p rust-photos`): the title bar shows the live Rust heap and its peak.
Task Manager's number minus the Rust heap is what the driver and system libraries use.
To compare renderers: `set SLINT_BACKEND=winit-software` before running (no GPU, slower, Mica may not show).

## Not ported yet

- Colour emoji stickers (the item type exists; nothing draws it)
- Slideshow transitions (the setting is stored but ignored)
- AVIF pictures
- The 7 bundled web fonts (the UI uses the system font)
- WebP export is lossless only (the Rust encoder has no lossy mode)
- Adjustment presets have storage in `core` but no UI yet
- Favorites / albums / presets of the old app lived in WebView2 localStorage and are not migrated

## Licenses

MIT for this code. The UI uses **Slint**, which is GPLv3, a royalty-free license that requires
attribution (an About box does this), or a commercial license. Check that it fits your plans.
Icons are Lucide (ISC), see `crates/app/ui/icons/LICENSE-lucide.txt`.
