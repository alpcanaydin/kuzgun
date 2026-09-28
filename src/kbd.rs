//! Shortcut key caps (⌘⇧O) wherever a shortcut is shown, in the system
//! font: it has the modifier glyphs whatever the UI font is.

use gpui_kit::component::kbd::Kbd;
use gpui_kit::*;

/// One shortcut in keymap syntax (`cmd-shift-o`, `escape`) as a row of
/// caps at a given text size. Chords are space separated.
pub fn caps_sized(keys: &str, size: Pixels) -> AnyElement {
    caps_styled(keys, None, size)
}

/// Caps drawn in `color` (label + a faint border of it), for caps inside a
/// sentence of that color.
fn caps_styled(keys: &str, color: Option<Hsla>, size: Pixels) -> AnyElement {
    let caps = keys
        .split_whitespace()
        .filter_map(|k| Keystroke::parse(k).ok())
        .map(|k| {
            let kbd = Kbd::new(k).outline().font_family(".SystemUIFont").text_size(size);
            match color {
                Some(c) => kbd
                    .text_color(c)
                    .border_color(c.opacity(0.4))
                    .bg(gpui_kit::transparent_black()),
                None => kbd,
            }
        });
    div().flex().flex_none().gap_1().children(caps).into_any_element()
}

