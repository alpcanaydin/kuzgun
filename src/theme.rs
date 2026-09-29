//! Kuzgun theme: config birebir.
//!
//! Kaynak: `~/.config/zed/settings.json` + `~/.config/zed/themes/_default.json`
//! içindeki "Le Blackque orange" (dark, system modunda aktif tema).
//!
//! - UI font: Inter 14, mono font: Geist Mono 13 (Pravka stays selectable)
//!
//! - Arka plan #000000, border #222222, accent #4a90f0 (UI mavisi; tablo ikonuyla ayni)
//! - Syntax renkleri asagida birebir tema dosyasindan.
//!
//! Fontlar `assets/fonts/` altina gomulu (include_bytes ile yuklenir).

use gpui_kit::component::theme::{Theme, ThemeMode};
use gpui_kit::*;

/// "Le Blackque orange" paleti — birebir hex degerler.
/// Referans palet: fazlasi Phase 6/7'de (syntax highlight) kullanilacak.
#[allow(dead_code)]
pub mod palette {
    pub const BACKGROUND: u32 = 0x000000;
    pub const TAB_BAR: u32 = 0x00000A;
    pub const TOOLBAR: u32 = 0x00000A;
    pub const ELEVATED: u32 = 0x111111;
    pub const BORDER: u32 = 0x222222;
    pub const FOREGROUND: u32 = 0xBFBFBF; // terminal.foreground
    pub const HINT: u32 = 0xC8C8C8; // hint / secondary text
    // UI accent: the table-icon blue (user choice 2026-09-23, was the old
    // orange #FF9040). Syntax `type` stays orange — it comes from the theme.
    pub const ACCENT: u32 = 0x4A90F0; // scrollbar thumb, caret, primary
    pub const ACCENT_DIM: u32 = 0x4A90F0; // alpha varyantlari temada islenir

    // Syntax (Le Blackque orange `syntax` tablosundan birebir)
    pub const SYNTAX_KEYWORD: u32 = 0x01B0FF;
    pub const SYNTAX_STRING: u32 = 0xABE682;
    pub const SYNTAX_NUMBER: u32 = 0xD77757;
    pub const SYNTAX_COMMENT: u32 = 0x637777;
    pub const SYNTAX_FUNCTION: u32 = 0x05DCDC;
    pub const SYNTAX_TYPE: u32 = 0xFF9040;
    pub const SYNTAX_BOOLEAN: u32 = 0xFF7878;
    pub const SYNTAX_OPERATOR: u32 = 0xFF7878;
    pub const SYNTAX_VARIABLE: u32 = 0xFFDC96;
    pub const SYNTAX_CONSTANT: u32 = 0x82AAFF;
    pub const SYNTAX_PUNCT_BRACKET: u32 = 0x169FFF;

    // Status / diagnostic
    pub const RED: u32 = 0xE50000; // bright red
    pub const GREEN: u32 = 0x00D900; // bright green
    pub const YELLOW: u32 = 0xE5E500; // bright yellow
    pub const BLUE: u32 = 0x0000FF; // bright blue

    // theme_overrides (settings.json) — search vurgusu
    pub const SEARCH_MATCH: u32 = 0xFF6A00;
    pub const SEARCH_ACTIVE_MATCH: u32 = 0xFFA500;
}

/// Pending-change tints: edited rows orange, rows /
/// objects marked for deletion red, newly added rows green. Unsaved until ⌘S.
pub const EDITED: u32 = 0xFF9040;
pub const DELETED: u32 = 0xE5484D;
pub const ADDED: u32 = 0x3FB950;

