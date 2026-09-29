//! User preferences + the Settings window (⌘,).
//!
//! Prefs live in `~/Library/Application Support/kuzgun/settings.json` and in a
//! process-wide `RwLock`, so render code reads fonts with plain functions
//! ([`ui_font`], [`mono_font`], …) without threading `cx` through.
//! Every change is saved immediately and applied live to all windows.

use std::sync::{LazyLock, RwLock};

use gpui_kit::assets::IconName;
use gpui_kit::component::searchable_list::SearchableVec;
use gpui_kit::component::select::{Select, SelectEvent, SelectState};
use gpui_kit::component::setting::{
    NumberFieldOptions, SettingField, SettingGroup, SettingItem, SettingPage, Settings,
};
use gpui_kit::component::{ActiveTheme as _, IndexPath, Root, Sizable as _, TitleBar};
use gpui_kit::*;
use serde::{Deserialize, Serialize};

use crate::themes;

/// Which theme slot is active.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Appearance {
    /// Follow macOS light / dark.
    #[default]
    System,
    Light,
    Dark,
}

impl Appearance {
    pub const ALL: [Appearance; 3] = [Appearance::System, Appearance::Light, Appearance::Dark];

    pub fn label(self) -> &'static str {
        match self {
            Appearance::System => "System",
            Appearance::Light => "Light",
            Appearance::Dark => "Dark",
        }
    }

    fn from_label(s: &str) -> Self {
        Self::ALL
            .into_iter()
            .find(|a| a.label() == s)
            .unwrap_or_default()
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Prefs {
    pub appearance: Appearance,
    /// Theme used in light mode / dark mode (chosen separately).
    pub light_theme: String,
    pub dark_theme: String,
    /// Interface font. Its size is the rem base, so it scales the whole UI.
    pub ui_font_family: String,
    pub ui_font_size: f32,
    /// Ticket keys, paths, code and the markdown code blocks.
    pub mono_font_family: String,
    pub mono_font_size: f32,
    /// Accent (primary) color name, see [`ACCENTS`]; "Theme" = the theme's own.
    pub accent: String,
    // ---- general ----
    /// Open the last board when the app starts.
    pub reopen_last: bool,
    /// Command for "Open in Editor" (`zed`, `code`, `cursor`…). Empty = the
    /// system's default app for markdown.
    pub editor_command: String,
    /// Editor app for Open in Editor (a name from `editors::installed`);
    /// empty = the system default for markdown.
    pub editor_app: String,
    /// Terminal app for Start agent; empty = the first one installed.
    pub terminal_app: String,
    // ---- board ----
    pub column_width: f32,
    /// Keep one drop column for each workflow stage even when it is empty.
    pub show_empty_columns: bool,
    /// Blocked unstarted tickets in their own column, left of the ready ones.
    pub split_blocked: bool,
    pub card_id: bool,
    pub card_project: bool,
    /// Short repeating fields (Type, Tranche, Priority, Labels…).
    pub card_fields: bool,
    /// AFK / HITL badge.
    pub card_mode: bool,
    pub card_checklist: bool,
    pub card_relations: bool,
    /// Days-in-column dots, from git history.
    pub card_age: bool,
    pub card_updated: bool,
    /// Card density: "Comfortable" | "Compact".
    pub density: String,
    /// Which default fonts the saved families came from; lets a new
    /// default replace an old untouched one once. Missing in old files = 0.
    #[serde(default)]
    pub font_defaults: u32,
    // ---- notifications ----
    pub notify_enabled: bool,
    /// Only while Kuzgun is not the active app.
    pub notify_background_only: bool,
    pub notify_agent_done: bool,
    pub notify_agent_started: bool,
    pub notify_unblocked: bool,
    pub notify_needs_you: bool,
    pub notify_closed: bool,
}

/// Bump when the default fonts change.
const FONT_DEFAULTS: u32 = 4;

pub const DENSITIES: [&str; 2] = ["Comfortable", "Compact"];

/// Accent choices (dark, light variant).
pub const ACCENTS: &[(&str, u32, u32)] = &[
    ("Violet", 0x9B7CF4, 0x6D4FD8),
    ("Teal", 0x2EB8A6, 0x0F8C7E),
    ("Blue", 0x4A90F0, 0x2F6FE0),
    ("Magenta", 0xD66BD0, 0xA83AA0),
    ("Sky", 0x3FB6E8, 0x0B7CB0),
    ("Theme", 0, 0),
];

pub const UI_SIZE_RANGE: (f32, f32) = (10., 20.);
pub const MONO_SIZE_RANGE: (f32, f32) = (9., 24.);
pub const COLUMN_RANGE: (f32, f32) = (220., 480.);

impl Default for Prefs {
    fn default() -> Self {
        Self {
            appearance: Appearance::System,
            light_theme: themes::DEFAULT_LIGHT.to_string(),
            dark_theme: themes::DEFAULT_DARK.to_string(),
            ui_font_family: crate::theme::UI_FONT.to_string(),
            ui_font_size: 15.,
            mono_font_family: crate::theme::MONO_FONT.to_string(),
            mono_font_size: 13.,
            accent: "Violet".to_string(),
            reopen_last: true,
            editor_command: String::new(),
            editor_app: String::new(),
            terminal_app: String::new(),
            column_width: 300.,
            show_empty_columns: false,
            split_blocked: true,
            card_id: true,
            card_project: true,
            card_fields: true,
            card_mode: true,
            card_checklist: true,
            card_relations: true,
            card_age: true,
            card_updated: false,
            density: DENSITIES[0].to_string(),
            font_defaults: FONT_DEFAULTS,
            notify_enabled: true,
            notify_background_only: true,
            notify_agent_done: true,
            notify_agent_started: false,
            notify_unblocked: true,
            notify_needs_you: true,
            notify_closed: true,
        }
    }
}

impl Prefs {
    fn sanitized(mut self) -> Self {
        let d = Prefs::default();
        // Files from older defaults (Pravka, then Inter): move them on.
        if self.font_defaults < FONT_DEFAULTS {
            if self.ui_font_family == crate::theme::TUSK_FONT
                || (self.font_defaults == 2 && self.ui_font_family == crate::theme::INTER_FONT)
            {
                self.ui_font_family = d.ui_font_family.clone();
            }
            if self.mono_font_family == crate::theme::TUSK_FONT {
                self.mono_font_family = d.mono_font_family.clone();
            }
            // The UI scale moved from 14 to 15 (caption text read too small).
            if self.font_defaults < 4 && self.ui_font_size == 14. {
                self.ui_font_size = d.ui_font_size;
            }
            self.font_defaults = FONT_DEFAULTS;
        }
        if !themes::names(true).contains(&self.light_theme.as_str()) {
            self.light_theme = d.light_theme;
        }
        if !themes::names(false).contains(&self.dark_theme.as_str()) {
            self.dark_theme = d.dark_theme;
        }
        if self.ui_font_family.trim().is_empty() {
            self.ui_font_family = d.ui_font_family;
        }
        if self.mono_font_family.trim().is_empty() {
            self.mono_font_family = d.mono_font_family;
        }
        self.ui_font_size = self
            .ui_font_size
            .round()
            .clamp(UI_SIZE_RANGE.0, UI_SIZE_RANGE.1);
        self.mono_font_size = self
            .mono_font_size
            .round()
            .clamp(MONO_SIZE_RANGE.0, MONO_SIZE_RANGE.1);
        self.column_width = self
            .column_width
            .round()
            .clamp(COLUMN_RANGE.0, COLUMN_RANGE.1);
        if !ACCENTS.iter().any(|(n, _, _)| *n == self.accent) {
            self.accent = d.accent.clone();
        }
        if !DENSITIES.contains(&self.density.as_str()) {
            self.density = d.density;
        }
        self
    }

    pub fn compact(&self) -> bool {
        self.density == "Compact"
    }
}

// Behind an `Arc`: render code reads settings per cell, a whole-struct clone
// each time would copy every string and map.
static PREFS: LazyLock<RwLock<std::sync::Arc<Prefs>>> =
    LazyLock::new(|| RwLock::new(std::sync::Arc::new(load())));
/// The two font families as ready `SharedString`s (UI, table).
static FONTS: LazyLock<RwLock<(SharedString, SharedString)>> = LazyLock::new(|| {
    let p = get();
    RwLock::new((
        crate::theme::family(&p.ui_font_family).into(),
        crate::theme::family(&p.mono_font_family).into(),
    ))
});

fn path() -> std::path::PathBuf {
    crate::store::dir().join("settings.json")
}

fn load() -> Prefs {
    std::fs::read_to_string(path())
        .ok()
        .and_then(|t| serde_json::from_str::<Prefs>(&t).ok())
        .unwrap_or_default()
        .sanitized()
}

fn save(p: &Prefs) {
    let path = path();
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    match serde_json::to_string_pretty(p) {
        Ok(text) => {
            if let Err(e) = std::fs::write(&path, text) {
                log::warn!("settings save failed: {e}");
            }
        }
        Err(e) => log::warn!("settings serialize failed: {e}"),
    }
}

pub fn get() -> std::sync::Arc<Prefs> {
    PREFS.read().map(|p| p.clone()).unwrap_or_default()
}

/// Change a setting: save, re-apply the theme and redraw every window.
pub fn update(cx: &mut App, f: impl FnOnce(&mut Prefs)) {
    let mut p = (*get()).clone();
    f(&mut p);
    let p = p.sanitized();
    if p == *get() {
        return;
    }
    save(&p);
    if let Ok(mut fonts) = FONTS.write() {
        *fonts = (
            crate::theme::family(&p.ui_font_family).into(),
            crate::theme::family(&p.mono_font_family).into(),
        );
    }
    if let Ok(mut w) = PREFS.write() {
        *w = std::sync::Arc::new(p);
    }
    crate::theme::apply(cx);
    cx.refresh_windows();
}

/// Is the light theme in effect right now (setting + macOS appearance)?
pub fn is_light(cx: &App) -> bool {
    match get().appearance {
        Appearance::Light => true,
        Appearance::Dark => false,
        Appearance::System => matches!(
            cx.window_appearance(),
            WindowAppearance::Light | WindowAppearance::VibrantLight
        ),
    }
}

/// The theme in effect right now.
pub fn active_theme(cx: &App) -> String {
    let p = get();
    if is_light(cx) {
        p.light_theme.clone()
    } else {
        p.dark_theme.clone()
    }
}

/// Pick a theme from the palette: it goes into its own mode's slot, and the
/// appearance switches to that mode if it isn't showing already — so the
/// choice is always visible.
pub fn choose_theme(cx: &mut App, name: &str) {
    let light = themes::names(true).contains(&name);
    let showing_light = is_light(cx);
    update(cx, |p| {
        if light {
            p.light_theme = name.to_string();
        } else {
            p.dark_theme = name.to_string();
        }
        if light != showing_light {
            p.appearance = if light {
                Appearance::Light
            } else {
                Appearance::Dark
            };
        }
    });
}

pub fn ui_font() -> SharedString {
    FONTS.read().map(|f| f.0.clone()).unwrap_or_default()
}

pub fn mono_font() -> SharedString {
    FONTS.read().map(|f| f.1.clone()).unwrap_or_default()
}

// ---------------------------------------------------------------------------
// Settings window
// ---------------------------------------------------------------------------

#[derive(Default)]
struct OpenSettings(Option<AnyWindowHandle>);
impl Global for OpenSettings {}

type FontSelect = Entity<SelectState<SearchableVec<SharedString>>>;

pub struct SettingsWindow {
    focus: FocusHandle,
    /// Searchable pickers: a system has hundreds of families, too many for the
    /// kit's (non-virtualized) dropdown menu.
    ui_font: FontSelect,
    mono_font: FontSelect,
    _subs: Vec<Subscription>,
}

impl SettingsWindow {
    /// Open (or focus) the settings window.
    pub fn open(cx: &mut App) {
        if let Some(handle) = cx.default_global::<OpenSettings>().0
            && cx
                .update_window(handle, |_, window, _| window.activate_window())
                .is_ok()
        {
            return;
        }
        let bounds = Bounds::centered(None, size(px(860.), px(600.)), cx);
        let result = cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                window_min_size: Some(size(px(560.), px(360.))),
                focus: !crate::background(),
                ..TitleBar::window_options()
            },
            |window, cx| {
                let view = cx.new(|cx| SettingsWindow::new(window, cx));
                view.read(cx).focus.clone().focus(window, cx);
                cx.new(|cx| Root::new(view, window, cx))
            },
        );
        match result {
            Ok(handle) => cx.set_global(OpenSettings(Some(handle.into()))),
            Err(e) => log::warn!("settings window failed to open: {e}"),
        }
    }

    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let fonts = font_options(cx);
        let p = get();
        let mut picker = |current: &str, set: fn(&mut Prefs, String)| {
            let ix = fonts.iter().position(|f| f.as_ref() == current);
            let state = cx.new(|cx| {
                SelectState::new(
                    SearchableVec::new(fonts.clone()),
                    ix.map(IndexPath::new),
                    window,
                    cx,
                )
                .searchable(true)
            });
            let sub = cx.subscribe(
                &state,
                move |_, _, ev: &SelectEvent<SearchableVec<SharedString>>, cx| {
                    let SelectEvent::Confirm(Some(v)) = ev else {
                        return;
                    };
                    let v = v.to_string();
                    update(cx, |p| set(p, v));
                },
            );
            (state, sub)
        };
        let (ui_font, s1) = picker(&p.ui_font_family, |p, v| p.ui_font_family = v);
        let (mono_font, s2) = picker(&p.mono_font_family, |p, v| p.mono_font_family = v);
        Self {
            focus: cx.focus_handle(),
            ui_font,
            mono_font,
            _subs: vec![s1, s2],
        }
    }

    /// A font picker row: the Select plus a reset to the default family.
    fn font_field(state: &FontSelect, is_ui: bool) -> SettingField<SharedString> {
        let family = move || {
            let p = get();
            if is_ui {
                p.ui_font_family.clone()
            } else {
                p.mono_font_family.clone()
            }
        };
        let default = if is_ui {
            Prefs::default().ui_font_family
        } else {
            Prefs::default().mono_font_family
        };
        let render_state = state.clone();
        let reset_state = state.clone();
        let dirty_default = default.clone();
        SettingField::render(move |_, _, _| Select::new(&render_state).small().w(px(220.)))
            .on_reset(
                move |_| family() != dirty_default,
                move |window, cx| {
                    let v = default.clone();
                    update(cx, |p| {
                        if is_ui {
                            p.ui_font_family = v.clone();
                        } else {
                            p.mono_font_family = v.clone();
                        }
                    });
                    reset_state.update(cx, |s, cx| {
                        s.set_selected_value(&SharedString::from(v), window, cx)
                    });
                },
            )
    }

    fn pages(&self) -> Vec<SettingPage> {
        let d = Prefs::default();
        let options = |light: bool| -> Vec<(SharedString, SharedString)> {
            themes::names(light)
                .into_iter()
                .map(|n| (SharedString::from(n), SharedString::from(n)))
                .collect()
        };
        let modes: Vec<(SharedString, SharedString)> = Appearance::ALL
            .into_iter()
            .map(|a| (SharedString::from(a.label()), SharedString::from(a.label())))
            .collect();
        let size_opts = |(min, max): (f32, f32)| NumberFieldOptions {
            min: min as f64,
            max: max as f64,
            step: 1.,
        };
        let switch = |title: &'static str,
                      desc: &'static str,
                      get_v: fn(&Prefs) -> bool,
                      set_v: fn(&mut Prefs, bool),
                      default: bool| {
            SettingItem::new(
                title,
                SettingField::switch(
                    move |_| get_v(&get()),
                    move |v, cx| update(cx, |p| set_v(p, v)),
                )
                .default_value(default),
            )
            .description(desc)
        };

        let general = SettingPage::new("General").icon(IconName::Settings2).group(
            SettingGroup::new()
                .title("Startup & Files")
                .item(switch(
                    "Reopen Last Board",
                    "Open the last used board when the app starts.",
                    |p| p.reopen_last,
                    |p, v| p.reopen_last = v,
                    d.reopen_last,
                ))
                .item(
                    SettingItem::new(
                        "Editor",
                        SettingField::dropdown(
                            std::iter::once((SharedString::from(""), SharedString::from("System default")))
                                .chain(crate::editors::installed().iter().map(|e| {
                                    (SharedString::from(e.name.clone()), SharedString::from(e.name.clone()))
                                }))
                                .collect(),
                            |_| get().editor_app.clone().into(),
                            |v: SharedString, cx| update(cx, |p| p.editor_app = v.to_string()),
                        )
                        .default_value(SharedString::from("")),
                    )
                    .description("Where Open in Editor opens a ticket. Found in your Applications folders."),
                )
                .item(
                    SettingItem::new(
                        "Terminal",
                        SettingField::dropdown(
                            std::iter::once((SharedString::from(""), SharedString::from("First installed")))
                                .chain(crate::terminals::installed().iter().map(|t| {
                                    (SharedString::from(t.name.clone()), SharedString::from(t.name.clone()))
                                }))
                                .collect(),
                            |_| get().terminal_app.clone().into(),
                            |v: SharedString, cx| update(cx, |p| p.terminal_app = v.to_string()),
                        )
                        .default_value(SharedString::from("")),
                    )
                    .description("Start agent opens a new window here and runs `claude` with the suggested command."),
                )
                .item(
                    SettingItem::new(
                        "Editor Command",
                        SettingField::input(
                            |_| get().editor_command.clone().into(),
                            |v: SharedString, cx| update(cx, |p| p.editor_command = v.to_string()),
                        )
                        .default_value(SharedString::from(d.editor_command.clone())),
                    )
                    .description("Advanced: a command run with the file path (for example `nvim` in a terminal wrapper). It wins over the Editor above."),
                )
        );

        let appearance = SettingPage::new("Appearance")
            .icon(IconName::Palette)
            .group(
            SettingGroup::new()
                .title("Theme")
                .item(
                    SettingItem::new(
                        "Mode",
                        SettingField::dropdown(
                            modes,
                            |_| get().appearance.label().into(),
                            |v: SharedString, cx| {
                                update(cx, |p| p.appearance = Appearance::from_label(&v))
                            },
                        )
                        .default_value(SharedString::from(d.appearance.label())),
                    )
                    .description("System follows macOS light / dark appearance."),
                )
                .item(
                    SettingItem::new(
                        "Accent Color",
                        SettingField::dropdown(
                            ACCENTS
                                .iter()
                                .map(|(n, _, _)| (SharedString::from(*n), SharedString::from(*n)))
                                .collect(),
                            |_| get().accent.clone().into(),
                            |v: SharedString, cx| update(cx, |p| p.accent = v.to_string()),
                        )
                        .default_value(SharedString::from(d.accent.clone())),
                    )
                    .description(
                        "Buttons, focus, caret, selection frames. \"Theme\" keeps the theme's own.",
                    ),
                )
                .item(
                    SettingItem::new(
                        "Light Theme",
                        SettingField::scrollable_dropdown(
                            options(true),
                            |_| get().light_theme.clone().into(),
                            |v: SharedString, cx| update(cx, |p| p.light_theme = v.to_string()),
                        )
                        .default_value(SharedString::from(d.light_theme.clone())),
                    )
                    .description("Used in light mode."),
                )
                .item(
                    SettingItem::new(
                        "Dark Theme",
                        SettingField::scrollable_dropdown(
                            options(false),
                            |_| get().dark_theme.clone().into(),
                            |v: SharedString, cx| update(cx, |p| p.dark_theme = v.to_string()),
                        )
                        .default_value(SharedString::from(d.dark_theme.clone())),
                    )
                    .description("Used in dark mode."),
                ),
        );

        let interface = SettingPage::new("Interface")
            .icon(IconName::PanelLeft)
            .group(
                SettingGroup::new()
                    .title("Fonts")
                    .item(
                        SettingItem::new("UI Font", Self::font_field(&self.ui_font, true))
                            .description("Sidebar, cards, title bar, menus and the detail view."),
                    )
                    .item(
                        SettingItem::new(
                            "UI Font Size",
                            SettingField::number_input(
                                size_opts(UI_SIZE_RANGE),
                                |_| get().ui_font_size as f64,
                                |v, cx| update(cx, |p| p.ui_font_size = v as f32),
                            )
                            .default_value(d.ui_font_size as f64),
                        )
                        .description("Scales the whole interface."),
                    )
                    .item(
                        SettingItem::new("Mono Font", Self::font_field(&self.mono_font, false))
                            .description("Ticket keys, file paths and code."),
                    )
                    .item(SettingItem::new(
                        "Mono Font Size",
                        SettingField::number_input(
                            size_opts(MONO_SIZE_RANGE),
                            |_| get().mono_font_size as f64,
                            |v, cx| update(cx, |p| p.mono_font_size = v as f32),
                        )
                        .default_value(d.mono_font_size as f64),
                    )),
            );

        let board = SettingPage::new("Board")
            .icon(IconName::SquareKanban)
            .group(
            SettingGroup::new()
                .title("Columns & Cards")
                .item(
                    SettingItem::new(
                        "Column Width",
                        SettingField::number_input(
                            NumberFieldOptions {
                                min: COLUMN_RANGE.0 as f64,
                                max: COLUMN_RANGE.1 as f64,
                                step: 10.,
                            },
                            |_| get().column_width as f64,
                            |v, cx| update(cx, |p| p.column_width = v as f32),
                        )
                        .default_value(d.column_width as f64),
                    )
                    .description("Pixels per column."),
                )
                .item(
                    SettingItem::new(
                        "Density",
                        SettingField::dropdown(
                            DENSITIES
                                .iter()
                                .map(|n| (SharedString::from(*n), SharedString::from(*n)))
                                .collect(),
                            |_| get().density.clone().into(),
                            |v: SharedString, cx| update(cx, |p| p.density = v.to_string()),
                        )
                        .default_value(SharedString::from(d.density.clone())),
                    )
                    .description("Compact cards show the title on one line."),
                )
                .item(switch(
                    "Show Empty Columns",
                    "Keep a column for every workflow stage, so it stays a drop target.",
                    |p| p.show_empty_columns,
                    |p, v| p.show_empty_columns = v,
                    d.show_empty_columns,
                ))
                .item(switch(
                    "Ticket ID",
                    "WS-12 on the top line.",
                    |p| p.card_id,
                    |p, v| p.card_id = v,
                    d.card_id,
                ))
                .item(switch(
                    "Project",
                    "Project chip when the board shows every project.",
                    |p| p.card_project,
                    |p, v| p.card_project = v,
                    d.card_project,
                ))
                .item(switch(
                    "Fields",
                    "Short fields that repeat across tickets: Type, Priority, Labels and your own.",
                    |p| p.card_fields,
                    |p, v| p.card_fields = v,
                    d.card_fields,
                ))
                .item(switch(
                    "AFK / HITL",
                    "Whether an agent can take the ticket alone or it needs a human.",
                    |p| p.card_mode,
                    |p, v| p.card_mode = v,
                    d.card_mode,
                ))
                .item(switch(
                    "Checklist Progress",
                    "Done / total checkboxes with a ring.",
                    |p| p.card_checklist,
                    |p, v| p.card_checklist = v,
                    d.card_checklist,
                ))
                .item(switch(
                    "Relations",
                    "Blocked, blocking and needs-a-human markers.",
                    |p| p.card_relations,
                    |p, v| p.card_relations = v,
                    d.card_relations,
                ))
                .item(switch(
                    "Days in Column",
                    "Dots for how long the ticket has had its status (git).",
                    |p| p.card_age,
                    |p, v| p.card_age = v,
                    d.card_age,
                ))
                .item(switch(
                    "Updated",
                    "When the file last changed.",
                    |p| p.card_updated,
                    |p, v| p.card_updated = v,
                    d.card_updated,
                )),
        );
        let notifications = SettingPage::new("Notifications")
            .icon(IconName::Bell)
            .group(
                SettingGroup::new()
                    .title("When to notify")
                    .item(switch(
                        "Notifications",
                        "Post macOS notifications for the moments below.",
                        |p| p.notify_enabled,
                        |p, v| p.notify_enabled = v,
                        d.notify_enabled,
                    ))
                    .item(switch(
                        "Only in the Background",
                        "Stay quiet while Kuzgun is the active app.",
                        |p| p.notify_background_only,
                        |p, v| p.notify_background_only = v,
                        d.notify_background_only,
                    ))
                    .item(switch(
                        "Agent Finished",
                        "An agent stopped work on a ticket: it waits on review.",
                        |p| p.notify_agent_done,
                        |p, v| p.notify_agent_done = v,
                        d.notify_agent_done,
                    ))
                    .item(switch(
                        "Agent Started",
                        "An agent began work on a ticket.",
                        |p| p.notify_agent_started,
                        |p, v| p.notify_agent_started = v,
                        d.notify_agent_started,
                    ))
                    .item(switch(
                        "Ready to Start",
                        "The last blocker of a ticket closed: it is on the frontier.",
                        |p| p.notify_unblocked,
                        |p, v| p.notify_unblocked = v,
                        d.notify_unblocked,
                    ))
                    .item(switch(
                        "Needs You",
                        "A ticket that needs a person (HITL) can start.",
                        |p| p.notify_needs_you,
                        |p, v| p.notify_needs_you = v,
                        d.notify_needs_you,
                    ))
                    .item(switch(
                        "Ticket Closed",
                        "A ticket moved to done, resolved or canceled.",
                        |p| p.notify_closed,
                        |p, v| p.notify_closed = v,
                        d.notify_closed,
                    )),
            );
        vec![general, appearance, interface, board, notifications]
    }
}

