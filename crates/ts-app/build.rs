// Puts the ThreadSmith icon into threadsmith.exe (Windows only).
fn main() {
    println!("cargo:rerun-if-changed=assets/threadsmith.ico");
    #[cfg(windows)]
    {
        if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
            let mut res = winresource::WindowsResource::new();
            res.set_icon("assets/threadsmith.ico");
            res.set("ProductName", "ThreadSmith");
            res.set("FileDescription", "ThreadSmith");
            if let Err(e) = res.compile() {
                // The app still works without an .exe icon.
                println!("cargo:warning=Couldn't add the icon to threadsmith.exe: {e}");
            }
        }
    }
}
