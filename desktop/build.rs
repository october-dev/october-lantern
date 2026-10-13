//! Embeds Lantern's icon and version details in the Windows executable, so Explorer, the taskbar,
//! Alt-Tab, Task Manager and the Start menu shortcut show the lantern rather than Windows' generic
//! program icon. The icon is windows/october-lantern.ico (scripts/make-icon-windows.py).
fn main() {
    println!("cargo:rerun-if-changed=../windows/october-lantern.ico");
    #[cfg(windows)]
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut resource = winresource::WindowsResource::new();
        resource
            .set_icon("../windows/october-lantern.ico")
            .set("ProductName", "October Lantern")
            .set("FileDescription", "October Lantern")
            .set("CompanyName", "October")
            .set("OriginalFilename", "october-lantern.exe");
        resource.compile().expect("embedding the Windows icon and version details");
    }
}
