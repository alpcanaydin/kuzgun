//! Command palette (⇧⌘P / ⌘K), ticket quick-open (⌘P), board switcher and
//! the status picker (s), all on the kit's `Command` component.
//!
//! Rows either carry an `Action` (the kit shows its live keybinding) or a
//! closure; `on_confirm` maps the confirmed row back to its `RunFn` through
//! the [`KuzgunHandle`] global, because the callback only receives `&mut App`.

use std::rc::Rc;

use gpui_kit::assets::IconName;
use gpui_kit::component::Icon;
use gpui_kit::component::command::{Command, CommandItem, CommandState};
use gpui_kit::component::theme::ActiveTheme as _;
use gpui_kit::*;

use crate::actions::*;
use crate::app::{KuzgunApp, KuzgunHandle, Screen};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PaletteMode {
    Commands,
    Tickets,
    Boards,
}

pub type RunFn = Rc<dyn Fn(&mut KuzgunApp, &mut Window, &mut Context<KuzgunApp>)>;

pub struct PaletteOverlay {
    pub mode: PaletteMode,
    pub state: Entity<CommandState>,
    pub items: Vec<CommandItem>,
}

impl PaletteOverlay {
    /// `items` are the rows from [`rows`], built by the caller while it
    /// holds the app (reading the entity here would re-enter its update).
    pub fn open(mode: PaletteMode, items: Vec<CommandItem>, window: &mut Window, cx: &mut Context<KuzgunApp>) -> Self {
        let state = cx.new(|cx| CommandState::new(window, cx));
        let handle = state.read(cx).focus_handle(cx);
        handle.focus(window, cx);
        Self { mode, state, items }
    }
}

fn run(f: impl Fn(&mut KuzgunApp, &mut Window, &mut Context<KuzgunApp>) + 'static) -> RunFn {
    Rc::new(f)
}

/// Rows and their runs, in the same order.
pub fn rows(app: &KuzgunApp, mode: PaletteMode) -> (Vec<CommandItem>, Vec<RunFn>) {
    match mode {
        PaletteMode::Commands => command_rows(app),
        PaletteMode::Tickets => ticket_rows(app),
        PaletteMode::Boards => board_rows(app),
    }
}

fn command_rows(app: &KuzgunApp) -> (Vec<CommandItem>, Vec<RunFn>) {
    let mut items = Vec::new();
    let mut runs: Vec<RunFn> = Vec::new();
    let noop = run(|_, _, _| {});
    let mut action = |label: &str, icon: IconName, a: Box<dyn Action>, keywords: &[&str]| {
        items.push(
            CommandItem::new()
                .label(label.to_string())
                .keywords(keywords.iter().map(|k| k.to_string()))
                .icon(Icon::new(icon))
                .action(a),
        );
        runs.push(noop.clone());
    };
    action("board: open folder…", IconName::FolderOpen, Box::new(OpenFolder), &["board", "open", "add"]);
    if app.screen == Screen::Board {
        action("ticket: go to…", IconName::Search, Box::new(QuickOpenTicket), &["find", "jump"]);
        action("board: reload", IconName::RefreshCw, Box::new(Reload), &["refresh"]);
        action("board: close", IconName::X, Box::new(CloseBoard), &["welcome"]);
        action("view: toggle sidebar", IconName::PanelLeft, Box::new(ToggleSidebar), &[]);
        action("view: swimlanes by project", IconName::Rows3, Box::new(ToggleSwimlanes), &["group", "lanes"]);
        action("view: hide done & canceled", IconName::EyeOff, Box::new(ToggleHideClosed), &["closed", "completed"]);
        action("view: show empty columns", IconName::Columns3, Box::new(ToggleEmptyColumns), &[]);
        action("filter: search…", IconName::Search, Box::new(FocusSearch), &["find", "query"]);
        action("filter: all tickets", IconName::Layers, Box::new(FilterAll), &["clear"]);
        action("filter: frontier", IconName::Compass, Box::new(FilterFrontier), &["ready", "unblocked", "wayfinder"]);
        action("filter: agent-ready (AFK)", IconName::Bot, Box::new(FilterAgent), &["afk", "agent"]);
        action("filter: blocked", IconName::Lock, Box::new(FilterBlocked), &[]);
        action("filter: needs a human (HITL)", IconName::Hand, Box::new(FilterHuman), &["manual", "hitl"]);
        if app.current_path().is_some() {
            action("ticket: copy /implement command", IconName::Terminal, Box::new(CopyImplement), &["agent", "implement"]);
            action("ticket: copy /triage command", IconName::Terminal, Box::new(CopyTriage), &["agent", "triage"]);
            action("ticket: copy /wayfinder command", IconName::Terminal, Box::new(CopyWayfinder), &["agent", "wayfinder", "map"]);
            action("ticket: open in editor", IconName::SquarePen, Box::new(OpenInEditor), &["edit", "zed", "code"]);
            action("ticket: reveal in finder", IconName::Folder, Box::new(RevealInFinder), &[]);
            action("ticket: copy ID", IconName::Hash, Box::new(CopyId), &["key"]);
            action("ticket: copy title", IconName::Copy, Box::new(CopyTitle), &[]);
            action("ticket: copy path", IconName::Link, Box::new(CopyPath), &["file"]);
        }
    }

    // Dynamic rows: projects, recent boards, themes.
    let mut dynamic: Vec<(String, IconName, RunFn)> = Vec::new();
    if app.screen == Screen::Board {
        dynamic.push((
            "project: all projects".into(),
            IconName::Layers,
            run(|a, _, cx| {
                a.view.project = None;
                a.save_view();
                cx.notify();
            }),
        ));
        for p in &app.board.projects {
            let name = p.name.clone();
            dynamic.push((
                format!("project: {}", p.title),
                IconName::FolderKanban,
                run(move |a, _, cx| {
                    a.view.project = Some(name.clone());
                    a.save_view();
                    cx.notify();
                }),
            ));
            for d in &p.docs {
                let path = d.path.clone();
                dynamic.push((
                    format!("doc: {} · {}", p.title, d.name),
                    IconName::BookOpen,
                    run(move |a, w, cx| a.open_doc(path.clone(), w, cx)),
                ));
            }
        }
    }
    for b in app.saved.iter().take(9) {
        let path = b.path.clone();
        dynamic.push((
            format!("board: open {}", b.name),
            IconName::SquareKanban,
            run(move |a, w, cx| a.open_board(path.clone(), w, cx)),
        ));
    }
    let prefs = crate::settings::get();
    for light in [false, true] {
        for name in crate::themes::names(light) {
            let current = if light { &prefs.light_theme } else { &prefs.dark_theme };
            if name == current {
                continue;
            }
            let mode = if light { "light" } else { "dark" };
            dynamic.push((
                format!("theme: {name} ({mode})"),
                IconName::Palette,
                run(move |_, _, cx| crate::settings::choose_theme(cx, name)),
            ));
        }
    }
    for mode in crate::settings::Appearance::ALL {
        if mode == prefs.appearance {
            continue;
        }
        dynamic.push((
            format!("theme: appearance {}", mode.label().to_lowercase()),
            if mode == crate::settings::Appearance::Light { IconName::Sun } else { IconName::Moon },
            run(move |_, _, cx| crate::settings::update(cx, |p| p.appearance = mode)),
        ));
    }
    for (label, icon, r) in dynamic {
        items.push(CommandItem::new().label(label).icon(Icon::new(icon)));
        runs.push(r);
    }
    let mut action = |label: &str, icon: IconName, a: Box<dyn Action>| {
        items.push(CommandItem::new().label(label.to_string()).icon(Icon::new(icon)).action(a));
        runs.push(run(|_, _, _| {}));
    };
    action("app: settings", IconName::Settings, Box::new(OpenSettings));
    action("app: hide", IconName::EyeOff, Box::new(HideApp));
    action("app: quit", IconName::Power, Box::new(Quit));
    (items, runs)
}

