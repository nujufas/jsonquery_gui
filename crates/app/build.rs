use std::path::Path;

/// Embeds the app icon as a Windows PE resource so it shows up on the .exe
/// file itself (Explorer, taskbar pin, "Open With", etc.) -- separate from
/// `.with_icon()` in main.rs, which only sets the *running window's* icon.
fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }

    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").expect("set by cargo");
    let icon = Path::new(&manifest_dir).join("../../assets/icon.ico");

    winresource::WindowsResource::new()
        .set_icon(icon.to_str().expect("icon path should be valid UTF-8"))
        .compile()
        .expect("failed to embed Windows exe icon resource");
}
