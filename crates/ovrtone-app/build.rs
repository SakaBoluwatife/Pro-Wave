//! Windows only: embeds the Ovrtone icon and product name in `ovrtone.exe`, so Explorer,
//! shortcuts and taskbar pins show the logo. Other targets set their icons at run time
//! (`ViewportBuilder::with_icon`) or through their package files (dist/linux, dist/macos).
fn main() {
    println!("cargo:rerun-if-changed=assets/ovrtone.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let mut res = winresource::WindowsResource::new();
    res.set_icon("assets/ovrtone.ico")
        .set("ProductName", "Ovrtone")
        .set("FileDescription", "Ovrtone")
        .set("CompanyName", "Bankz Sound Lab");
    // An icon is not worth failing a build over (e.g. no Windows SDK resource compiler).
    if let Err(e) = res.compile() {
        println!("cargo:warning=could not embed the Windows icon: {e}");
    }
}