fn ticket_rows(app: &KuzgunApp) -> (Vec<CommandItem>, Vec<RunFn>) {
    let mut items = Vec::new();
    let mut runs: Vec<RunFn> = Vec::new();
    for t in &app.board.tickets {
        let p = &app.board.projects[t.project];
        items.push(
            CommandItem::new()
                .label(format!("{}  {}", t.key, t.title))
                .keywords([t.status.clone(), p.name.clone(), t.slug.clone()])
                .icon(crate::icons::status_icon(t.category)),
        );
        let path = t.path.clone();
        runs.push(run(move |a, w, cx| {
            let full = a.detail.as_ref().is_some_and(|d| d.full);
            a.open_detail(path.clone(), full, w, cx);
        }));
    }
    (items, runs)
}

fn board_rows(app: &KuzgunApp) -> (Vec<CommandItem>, Vec<RunFn>) {
    let mut items = Vec::new();
    let mut runs: Vec<RunFn> = Vec::new();
    for b in &app.saved {
        items.push(
            CommandItem::new()
                .label(format!("{}  {}", b.name, crate::store::tilde(&b.path)))
                .icon(Icon::new(IconName::SquareKanban)),
        );
        let path = b.path.clone();
        runs.push(run(move |a, w, cx| a.open_board(path.clone(), w, cx)));
    }
    (items, runs)
}

impl KuzgunApp {
    pub fn render_palette(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let overlay = self.palette.as_ref().expect("palette open");
        let elevated = cx.theme().popover;
        let border = cx.theme().border;
        let placeholder = match overlay.mode {
            PaletteMode::Commands => "Type a command…",
            PaletteMode::Tickets => "Go to ticket…",
            PaletteMode::Boards => "Open a board…",
        };
        div()
            .id("palette-backdrop")
            .absolute()
            .inset_0()
            .pt(px(48.))
            .on_mouse_down(MouseButton::Left, |_, window, cx| {
                let view = cx.global::<KuzgunHandle>().0.clone();
                view.update(cx, |this, cx| this.close_palette(window, cx));
            })
            .flex()
            .flex_row()
            .justify_center()
            .items_start()
            .child(
                div()
                    .id("palette-card")
                    .w(px(600.))
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .rounded(px(8.))
                    .border_1()
                    .border_color(border)
                    .bg(elevated)
                    .shadow_lg()
                    .child(
                        Command::new(&overlay.state)
                            .items(overlay.items.clone())
                            .placeholder(placeholder)
                            .on_confirm(|path, window, cx| {
                                let view = cx.global::<KuzgunHandle>().0.clone();
                                view.update(cx, |this, cx| this.palette_confirm(path.row, window, cx));
                            })
                            .on_cancel(|window, cx| {
                                let view = cx.global::<KuzgunHandle>().0.clone();
                                view.update(cx, |this, cx| this.close_palette(window, cx));
                            }),
                    ),
            )
    }
}
