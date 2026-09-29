//! Saved boards (the welcome screen's list) and per-board view state.
//!
//! Both live in `~/Library/Application Support/kuzgun/` as JSON.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct SavedBoard {
    pub path: PathBuf,
    /// Display name; defaults to the folder name.
    pub name: String,
    /// Unix seconds of the last open.
    pub last_opened: i64,
    #[serde(default)]
    pub pinned: bool,
    /// Ticket count at the last open, for the welcome row.
    #[serde(default)]
    pub tickets: usize,
}

/// What a board remembers between opens.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct ViewState {
    /// Collapsed column statuses.
    pub collapsed: Vec<String>,
    /// Sidebar project filter (project folder name).
    pub project: Option<String>,
    pub swimlanes: bool,
    pub hide_closed: bool,
    pub sidebar_hidden: bool,
    pub detail_w: Option<f32>,
    pub selected: Option<PathBuf>,
    pub layout: Layout,
    pub ordering: Ordering,
    pub completed: Completed,
    /// Folded sections of the sidebar and the detail (`side:…`, `detail:…`).
    pub folded: Vec<String>,
    /// The sidebar lists projects whose tickets are all closed.
    pub show_finished_projects: bool,
    /// The page in the main area.
    pub page: Page,
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub enum Page {
    #[default]
    Board,
    Home,
    Dependencies,
    Agents,
}

/// Board columns or a list grouped by status (Linear's Display ▸ Layout).
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub enum Layout {
    #[default]
    Board,
    List,
}

/// Card order inside a column or group.
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub enum Ordering {
    #[default]
    Number,
    Updated,
    TimeInStatus,
    Title,
}

impl Ordering {
    pub const ALL: [Ordering; 4] = [
        Ordering::Number,
        Ordering::Updated,
        Ordering::TimeInStatus,
        Ordering::Title,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Ordering::Number => "Number",
            Ordering::Updated => "Last updated",
            Ordering::TimeInStatus => "Time in status",
            Ordering::Title => "Title",
        }
    }
}

/// Which closed tickets show (Linear's Display ▸ Completed issues).
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub enum Completed {
    #[default]
    All,
    PastWeek,
    PastMonth,
    None,
}

impl Completed {
    pub const ALL: [Completed; 4] = [
        Completed::All,
        Completed::PastWeek,
        Completed::PastMonth,
        Completed::None,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Completed::All => "All",
            Completed::PastWeek => "Past week",
            Completed::PastMonth => "Past month",
            Completed::None => "None",
        }
    }
    /// Seconds a closed ticket stays visible; None = no limit.
    pub fn window(self) -> Option<i64> {
        match self {
            Completed::All => None,
            Completed::PastWeek => Some(7 * 86_400),
            Completed::PastMonth => Some(30 * 86_400),
            Completed::None => Some(0),
        }
    }
}

pub fn dir() -> PathBuf {
    dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("kuzgun")
}

fn boards_path() -> PathBuf {
    dir().join("boards.json")
}

fn views_path() -> PathBuf {
    dir().join("views.json")
}

fn read_json<T: for<'a> Deserialize<'a> + Default>(path: &Path) -> T {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

fn write_json<T: Serialize>(path: &Path, v: &T) {
    if let Some(d) = path.parent() {
        let _ = std::fs::create_dir_all(d);
    }
    match serde_json::to_string_pretty(v) {
        Ok(text) => {
            // Write then rename, so a crash never leaves half a file.
            let tmp = path.with_extension("json.tmp");
            if std::fs::write(&tmp, text).is_ok() {
                let _ = std::fs::rename(&tmp, path);
            }
        }
        Err(e) => log::warn!("save {} failed: {e}", path.display()),
    }
}

/// Saved boards, pinned first, then the most recently opened.
pub fn load_boards() -> Vec<SavedBoard> {
    let mut v: Vec<SavedBoard> = read_json(&boards_path());
    sort(&mut v);
    v
}

fn sort(v: &mut [SavedBoard]) {
    v.sort_by(|a, b| {
        b.pinned
            .cmp(&a.pinned)
            .then(b.last_opened.cmp(&a.last_opened))
    });
}

pub fn save_boards(v: &[SavedBoard]) {
    write_json(&boards_path(), &v);
}

/// Records an open (adds the board when new) and returns the saved list.
pub fn touch_board(path: &Path, tickets: usize) -> Vec<SavedBoard> {
    let mut v = load_boards();
    let now = chrono::Utc::now().timestamp();
    match v.iter_mut().find(|b| b.path == path) {
        Some(b) => {
            b.last_opened = now;
            b.tickets = tickets;
        }
        None => v.push(SavedBoard {
            path: path.to_path_buf(),
            name: default_name(path),
            last_opened: now,
            pinned: false,
            tickets,
        }),
    }
    sort(&mut v);
    save_boards(&v);
    v
}

/// `.scratch` inside `they` → `they`; otherwise the folder name.
pub fn default_name(path: &Path) -> String {
    let own = path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();
    let generic = own.starts_with('.')
        || matches!(
            own.as_str(),
            "issues" | "tickets" | "tasks" | "docs" | "board"
        );
    if generic && let Some(parent) = path.parent().and_then(|p| p.file_name()) {
        return parent.to_string_lossy().to_string();
    }
    own
}

pub fn view_state(path: &Path) -> ViewState {
    let all: BTreeMap<String, ViewState> = read_json(&views_path());
    all.get(&path.display().to_string())
        .cloned()
        .unwrap_or_default()
}

pub fn save_view_state(path: &Path, v: &ViewState) {
    let mut all: BTreeMap<String, ViewState> = read_json(&views_path());
    let key = path.display().to_string();
    if all.get(&key) == Some(v) {
        return;
    }
    all.insert(key, v.clone());
    write_json(&views_path(), &all);
}

/// `/Users/me/x` → `~/x`.
pub fn tilde(path: &Path) -> String {
    if let Some(home) = dirs::home_dir()
        && let Ok(rest) = path.strip_prefix(&home)
    {
        return format!("~/{}", rest.display());
    }
    path.display().to_string()
}
