fn main() {
    println!("cargo:rerun-if-changed=../../assets/zhell.ico");
    #[cfg(windows)]
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("../../assets/zhell.ico")
            .set("ProductName", "Zhell")
            .set("FileDescription", "Zhell — the terminal that remembers")
            .set("CompanyName", "TheHolyOneZ")
            .set("LegalCopyright", "© TheHolyOneZ · GPL-3.0-or-later")
            .set("Comments", "https://zsync.eu/zhell/");
        if let Err(e) = res.compile() {
            println!("cargo:warning=couldn't embed the Windows icon: {e}");
        }
    }
}
