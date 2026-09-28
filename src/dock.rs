//! Dock icon for the bare dev binary (`cargo run` isn't an .app bundle, so
//! macOS would show a generic icon). A bundled Kuzgun.app keeps its compiled
//! Icon Composer icon instead.

#[cfg(target_os = "macos")]
pub fn set_icon() {
    use objc2::AnyThread as _;
    use objc2::MainThreadMarker;
    use objc2_app_kit::{NSApplication, NSImage};
    use objc2_foundation::NSData;

    let in_bundle = std::env::current_exe()
        .is_ok_and(|p| p.to_string_lossy().contains(".app/Contents/MacOS/"));
    if in_bundle {
        return;
    }
    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    let data = NSData::with_bytes(include_bytes!("../assets/icon/kuzgun-1024.png"));
    let Some(image) = NSImage::initWithData(NSImage::alloc(), &data) else {
        return;
    };
    let app = NSApplication::sharedApplication(mtm);
    unsafe { app.setApplicationIconImage(Some(&image)) };
}

#[cfg(not(target_os = "macos"))]
pub fn set_icon() {}

/// The bold app-menu title comes from the process name; the dev binary is
/// `kuzgun`, the bundle says "Kuzgun" (CFBundleName). Call before the app runs.
#[cfg(target_os = "macos")]
pub fn set_process_name() {
    use objc2_foundation::{NSProcessInfo, NSString};
    NSProcessInfo::processInfo().setProcessName(&NSString::from_str("Kuzgun"));
}

#[cfg(not(target_os = "macos"))]
pub fn set_process_name() {}

/// Opts out of App Nap: a board in the background still follows its
/// files live. The token lives for the whole process.
#[cfg(target_os = "macos")]
pub fn keep_live() {
    use objc2_foundation::{NSActivityOptions, NSProcessInfo, NSString};
    let token = NSProcessInfo::processInfo().beginActivityWithOptions_reason(
        NSActivityOptions::UserInitiatedAllowingIdleSystemSleep,
        &NSString::from_str("Following board files live"),
    );
    std::mem::forget(token);
}

#[cfg(not(target_os = "macos"))]
pub fn keep_live() {}
