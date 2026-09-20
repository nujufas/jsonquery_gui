//! The Android shell of jsonquery.
//!
//! `libjsonquery_android.so` is loaded by `MainActivity` (a `NativeActivity`),
//! which calls [`android_main`]. Everything the OS has to do for the app —
//! the document picker, the clipboard, the on-screen keyboard, "Open with" —
//! is Kotlin's job (`android/app/src/main/kotlin`); this crate is the seam:
//!
//! * [`jvm`] calls *into* Kotlin,
//! * [`native`] receives calls *from* Kotlin,
//! * [`shared`] is the state the two sides hand each other,
//! * [`platform`] implements `jsonquery_gui::platform::Platform` on top.

mod jvm;
mod native;
mod platform;
mod shared;

use android_activity::AndroidApp;

/// Entry point, called on the app's own native thread by `android-activity`.
#[no_mangle]
fn android_main(app: AndroidApp) {
    android_logger::init_once(
        android_logger::Config::default()
            .with_tag("jsonquery")
            .with_max_level(log::LevelFilter::Info),
    );
    install_panic_hook();
    log::info!("jsonquery {} starting", env!("CARGO_PKG_VERSION"));
    point_temp_files_at_the_app_cache(&app);

    match platform::AndroidPlatform::new(&app) {
        Some(platform) => jsonquery_gui::platform::install(Box::new(platform)),
        None => log::error!("no JNI bridge to the activity; file pickers will not work"),
    }

    let options = eframe::NativeOptions {
        android_app: Some(app),
        ..Default::default()
    };
    let result = eframe::run_native(
        "jsonquery",
        options,
        Box::new(|cc| Ok(Box::new(jsonquery_gui::App::new(cc)))),
    );
    if let Err(error) = result {
        log::error!("eframe exited with an error: {error}");
    }
}

/// Send panics to logcat, where `adb logcat` and Play's crash reports will
/// find them, before the default handler aborts the thread.
fn install_panic_hook() {
    let default = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        log::error!("panic: {info}");
        default(info);
    }));
}

/// Downloads (Open URL) go to `std::env::temp_dir()`, which on Android is
/// `/data/local/tmp` unless `TMPDIR` says otherwise — not writable by an app.
fn point_temp_files_at_the_app_cache(app: &AndroidApp) {
    if let Some(cache) = app
        .internal_data_path()
        .and_then(|files| files.parent().map(|app_dir| app_dir.join("cache")))
    {
        std::env::set_var("TMPDIR", cache);
    }
}