/// Default fonts. The live values come from
/// the user's settings: `settings::ui_font()` / `ui_text()` (sidebar rows,
/// tabs — one step below the UI size) and `settings::table_font()` /
/// `table_text()` (grid, structure, SQL editor). `text_xs` / `text_sm` are
/// rem-based, and the kit Root sets rem = the UI font size.
/// The platform UI font (SF Pro on macOS, with its optical sizes).
pub const UI_FONT: &str = SYSTEM_FONT;
pub const MONO_FONT: &str = "Geist Mono";
/// Picker name of the platform UI font; [`family`] maps it to the real one.
pub const SYSTEM_FONT: &str = "System";
pub const INTER_FONT: &str = "Inter";
/// Tusk's font, embedded and still selectable.
pub const TUSK_FONT: &str = "Pravka";
/// Picker entries shown first: the system font and the embedded families.
pub const EMBEDDED: [&str; 4] = [SYSTEM_FONT, INTER_FONT, MONO_FONT, TUSK_FONT];

/// The family to hand the text system for a saved picker name.
pub fn family(name: &str) -> String {
    if name == SYSTEM_FONT {
        ".SystemUIFont".into()
    } else {
        name.to_string()
    }
}

/// Loads the embedded fonts (Inter, Geist Mono, Pravka) into the text system.
pub fn load_embedded_fonts(cx: &mut App) -> anyhow::Result<()> {
    use std::borrow::Cow;
    macro_rules! font {
        ($f:literal) => {
            Cow::Borrowed(include_bytes!(concat!("../assets/fonts/", $f)).as_slice())
        };
    }
    cx.text_system().add_fonts(vec![
        font!("Inter-Regular.ttf"),
        font!("Inter-Italic.ttf"),
        font!("Inter-Medium.ttf"),
        font!("Inter-MediumItalic.ttf"),
        font!("Inter-SemiBold.ttf"),
        font!("Inter-SemiBoldItalic.ttf"),
        font!("Inter-Bold.ttf"),
        font!("Inter-BoldItalic.ttf"),
        font!("GeistMono-Regular.ttf"),
        font!("GeistMono-Medium.ttf"),
        font!("GeistMono-SemiBold.ttf"),
        font!("GeistMono-Bold.ttf"),
        font!("Pravka-Regular.ttf"),
        font!("Pravka-Bold.ttf"),
        font!("Pravka-Italic.ttf"),
        font!("Pravka-BoldItalic.ttf"),
        font!("Pravka-Semibold.ttf"),
    ])?;
    Ok(())
}

