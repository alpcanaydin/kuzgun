use gpui_kit::assets::AllAssets;
use gpui_kit::component::{Root, TitleBar};
use gpui_kit::*;

mod actions;
mod agents;
mod app;
mod board;
mod deps;
mod detail;
mod agents_page;
mod dock;
mod editors;
mod git;
mod home;
mod icons;
mod kbd;
mod menus;
mod model;
mod notify;
mod palette;
mod settings;
mod store;
mod terminals;
mod theme;
mod themes;
mod toast;
mod watch;

/// Handle of the main window, for dock reopen.
struct MainWindow(AnyWindowHandle);
impl Global for MainWindow {}

fn main() {
    if std::env::args().any(|a| a == "--test-notification") {
        let r = notify::send("Kuzgun", "Notifications work.");
        println!("{r:?}");
        // The request completes on a system thread.
        std::thread::sleep(std::time::Duration::from_secs(4));
        return;
    }
    let app = gpui_kit::application().with_assets(AllAssets);
    // Dock click / relaunch while running: bring the main window back, or
    // reopen it on the welcome screen if it was closed.
    app.on_reopen(|cx| {
        if let Some(handle) = cx.try_global::<MainWindow>().map(|m| m.0)
            && cx
                .update_window(handle, |_, window, _| window.activate_window())
                .is_ok()
        {
            return;
        }
        open_main_window(cx, false);
    });
    dock::set_process_name();
    dock::keep_live();
    app.run(move |cx: &mut App| {
        gpui_kit::init(cx);
        actions::bind_keys(cx);
        menus::install(cx);
        if let Err(e) = theme::load_embedded_fonts(cx) {
            eprintln!("font load error: {e:#}");
        }
        theme::apply(cx);
        dock::set_icon();
        if crate::settings::get().notify_enabled {
            notify::request_permission();
        }
        open_main_window(cx, true);
        if !background() {
            cx.activate(true);
        }
    });
}

/// `KUZGUN_BACKGROUND=1`: never activate the app or focus new windows.
pub fn background() -> bool {
    std::env::var_os("KUZGUN_BACKGROUND").is_some()
}

fn open_main_window(cx: &mut App, auto_open: bool) {
    let bounds = Bounds::centered(None, size(px(1360.), px(860.)), cx);
    let result = cx.open_window(
        WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            window_min_size: Some(size(px(900.), px(560.))),
            focus: !background(),
            ..TitleBar::window_options()
        },
        |window, cx| {
            let view = cx.new(|cx| app::KuzgunApp::new(window, cx));
            cx.set_global(app::KuzgunHandle(view.clone()));
            let focus = view.read(cx).focus.clone();
            focus.focus(window, cx);
            window
                .observe_window_appearance(|_, cx| {
                    if settings::get().appearance == settings::Appearance::System {
                        theme::apply(cx);
                        cx.refresh_windows();
                    }
                })
                .detach();
            let close_view = view.clone();
            window.on_window_should_close(cx, move |window, cx| {
                close_view.update(cx, |app, cx| app.on_close_request(window, cx))
            });
            // A path argument opens that folder; otherwise the last board.
            let arg = std::env::args().nth(1).map(std::path::PathBuf::from);
            view.update(cx, |app, cx| match arg {
                Some(p) if p.is_dir() => {
                    let p = std::fs::canonicalize(&p).unwrap_or(p);
                    app.open_board(p, window, cx)
                }
                _ if auto_open => app.auto_open(window, cx),
                _ => {}
            });
            cx.new(|cx| Root::new(view, window, cx))
        },
    );
    match result {
        Ok(handle) => cx.set_global(MainWindow(handle.into())),
        Err(e) => eprintln!("failed to open window: {e}"),
    }
}
