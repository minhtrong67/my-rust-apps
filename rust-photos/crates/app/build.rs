fn main() {
    let cfg = slint_build::CompilerConfiguration::new().with_style("fluent".into());
    slint_build::compile_with_config("ui/app.slint", cfg).expect("slint compile");
    #[cfg(windows)]
    {
        let mut res = winres::WindowsResource::new();
        res.set_icon("assets/icon.ico");
        res.set("ProductName", "Rust Photos");
        res.set("FileDescription", "Rust Photos");
        res.set("CompanyName", "minhtrong67");
        let _ = res.compile();
    }
}
