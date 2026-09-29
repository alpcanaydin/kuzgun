//! Bottom-right toasts (the kit's notification layer) for results and errors.

use gpui_kit::component::WindowExt as _;
use gpui_kit::component::notification::Notification;
use gpui_kit::{App, Window};

/// Push on the next frame: callers are usually inside a render or an
/// entity update, where the window's root can't be borrowed.
/// `ok`: Some(true) success, Some(false) error, None neutral.
pub fn push(window: &mut Window, cx: &mut App, ok: Option<bool>, msg: String) {
    push_at(window, cx, ok, msg, None);
}

fn push_at(
    window: &mut Window,
    cx: &mut App,
    ok: Option<bool>,
    msg: String,
    at: Option<gpui_kit::Anchor>,
) {
    window.defer(cx, move |window, cx| {
        let mut note = match ok {
            Some(true) => Notification::success(msg),
            Some(false) => Notification::error(msg),
            None => Notification::info(msg),
        };
        if let Some(at) = at {
            note = note.placement(at);
        }
        window.push_notification(note, cx);
    });
}