/// Applies the theme + fonts from the user's settings (`settings.rs`).
/// Called at startup and whenever a setting changes.
///
/// "Kuzgun Dark" is the kit default dark theme plus the Le Blackque overrides
/// ([`apply_kuzgun_dark`]); every other theme is a kit `ThemeConfig` built from
/// a palette in `themes.rs`, so the kit derives the remaining shades.
pub fn apply(cx: &mut App) {
    use gpui_kit::component::theme::{ThemeConfig, ThemeRegistry};
    use std::rc::Rc;

    let prefs = crate::settings::get();
    let light = crate::settings::is_light(cx);
    let name = crate::settings::active_theme(cx);
    // Light mode always has a palette (Kuzgun Light is the default one).
    let pal = crate::themes::find(&name).or_else(|| {
        light
            .then(|| crate::themes::find(crate::themes::DEFAULT_LIGHT))
            .flatten()
    });
    let config: Rc<ThemeConfig> =
        match pal.map(|p| serde_json::from_value::<ThemeConfig>(crate::themes::config_json(p))) {
            Some(Ok(c)) => Rc::new(c),
            Some(Err(e)) => {
                log::warn!("theme {name} failed to parse: {e}");
                ThemeRegistry::global(cx).default_dark_theme().clone()
            }
            None => ThemeRegistry::global(cx).default_dark_theme().clone(),
        };
    let mode = if pal.is_some_and(|p| p.light) {
        ThemeMode::Light
    } else {
        ThemeMode::Dark
    };
    if !cx.has_global::<Theme>() {
        Theme::change(ThemeMode::Dark, None, cx);
    }
    {
        let theme = Theme::global_mut(cx);
        if mode.is_dark() {
            theme.dark_theme = config;
        } else {
            theme.light_theme = config;
        }
    }
    Theme::change(mode, None, cx);
    {
        let theme = Theme::global_mut(cx);
        match pal {
            None => apply_kuzgun_dark(theme),
            // Colors Kuzgun Dark sets by hand would otherwise survive a
            // switch to another theme: take them from this palette.
            Some(p) => {
                let c = |v: u32| -> Hsla { rgb(v).into() };
                theme.colors.tab_bar = c(p.surface);
                theme.colors.tab = c(p.surface);
                theme.colors.tab_active = c(p.bg);
                theme.colors.group_box = c(p.surface);
                theme.colors.input = c(crate::themes::visible_input(p));
                theme.colors.red = c(p.red);
                theme.colors.green = c(p.green);
                theme.colors.yellow = c(p.yellow);
                theme.colors.blue = c(p.blue);
                // Some themes use one color for borders and raised surfaces
                // (popovers, sheets): lift the border so it shows on both.
                let border = crate::themes::visible_border(p);
                theme.colors.border = c(border);
                let (rule, alpha) = crate::themes::grid_rule_rgba(p);
                theme.colors.table_row_border = c(rule).opacity(alpha);
            }
        }
        theme.font_family = family(&prefs.ui_font_family).into();
        theme.font_size = px(prefs.ui_font_size);
        theme.mono_font_family = family(&prefs.mono_font_family).into();
        theme.mono_font_size = px(prefs.mono_font_size);
        theme.radius = px(6.);
        theme.radius_lg = px(6.);
        // Sidebars (Settings) share the window background.
        let bg = theme.colors.background;
        theme.colors.sidebar = bg;
        theme.tokens.sidebar = bg.into();
        theme.tokens.background = bg.into();
        // Toasts bottom-right, clear of the status bar.
        theme.notification.placement = gpui_kit::Anchor::BottomRight;
        theme.notification.margins.bottom = px(40.);
        theme.notification.margins.right = px(12.);
        // Settings ▸ Appearance ▸ Accent Color.
        if let Some(&(_, dark_c, light_c)) = crate::settings::ACCENTS
            .iter()
            .find(|(n, _, _)| *n == prefs.accent && *n != "Theme")
        {
            let c: Hsla = rgb(if light { light_c } else { dark_c }).into();
            let colors = &mut theme.colors;
            colors.accent = c;
            colors.primary = c;
            colors.primary_hover = c.opacity(0.9);
            colors.primary_active = c.opacity(0.8);
            colors.caret = c;
            colors.ring = c;
            colors.link = c;
            colors.scrollbar_thumb = c;
            colors.list_active_border = c;
            colors.table_active_border = c;
            colors.drag_border = c.opacity(0.65);
            colors.drop_target = c.opacity(0.2);
        }
        // Primary buttons: the kit's hover and active tokens carry their own
        // text color; derive them from the accent so the label stays white.
        {
            use gpui_kit::component::theme::ThemeToken;
            let c = theme.colors.primary;
            let white: Hsla = rgb(0xFFFFFF).into();
            let shade: Hsla = if light {
                rgb(0x000000).into()
            } else {
                rgb(0xFFFFFF).into()
            };
            let hover = c.blend(shade.opacity(0.12));
            let active = c.blend(shade.opacity(0.22));
            theme.colors.primary_foreground = white;
            theme.colors.primary_hover = hover;
            theme.colors.primary_active = active;
            theme.tokens.primary = ThemeToken::new(white, c.into());
            theme.tokens.primary_hover = ThemeToken::new(white, hover.into());
            theme.tokens.primary_active = ThemeToken::new(white, active.into());
            theme.tokens.button_primary = ThemeToken::new(white, c.into());
            theme.tokens.button_primary_hover = ThemeToken::new(white, hover.into());
            theme.tokens.button_primary_active = ThemeToken::new(white, active.into());
            theme.colors.button_primary_foreground = white;
            // Menus and selected items fill with the accent: white text.
            theme.colors.accent_foreground = white;
            theme.tokens.accent = ThemeToken::new(white, theme.colors.accent.into());
        }
        // Selected grid row: the accent, unless it reads like a pending
        // change color (edited / deleted / added) — then plain blue.
        let accent = theme.colors.accent;
        let sel = if crate::themes::clashes_with_changes(accent) {
            Hsla::from(rgb(if light { 0x2F6FE0 } else { 0x4A90F0 }))
        } else {
            accent
        };
        theme.tokens.table_active = sel.opacity(if light { 0.16 } else { 0.24 }).into();
        // The selected cell's frame.
        theme.colors.table_active_border = sel;
    }
    Theme::sync_base(cx);
}

