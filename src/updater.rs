//! In-app updates through Sparkle (macOS).
//!
//! Kuzgun.app embeds Sparkle.framework (scripts/bundle.sh) and its Info.plist
//! points at the appcast published with each GitHub release. Sparkle checks
//! once a day and downloads new versions in the background; once one is
//! ready it hands us an "install now" block (`willInstallUpdateOnQuit`),
//! which the status bar's "Restart to Update" button calls. Ignored, the
//! update installs when Kuzgun quits. "Check for Updates…" (menu / palette)
//! runs Sparkle's own interactive check with its standard windows.
//!
//! The bare `cargo run` binary has no bundle, so updates are off there.
//! `KUZGUN_UPDATE_FEED=<url>` overrides the feed (local release testing);
//! `KUZGUN_FAKE_UPDATE=<version>` shows the ready state without Sparkle.

use gpui_kit::*;

/// What the UI needs: whether updates work at all, and a ready update.
#[derive(Default)]
pub struct Updater {
    pub enabled: bool,
    /// Version of a downloaded update waiting for a restart.
    pub ready: Option<String>,
}
impl Global for Updater {}

pub fn init(cx: &mut App) {
    cx.set_global(Updater::default());
    // `KUZGUN_FAKE_UPDATE=0.2.0`: pretend an update is ready (UI checks).
    if let Ok(v) = std::env::var("KUZGUN_FAKE_UPDATE") {
        cx.set_global(Updater {
            enabled: true,
            ready: Some(v),
        });
        return;
    }
    #[cfg(target_os = "macos")]
    if let Some(mut rx) = mac::start() {
        cx.global_mut::<Updater>().enabled = true;
        AVAILABLE.store(true, std::sync::atomic::Ordering::Relaxed);
        cx.spawn(async move |cx: &mut AsyncApp| {
            use futures::StreamExt as _;
            while let Some(version) = rx.next().await {
                cx.update(|cx| {
                    cx.global_mut::<Updater>().ready = Some(version);
                    crate::menus::refresh(cx);
                    cx.refresh_windows();
                });
            }
        })
        .detach();
    }
}

/// Updates work in this run (for code without an `App` at hand).
static AVAILABLE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

pub fn available() -> bool {
    AVAILABLE.load(std::sync::atomic::Ordering::Relaxed)
}

pub fn enabled(cx: &App) -> bool {
    cx.try_global::<Updater>().is_some_and(|u| u.enabled)
}

pub fn ready(cx: &App) -> Option<String> {
    cx.try_global::<Updater>().and_then(|u| u.ready.clone())
}

/// Sparkle's interactive check (shows "up to date" / the update window).
pub fn check_for_updates() {
    #[cfg(target_os = "macos")]
    mac::check();
}

/// Install the downloaded update now and relaunch.
pub fn restart_to_update() {
    #[cfg(target_os = "macos")]
    mac::install_now();
}

#[cfg(target_os = "macos")]
mod mac {
    use std::cell::RefCell;
    use std::sync::{Mutex, OnceLock};

    use block2::{Block, RcBlock};
    use futures::channel::mpsc;
    use objc2::rc::{Allocated, Retained};
    use objc2::runtime::{AnyClass, AnyObject, Bool, NSObject, NSObjectProtocol};
    use objc2::{AnyThread, class, define_class, msg_send};
    use objc2_foundation::NSString;

    static READY: OnceLock<Mutex<mpsc::UnboundedSender<String>>> = OnceLock::new();

    thread_local! {
        // Sparkle holds its delegate weakly; both live for the whole run.
        static CONTROLLER: RefCell<Option<Retained<AnyObject>>> = const { RefCell::new(None) };
        static DELEGATE: RefCell<Option<Retained<Delegate>>> = const { RefCell::new(None) };
        static INSTALL_NOW: RefCell<Option<RcBlock<dyn Fn()>>> = const { RefCell::new(None) };
    }

    define_class!(
        // SAFETY: NSObject has no subclassing requirements; no Drop impl.
        #[unsafe(super(NSObject))]
        #[name = "KuzgunUpdaterDelegate"]
        struct Delegate;

        impl Delegate {
            /// An update finished downloading and will install on quit.
            /// Returning YES keeps Sparkle from prompting: we show our own
            /// button and call the block from it.
            #[unsafe(method(updater:willInstallUpdateOnQuit:immediateInstallationBlock:))]
            fn will_install_on_quit(&self, _updater: &AnyObject, item: &AnyObject, install: &Block<dyn Fn()>) -> Bool {
                let version: Option<Retained<NSString>> = unsafe { msg_send![item, displayVersionString] };
                INSTALL_NOW.with(|b| *b.borrow_mut() = Some(install.copy()));
                if let Some(tx) = READY.get() {
                    let _ = tx.lock().unwrap().unbounded_send(version.map(|v| v.to_string()).unwrap_or_default());
                }
                Bool::YES
            }

            #[unsafe(method_id(feedURLStringForUpdater:))]
            fn feed_url(&self, _updater: &AnyObject) -> Option<Retained<NSString>> {
                std::env::var("KUZGUN_UPDATE_FEED").ok().map(|s| NSString::from_str(&s))
            }
        }

        unsafe impl NSObjectProtocol for Delegate {}
    );

    /// Load the embedded framework and start the updater. None when not
    /// running from a bundle that ships Sparkle (e.g. `cargo run`).
    pub fn start() -> Option<mpsc::UnboundedReceiver<String>> {
        let exe = std::env::current_exe().ok()?;
        let framework = exe.parent()?.parent()?.join("Frameworks/Sparkle.framework");
        if !framework.exists() {
            return None;
        }
        let path = NSString::from_str(framework.to_str()?);
        let bundle: Option<Retained<AnyObject>> =
            unsafe { msg_send![class!(NSBundle), bundleWithPath: &*path] };
        let loaded: Bool = unsafe { msg_send![&*bundle?, load] };
        if !loaded.as_bool() {
            return None;
        }
        let cls = AnyClass::get(c"SPUStandardUpdaterController")?;

        let (tx, rx) = mpsc::unbounded();
        READY.set(Mutex::new(tx)).ok()?;
        let delegate: Retained<Delegate> = unsafe { msg_send![Delegate::alloc(), init] };
        let controller: Option<Retained<AnyObject>> = unsafe {
            let alloc: Allocated<AnyObject> = msg_send![cls, alloc];
            msg_send![
                alloc,
                initWithStartingUpdater: Bool::YES,
                updaterDelegate: &*delegate,
                userDriverDelegate: std::ptr::null::<AnyObject>()
            ]
        };
        let controller = controller?;
        // Local release testing: look for the update right away.
        if std::env::var_os("KUZGUN_UPDATE_FEED").is_some() {
            let updater: Retained<AnyObject> = unsafe { msg_send![&*controller, updater] };
            let _: () = unsafe { msg_send![&*updater, checkForUpdatesInBackground] };
        }
        CONTROLLER.with(|c| *c.borrow_mut() = Some(controller));
        DELEGATE.with(|d| *d.borrow_mut() = Some(delegate));
        Some(rx)
    }

    pub fn check() {
        CONTROLLER.with(|c| {
            if let Some(controller) = c.borrow().as_ref() {
                let _: () = unsafe {
                    msg_send![&**controller, checkForUpdates: std::ptr::null::<AnyObject>()]
                };
            }
        });
    }

    pub fn install_now() {
        if let Some(block) = INSTALL_NOW.with(|b| b.borrow_mut().take()) {
            block.call(());
        }
    }
}
