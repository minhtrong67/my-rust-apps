// Embeds the app icon and version info into rust-file.exe (Windows only).
// A failure here is never fatal: the app still builds, it just has no embedded icon.
fn main() {
    #[cfg(windows)]
    {
        let mut res = winres::WindowsResource::new();
        res.set_icon("assets/icon.ico");
        res.set("ProductName", "Rust File");
        res.set("FileDescription", "Rust File - native file manager");
        if let Err(e) = res.compile() {
            println!("cargo:warning=could not embed the Windows icon: {e}");
        }
    }
}