/// The default theme: the user's "Le Blackque orange", on top of the kit
/// default dark colors.
fn apply_kuzgun_dark(theme: &mut Theme) {
    {
        // Syntax + editor colors straight from the user's theme file
        // (`~/.config/zed/themes/_default.json` → "Le Blackque orange" with the
        // settings.json overrides). The kit's HighlightTheme uses the same
        // schema as themes, so it deserializes 1:1.
        match serde_json::from_str::<gpui_kit::component::highlighter::HighlightTheme>(
            include_str!("../assets/zed-theme.json"),
        ) {
            Ok(h) => theme.highlight_theme = std::sync::Arc::new(h),
            Err(e) => log::warn!("zed theme parse failed: {e}"),
        }

        theme.colors.background = rgb(palette::BACKGROUND).into();
        theme.colors.foreground = rgb(palette::FOREGROUND).into();
        theme.colors.accent = rgb(palette::ACCENT).into();
        theme.colors.accent_foreground = rgb(0xFFFFFF).into();
        theme.colors.border = rgb(palette::BORDER).into();
        theme.colors.primary = rgb(palette::ACCENT).into();
        theme.colors.primary_foreground = rgb(palette::BACKGROUND).into();
        theme.colors.title_bar = rgb(palette::BACKGROUND).into();
        theme.colors.title_bar_border = rgb(palette::BORDER).into();
        theme.colors.status_bar = rgb(palette::BACKGROUND).into();
        theme.colors.status_bar_border = rgb(palette::BORDER).into();
        theme.colors.muted = rgb(palette::ELEVATED).into();
        theme.colors.muted_foreground = rgb(palette::HINT).into();
        theme.colors.caret = rgb(palette::ACCENT).into();
        // Focus ring: blue (settings screenshots), independent of accent.
        theme.colors.ring = rgb(0x2F81F7).into();
        theme.colors.link = rgb(palette::SYNTAX_KEYWORD).into();
        theme.colors.scrollbar_thumb = rgb(palette::ACCENT).into();
        // Controls sit lighter than the page:
        // inputs AND default buttons derive from this token.
        // The kit draws input edges in this color and fills at 30% of it:
        // #2A edge, ~#1A fill on the #12 cards.
        theme.colors.input = rgb(0x2A2A2A).into();
        theme.colors.popover = rgb(palette::ELEVATED).into();
        theme.colors.popover_foreground = rgb(palette::FOREGROUND).into();
        // Syntax eslemesi: diagnostic/status renkleri ANSI bright karsiliklari
        theme.colors.red = rgb(palette::RED).into();
        // Softer than the terminal green for success states.
        theme.colors.green = rgb(0x4CC38A).into();
        // "modified" yellow (tab labels, warnings); ANSI #E5E500 is too acid.
        theme.colors.yellow = rgb(0xE8C84A).into();
        // Terminal bright blue (#0000FF) is unreadable on black as text; the kit
        // uses `blue` for completion-match highlights, so use UI blue.
        theme.colors.blue = rgb(0x5B9BFF).into();
    }
    theme.colors.tab_bar = rgb(palette::TAB_BAR).into();
    theme.colors.tab = rgb(0x141414).into();
    theme.colors.group_box = rgb(0x121212).into();
    // Grid: plain rows are the page (= sidebar) color, stripes a faint lift,
    // the active row a neutral gray of this theme.
    theme.tokens.table = Hsla::from(rgb(palette::BACKGROUND)).into();
    theme.tokens.table_even = Hsla::from(rgb(0x0C0C0C)).into();
    // Rules as translucent white: #1D on black, still a lighter line over
    // tinted (selected / pending) rows.
    theme.colors.table_row_border = Hsla::from(rgb(0xFFFFFF)).opacity(0.11);
}
