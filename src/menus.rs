//! macOS menu bar. Every item is an action that also has a keybinding
//! (actions.rs), so the menu shows the shortcut next to it.

use gpui_kit::component::input::{Copy, Cut, Paste, Redo, SelectAll, Undo};
use gpui_kit::*;

use crate::actions::*;

pub fn install(cx: &mut App) {
    cx.on_action(|_: &Quit, cx| cx.quit());
    cx.on_action(|_: &HideApp, cx| cx.hide());
    cx.on_action(|_: &OpenSettings, cx| crate::settings::SettingsWindow::open(cx));
    cx.set_menus([
        Menu::new("Kuzgun").items([
            MenuItem::action("Settings…", OpenSettings),
            MenuItem::action("Command Palette…", TogglePalette),
            MenuItem::separator(),
            MenuItem::action("Hide Kuzgun", HideApp),
            MenuItem::action("Quit Kuzgun", Quit),
        ]),
        Menu::new("File").items([
            MenuItem::action("Open Folder…", OpenFolder),
            MenuItem::action("Go to Ticket…", QuickOpenTicket),
            MenuItem::separator(),
            MenuItem::action("Reload Board", Reload),
            MenuItem::action("Close Board", CloseBoard),
        ]),
        Menu::new("Edit").items([
            MenuItem::os_action("Undo", Undo, OsAction::Undo),
            MenuItem::os_action("Redo", Redo, OsAction::Redo),
            MenuItem::separator(),
            MenuItem::os_action("Cut", Cut, OsAction::Cut),
            MenuItem::os_action("Copy", Copy, OsAction::Copy),
            MenuItem::os_action("Paste", Paste, OsAction::Paste),
            MenuItem::os_action("Select All", SelectAll, OsAction::SelectAll),
            MenuItem::separator(),
            MenuItem::action("Find…", FocusSearch),
        ]),
        Menu::new("View").items([
            MenuItem::action("Toggle Sidebar", ToggleSidebar),
            MenuItem::action("Swimlanes by Project", ToggleSwimlanes),
            MenuItem::action("Hide Done & Canceled", ToggleHideClosed),
            MenuItem::action("Show Empty Columns", ToggleEmptyColumns),
            MenuItem::separator(),
            MenuItem::action("All Tickets", FilterAll),
            MenuItem::action("Frontier", FilterFrontier),
            MenuItem::action("Blocked", FilterBlocked),
            MenuItem::action("Agent-ready (AFK)", FilterAgent),
            MenuItem::action("Needs a Human (HITL)", FilterHuman),
        ]),
        Menu::new("Ticket").items([
            MenuItem::action("Peek", Peek),
            MenuItem::action("Open", OpenFull),
            MenuItem::action("Back", NavBack),
            MenuItem::action("Forward", NavForward),
            MenuItem::separator(),
            MenuItem::action("Copy /implement Command", CopyImplement),
            MenuItem::action("Copy /triage Command", CopyTriage),
            MenuItem::action("Copy /wayfinder Command", CopyWayfinder),
            MenuItem::separator(),
            MenuItem::action("Open in Editor", OpenInEditor),
            MenuItem::action("Reveal in Finder", RevealInFinder),
            MenuItem::separator(),
            MenuItem::action("Copy ID", CopyId),
            MenuItem::action("Copy Title", CopyTitle),
            MenuItem::action("Copy Path", CopyPath),
        ]),
    ]);
}