/// Installed font families for the pickers: the embedded ones first
/// (System, Inter, Geist Mono, Pravka), then the installed ones. Hidden system families
/// (`.SF…`) are skipped.
fn font_options(cx: &App) -> Vec<SharedString> {
    let mut names: Vec<String> = cx
        .text_system()
        .all_font_names()
        .into_iter()
        .filter(|n| !n.starts_with('.') && !n.is_empty())
        .collect();
    names.sort_by_key(|n| n.to_lowercase());
    names.dedup();
    names.retain(|n| !crate::theme::EMBEDDED.contains(&n.as_str()));
    crate::theme::EMBEDDED
        .iter()
        .map(|s| s.to_string())
        .chain(names)
        .map(SharedString::from)
        .collect()
}

impl Render for SettingsWindow {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = cx.theme();
        div()
            .track_focus(&self.focus)
            .size_full()
            .flex()
            .flex_col()
            .bg(t.background)
            .text_color(t.foreground)
            .font_family(ui_font())
            .child(
                TitleBar::new()
                    .child(
                        div()
                            .flex_1()
                            .flex()
                            .justify_center()
                            .text_sm()
                            .font_weight(FontWeight::MEDIUM)
                            .child("Settings"),
                    )
                    .child(div().w(px(60.))),
            )
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .child(Settings::new("kuzgun-settings").small().pages(self.pages())),
            )
    }
}
