fn main() {
    slint_build::compile("ui/main.slint").unwrap();

    #[cfg(windows)]
    {
        let mut res = winres::WindowsResource::new();
        res.set_icon("../assets/logo.ico");
        res.set("FileDescription", "Simply Transfer");
        res.set("ProductName", "Simply Transfer");
        res.set("LegalCopyright", "GPL-3.0");
        if let Err(e) = res.compile() {
            eprintln!("cargo:warning=Failed to compile Windows resource: {e}");
        }
    }
}
