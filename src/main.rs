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
    // `kuzgun --inspect <folder>`: print what Kuzgun reads, for checks.
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).map(String::as_str) == Some("--inspect") {
        let picked = std::path::PathBuf::from(args.get(2).cloned().unwrap_or_else(|| ".".into()));
        let root = match model::locate_tracker(&picked) {
            model::Tracker::Local(p) => p,
            other => {
                println!("tracker: {other:?}");
                return;
            }
        };
        let b = model::Board::load(&root);
        println!("tracker: {}", root.display());
        println!("tickets: {}  errors: {}", b.tickets.len(), b.errors.len());
        for (pi, p) in b.projects.iter().enumerate() {
            let n = b.tickets.iter().filter(|t| t.project == pi).count();
            let docs: Vec<&str> = p.docs.iter().map(|d| d.name.as_str()).collect();
            println!("  project {} [{}] {n} tickets, docs {docs:?}, map {}", p.title, p.key, p.map.is_some());
        }
        let mut st: std::collections::BTreeMap<String, usize> = Default::default();
        for t in &b.tickets {
            *st.entry(format!("{} -> {:?}", t.status_key, t.category)).or_default() += 1;
        }
        for (k, v) in st {
            println!("  status {k}: {v}");
        }
        let edges: usize = b.tickets.iter().map(|t| t.blocked_by.len()).sum();
        let refs: usize = b.tickets.iter().map(|t| t.blocked_refs.len()).sum();
        let unresolved: Vec<String> = b
            .tickets
            .iter()
            .flat_map(|t| t.blocked_refs.iter().filter(|r| r.hit.is_none() && r.project.is_none()).map(move |r| format!("{}: {}", t.key, r.text)))
            .collect();
        println!("  blocked-by refs {refs}, resolved edges {edges}, unresolved {}", unresolved.len());
        for u in unresolved.iter().take(8) {
            println!("    ? {u}");
        }
        let facets: Vec<String> = b.facets.iter().map(|f| format!("{}({})", f.key, f.values.len())).collect();
        println!("  facets {facets:?}");
        let (checks, done): (usize, usize) = b.tickets.iter().map(|t| t.checklist_counts()).fold((0, 0), |a, (d, n)| (a.0 + n, a.1 + d));
        println!("  checkboxes {done}/{checks}, untitled {}", b.tickets.iter().filter(|t| t.title.is_empty()).count());
        return;
    }
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
