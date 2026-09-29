//! Kuzgun actions: every command in the palette, the menu bar and the
//! keymap goes through here.
//!
//! Board keys are single letters (j / k / h / l, space, i), so they bind in
//! the `Board` context, which is only focused while no input has focus.

use gpui_kit::*;

gpui_kit::actions!(
    kuzgun,
    [
        TogglePalette,
        CheckForUpdates,
        RestartToUpdate,
        QuickOpenTicket,
        OpenFolder,
        OpenRecent1,
        OpenRecent2,
        OpenRecent3,
        OpenRecent4,
        OpenRecent5,
        CloseBoard,
        Reload,
        CloseOverlay,
        FocusSearch,
        ToggleSidebar,
        ToggleSwimlanes,
        ToggleHideClosed,
        ToggleEmptyColumns,
        MoveDown,
        MoveUp,
        MoveLeft,
        MoveRight,
        Peek,
        OpenFull,
        CloseDetail,
        NavBack,
        NavForward,
        PrevTicket,
        NextTicket,
        OpenInEditor,
        RevealInFinder,
        CopyId,
        CopyPath,
        CopyTitle,
        CopyImplement,
        CopyTriage,
        CopyWayfinder,
        FilterAll,
        FilterFrontier,
        FilterBlocked,
        FilterHuman,
        FilterAgent,
        OpenSettings,
        HideApp,
        Quit,
    ]
);

pub const APP: &str = "KuzgunApp";
pub const BOARD: &str = "Board";

pub fn bind_keys(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("cmd-q", Quit, None),
        KeyBinding::new("cmd-h", HideApp, None),
        KeyBinding::new("cmd-,", OpenSettings, None),
        KeyBinding::new("cmd-shift-p", TogglePalette, Some(APP)),
        KeyBinding::new("cmd-k", TogglePalette, Some(APP)),
        KeyBinding::new("cmd-p", QuickOpenTicket, Some(APP)),
        KeyBinding::new("cmd-o", OpenFolder, Some(APP)),
        KeyBinding::new("cmd-1", OpenRecent1, Some(APP)),
        KeyBinding::new("cmd-2", OpenRecent2, Some(APP)),
        KeyBinding::new("cmd-3", OpenRecent3, Some(APP)),
        KeyBinding::new("cmd-4", OpenRecent4, Some(APP)),
        KeyBinding::new("cmd-5", OpenRecent5, Some(APP)),
        KeyBinding::new("cmd-w", CloseBoard, Some(APP)),
        KeyBinding::new("cmd-r", Reload, Some(APP)),
        KeyBinding::new("cmd-f", FocusSearch, Some(APP)),
        KeyBinding::new("cmd-b", ToggleSidebar, Some(APP)),
        KeyBinding::new("cmd-e", OpenInEditor, Some(APP)),
        KeyBinding::new("cmd-shift-r", RevealInFinder, Some(APP)),
        KeyBinding::new("cmd-.", CopyId, Some(APP)),
        KeyBinding::new("cmd-shift-.", CopyPath, Some(APP)),
        KeyBinding::new("cmd-shift-c", CopyTitle, Some(APP)),
        KeyBinding::new("escape", CloseOverlay, Some(APP)),
        KeyBinding::new("cmd-[", NavBack, Some(APP)),
        KeyBinding::new("cmd-]", NavForward, Some(APP)),
        // Board navigation, only while no text field has focus.
        KeyBinding::new("j", MoveDown, Some(BOARD)),
        KeyBinding::new("down", MoveDown, Some(BOARD)),
        KeyBinding::new("k", MoveUp, Some(BOARD)),
        KeyBinding::new("up", MoveUp, Some(BOARD)),
        KeyBinding::new("h", MoveLeft, Some(BOARD)),
        KeyBinding::new("left", MoveLeft, Some(BOARD)),
        KeyBinding::new("l", MoveRight, Some(BOARD)),
        KeyBinding::new("right", MoveRight, Some(BOARD)),
        KeyBinding::new("space", Peek, Some(BOARD)),
        KeyBinding::new("enter", OpenFull, Some(BOARD)),
        KeyBinding::new("t", ToggleSwimlanes, Some(BOARD)),
        KeyBinding::new("/", FocusSearch, Some(BOARD)),
        KeyBinding::new("i", CopyImplement, Some(BOARD)),
        KeyBinding::new("0", FilterAll, Some(BOARD)),
        KeyBinding::new("1", FilterFrontier, Some(BOARD)),
        KeyBinding::new("2", FilterBlocked, Some(BOARD)),
        KeyBinding::new("3", FilterAgent, Some(BOARD)),
        KeyBinding::new("4", FilterHuman, Some(BOARD)),
    ]);
}
