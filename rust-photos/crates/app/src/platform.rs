//! Window backdrop (Mica / Mica Alt / Acrylic) through DWM. Only Windows 11 draws Mica; elsewhere the UI uses solid colours.
use crate::AppWindow;
use rust_photos_core::store::Material;

#[cfg(windows)]
pub fn apply_material(ui: &AppWindow, m: Material, dark: bool) -> bool {
    use slint::winit_030::WinitWindowAccessor;
    use slint::ComponentHandle;
    ui.window()
        .with_winit_window(|w| {
            // Clear whatever was active before so switching materials never stacks effects.
            let _ = window_vibrancy::clear_mica(w);
            let _ = window_vibrancy::clear_tabbed(w);
            let _ = window_vibrancy::clear_acrylic(w);
            match m {
                Material::None => true,
                Material::Mica => window_vibrancy::apply_mica(w, Some(dark)).is_ok(),
                Material::MicaAlt => window_vibrancy::apply_tabbed(w, Some(dark)).is_ok(),
                Material::Acrylic => {
                    let tint = if dark { (32, 32, 32, 70) } else { (243, 243, 243, 70) };
                    window_vibrancy::apply_acrylic(w, Some(tint)).is_ok()
                }
            }
        })
        .unwrap_or(false)
}

#[cfg(not(windows))]
pub fn apply_material(_ui: &AppWindow, m: Material, _dark: bool) -> bool {
    m == Material::None
}

/// The accent colour chosen in Windows (Settings > Personalization > Colors). `None` elsewhere or when it cannot be read.
#[cfg(windows)]
pub fn system_accent() -> Option<[u8; 3]> {
    use winreg::{enums::HKEY_CURRENT_USER, RegKey};
    let key = RegKey::predef(HKEY_CURRENT_USER).open_subkey(r"Software\Microsoft\Windows\CurrentVersion\Explorer\Accent").ok()?;
    let v: u32 = key.get_value("AccentColorMenu").ok()?;
    Some(rust_photos_core::theme::rgb_from_abgr(v))
}

#[cfg(not(windows))]
pub fn system_accent() -> Option<[u8; 3]> {
    None
}
