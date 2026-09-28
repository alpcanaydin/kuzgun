//! App shell: the welcome screen and the board, the title and status bars,
//! the command palette overlay, live reload and every file write.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

use futures::StreamExt as _;
use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::component::menu::{ContextMenuExt as _, DropdownMenu as _, PopupMenuItem};
use gpui_kit::component::text::TextViewState;
use gpui_kit::component::theme::ActiveTheme as _;
use gpui_kit::component::{Icon, Sizable as _, TitleBar};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::actions::*;
use crate::detail::Detail;
use crate::model::{self, Board, Category, Column};
use crate::palette::{PaletteMode, PaletteOverlay};
use crate::store::{self, SavedBoard, ViewState};

/// Global handle so `&mut App`-only callbacks (menus, palette) reach the app.
pub struct KuzgunHandle(pub Entity<KuzgunApp>);
impl Global for KuzgunHandle {}

pub fn with_app(cx: &mut App, f: impl FnOnce(&mut KuzgunApp, &mut Context<KuzgunApp>)) {
    let view = cx.global::<KuzgunHandle>().0.clone();
    view.update(cx, f);
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Screen {
    Welcome,
    Board,
}

/// Quick filters (Jira's quick filter buttons).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Quick {
    #[default]
    All,
    /// Wayfinder's frontier: open, unclaimed, every blocker closed.
    Frontier,
    Blocked,
    /// Needs a person (HITL).
    Human,
    /// An agent can take it alone (AFK).
    Agent,
    /// Changed in the last 24 hours, newest first (Linear's Activity).
    Recent,
    /// Inconsistent or stuck: stale status, broken refs, blocker cycles.
    Attention,
}

impl Quick {
    pub const ALL: [Quick; 7] = [Quick::All, Quick::Recent, Quick::Frontier, Quick::Blocked, Quick::Agent, Quick::Human, Quick::Attention];
    pub fn label(self) -> &'static str {
        match self {
            Quick::All => "All",
            Quick::Frontier => "Frontier",
            Quick::Blocked => "Blocked",
            Quick::Human => "Needs a human (HITL)",
            Quick::Agent => "Agent-ready (AFK)",
            Quick::Recent => "Recently updated",
            Quick::Attention => "Needs attention",
        }
    }

    fn slot(self) -> usize {
        match self {
            Quick::All => 0,
            Quick::Frontier => 1,
            Quick::Blocked => 2,
            Quick::Human => 3,
            Quick::Agent => 4,
            Quick::Recent => 5,
            Quick::Attention => 6,
        }
    }

    pub fn count(self, idx: &Index) -> usize {
        idx.quick[self.slot()]
    }

    /// The count, with Recently updated taken from git (it loads later).
    pub fn count_in(self, app: &KuzgunApp) -> usize {
        if self == Quick::Recent {
            let now = now_unix();
            return (0..app.board.tickets.len()).filter(|&i| now - app.updated_at(i) <= 86_400).count();
        }
        self.count(&app.idx)
    }
}

/// The derived column of unstarted tickets that wait on open blockers.
/// One agent conversation and the markdown views of its messages.
#[derive(Default)]
pub struct Conversation {
    pub transcript: crate::transcript::Transcript,
    pub md: Vec<Option<Entity<TextViewState>>>,
}

pub const WAITING: &str = "waiting-on-blockers";

/// Filter keys that are not ticket fields.
pub const FILTER_STATUS: &str = "Status";
pub const FILTER_RELATIONS: &str = "Relations";
pub const FILTER_MODE: &str = "Mode";

/// Facts about the loaded board, computed once per load so render stays
/// cheap (it runs on every scroll frame).
#[derive(Default)]
pub struct Index {
    pub blocked: Vec<bool>,
    pub frontier: Vec<bool>,
    pub modes: Vec<Option<model::Mode>>,
    /// Lowercase search text per ticket.
    pub hay: Vec<String>,
    pub open_blockers: Vec<Vec<usize>>,
    pub blocking_open: Vec<usize>,
    /// Counts per quick view, in `Quick::ALL` slot order.
    pub quick: [usize; 7],
    /// Why a ticket needs a look (stale status, broken refs, cycles…).
    pub attention: Vec<Vec<String>>,
}

pub struct KuzgunApp {
    pub focus: FocusHandle,
    pub board_focus: FocusHandle,
    pub screen: Screen,
    pub saved: Vec<SavedBoard>,
    pub root: Option<PathBuf>,
    pub board: Board,
    pub loading: bool,
    pub view: ViewState,
    pub quick: Quick,
    /// Facet filters: field key → value.
    pub filters: std::collections::BTreeMap<String, String>,
    /// Per-board facts computed once per load (render reads these).
    pub idx: Index,
    /// Virtualized card lists, one per column status.
    pub lists: std::cell::RefCell<HashMap<String, (ListState, std::rc::Rc<Vec<usize>>)>>,
    /// Card layout inputs; a change remeasures the lists.
    pub layout_sig: std::cell::Cell<u64>,
    pub search: Entity<InputState>,
    pub selected: Option<PathBuf>,
    pub detail: Option<Detail>,
    pub detail_drag: Option<(f32, f32)>,
    /// Detail pages behind and ahead of the current one: (path, is doc).
    pub nav_back: Vec<(PathBuf, bool)>,
    pub nav_fwd: Vec<(PathBuf, bool)>,
    pub palette: Option<PaletteOverlay>,
    pub palette_runs: Vec<crate::palette::RunFn>,
    pub toasts: Vec<(Option<bool>, String)>,
    /// Cards changed on disk a moment ago (they glow for a second).
    pub flash: HashMap<PathBuf, Instant>,
    /// Per file: (status since, created), unix seconds, from git.
    pub ages: HashMap<PathBuf, crate::git::FileGit>,
    pub last_change: Option<(SystemTime, usize)>,
    pub live: bool,
    /// Agent runs from the Claude Code transcripts of this board's repo.
    pub agent_runs: Vec<crate::agents::AgentRun>,
    _history_task: Option<Task<()>>,
    /// Agent runs of any age, read once per board. Only the session page
    /// uses them; statuses follow the recent runs alone.
    pub history_runs: Vec<crate::agents::AgentRun>,
    /// Agent conversations read so far, by session file.
    pub conversations: HashMap<PathBuf, Conversation>,
    /// The board as last seen, to notify about what changed.
    snaps: HashMap<PathBuf, crate::notify::Snap>,
    _agents_task: Option<Task<()>>,
    pub collapsed_lanes: Vec<usize>,
    pub board_scroll: ScrollHandle,
    /// Dependency map: scroll, zoom and a drag-to-pan in progress.
    pub deps_scroll: ScrollHandle,
    pub deps_zoom: f32,
    pub deps_drag: Option<(Point<Pixels>, Point<Pixels>)>,
    /// The agent session page, when open.
    pub session: Option<crate::session::SessionView>,
    /// A ticket whose session page opens once its agent runs are read.
    pub pending_session: Option<PathBuf>,
    /// A ticket key to open once the board loads (`--ticket WS-5`).
    pub pending_ticket: Option<String>,
    /// Why the last picked folder did not open, shown on the welcome screen.
    pub welcome_notice: Option<String>,
    /// A drag on the dependency minimap is in progress.
    pub deps_mini_drag: bool,
    /// A sidebar facet (Type, Tranche...) picked as a flat results list.
    pub facet_view: Option<(String, String)>,
    watcher: Option<crate::watch::BoardWatcher>,
    _watch_task: Option<Task<()>>,
    _load_task: Option<Task<()>>,
    _ages_task: Option<Task<()>>,
    _tick: Option<Task<()>>,
    _subs: Vec<Subscription>,
}

impl KuzgunApp {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let search = cx.new(|cx| {
            InputState::new(window, cx).placeholder("Filter by title, key, status, label…")
        });
        let sub = cx.subscribe_in(&search, window, |this, _, ev: &InputEvent, window, cx| match ev {
            InputEvent::Change => cx.notify(),
            InputEvent::PressEnter { .. } => {
                this.select_first_visible(cx);
                this.board_focus.focus(window, cx);
            }
            _ => {}
        });
        Self {
            focus: cx.focus_handle(),
            board_focus: cx.focus_handle(),
            screen: Screen::Welcome,
            saved: store::load_boards(),
            root: None,
            board: Board::default(),
            loading: false,
            view: ViewState::default(),
            quick: Quick::All,
            filters: Default::default(),
            idx: Index::default(),
            lists: Default::default(),
            layout_sig: Default::default(),
            search,
            selected: None,
            detail: None,
            detail_drag: None,
            nav_back: Vec::new(),
            nav_fwd: Vec::new(),
            palette: None,
            palette_runs: Vec::new(),
            toasts: Vec::new(),
            flash: HashMap::new(),
            ages: HashMap::new(),
            last_change: None,
            live: false,
            agent_runs: Vec::new(),
            snaps: HashMap::new(),
            _agents_task: None,
            collapsed_lanes: Vec::new(),
            board_scroll: ScrollHandle::new(),
            deps_scroll: ScrollHandle::new(),
            deps_zoom: 1.,
            deps_drag: None,
            deps_mini_drag: false,
            welcome_notice: None,
            pending_ticket: None,
            session: None,
            pending_session: None,
            conversations: HashMap::new(),
            history_runs: Vec::new(),
            _history_task: None,
            facet_view: None,
            watcher: None,
            _watch_task: None,
            _load_task: None,
            _ages_task: None,
            _tick: None,
            _subs: vec![sub],
        }
    }

    pub fn toast(&mut self, ok: impl Into<Option<bool>>, msg: impl Into<String>) {
        self.toasts.push((ok.into(), msg.into()));
    }

    // ---------- boards ----------

    /// Startup: reopen the last board when the setting says so.
    pub fn auto_open(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !crate::settings::get().reopen_last {
            return;
        }
        if let Some(b) = self.saved.iter().max_by_key(|b| b.last_opened).cloned()
            && b.path.is_dir()
        {
            self.open_board(b.path, window, cx);
        }
    }

    pub fn prompt_open_folder(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let rx = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("Open Board".into()),
        });
        cx.spawn_in(window, async move |this, cx| {
            let Ok(Ok(Some(paths))) = rx.await else {
                return;
            };
            let Some(path) = paths.into_iter().next() else {
                return;
            };
            let _ = this.update_in(cx, |this, window, cx| this.open_board(path, window, cx));
        })
        .detach();
    }

    pub fn open_recent(&mut self, n: usize, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(b) = self.saved.get(n).cloned() {
            self.open_board(b.path, window, cx);
        }
    }

    pub fn open_board(&mut self, path: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        if !path.is_dir() {
            self.toast(false, format!("Folder not found: {}", store::tilde(&path)));
            self.saved.retain(|b| b.path != path);
            store::save_boards(&self.saved);
            cx.notify();
            return;
        }
        let path = match model::locate_tracker(&path) {
            model::Tracker::Local(p) => p,
            model::Tracker::Remote(name) => {
                self.welcome_notice = Some(format!(
                    "{} keeps its issues on {name} (see docs/agents/issue-tracker.md). Kuzgun reads only the local markdown tracker in .scratch/.",
                    store::default_name(&path)
                ));
                if self.screen != Screen::Welcome {
                    self.close_board(window, cx);
                }
                self.saved.retain(|b| b.path != path);
                store::save_boards(&self.saved);
                cx.notify();
                return;
            }
            model::Tracker::Absent(rel) => {
                self.welcome_notice = Some(format!(
                    "{} keeps its tickets in {rel}/, but that folder is not here. Its .gitignore may leave it out of the repo.",
                    store::default_name(&path)
                ));
                if self.screen != Screen::Welcome {
                    self.close_board(window, cx);
                }
                self.saved.retain(|b| b.path != path);
                store::save_boards(&self.saved);
                cx.notify();
                return;
            }
            model::Tracker::Missing => {
                self.welcome_notice = Some(format!(
                    "{} does not use mattpocock/skills: it has no docs/agents/issue-tracker.md, no .scratch/ folder and no ticket files.",
                    store::default_name(&path)
                ));
                if self.screen != Screen::Welcome {
                    self.close_board(window, cx);
                }
                self.saved.retain(|b| b.path != path);
                store::save_boards(&self.saved);
                cx.notify();
                return;
            }
        };
        self.welcome_notice = None;
        self.save_view();
        self.root = Some(path.clone());
        self.screen = Screen::Board;
        self.view = store::view_state(&path);
        self.selected = self.view.selected.clone();
        self.detail = None;
        self.board = Board::default();
        self.quick = Quick::All;
        self.filters.clear();
        self.idx = Index::default();
        self.lists.borrow_mut().clear();
        self.ages.clear();
        self.snaps.clear();
        self.flash.clear();
        self.last_change = None;
        self.search.update(cx, |s, cx| s.set_value("", window, cx));
        self.board_focus.focus(window, cx);
        self.start_watch(&path, cx);
        self.load(Vec::new(), cx);
        self.start_tick(cx);
        self.start_agents(cx);
        window.set_window_title(&format!("{} — Kuzgun", store::default_name(&path)));
        cx.notify();
    }

    pub fn close_board(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.save_view();
        self.screen = Screen::Welcome;
        self.session = None;
        self.root = None;
        self.board = Board::default();
        self.detail = None;
        self.watcher = None;
        self._watch_task = None;
        self._tick = None;
        self._agents_task = None;
        self.agent_runs.clear();
        self.snaps.clear();
        self.live = false;
        self.saved = store::load_boards();
        self.focus.focus(window, cx);
        window.set_window_title("Kuzgun");
        cx.notify();
    }

    pub fn save_view(&mut self) {
        if let Some(root) = &self.root {
            self.view.selected = self.selected.clone();
            store::save_view_state(root, &self.view);
        }
    }

    fn start_watch(&mut self, root: &Path, cx: &mut Context<Self>) {
        match crate::watch::watch(root) {
            Ok((w, mut rx)) => {
                self.watcher = Some(w);
                self.live = true;
                self._watch_task = Some(cx.spawn(async move |this, cx| {
                    while let Some(batch) = rx.next().await {
                        if this.update(cx, |this, cx| this.load(batch, cx)).is_err() {
                            return;
                        }
                    }
                }));
            }
            Err(e) => {
                self.live = false;
                self.toast(false, format!("Live reload is off: {e}"));
            }
        }
    }

    /// Redraws once a second while the board is open: relative times
    /// ("2s ago") and the fading change glow.
    fn start_tick(&mut self, cx: &mut Context<Self>) {
        self._tick = Some(cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(Duration::from_millis(1000)).await;
                let alive = this.update(cx, |this, cx| {
                    this.flash.retain(|_, t| t.elapsed() < Duration::from_millis(2600));
                    cx.notify();
                });
                if alive.is_err() {
                    return;
                }
            }
        }));
    }

    /// Rescans the agent transcripts every few seconds (they are written
    /// continuously while an agent works, so a file watcher never settles).
    fn start_agents(&mut self, cx: &mut Context<Self>) {
        let Some(repo) = self.root.as_deref().and_then(crate::agents::repo_root) else {
            return;
        };
        self.history_runs.clear();
        let r = repo.clone();
        self._history_task = Some(cx.spawn(async move |this, cx| {
            let runs = cx.background_spawn(async move { crate::agents::history(&r) }).await;
            let _ = this.update(cx, |this, cx| {
                this.history_runs = runs;
                cx.notify();
            });
        }));
        self._agents_task = Some(cx.spawn(async move |this, cx| {
            loop {
                let r = repo.clone();
                let runs = cx.background_spawn(async move { crate::agents::scan(&r) }).await;
                if std::env::var_os("KUZGUN_DEBUG_AGENTS").is_some() {
                    for r in &runs {
                        eprintln!("agent {:?} {} {}", r.state, r.ticket_rel, r.transcript.display());
                    }
                }
                let alive = this.update(cx, |this, cx| {
                    let key = |v: &[crate::agents::AgentRun]| {
                        v.iter().map(|r| (r.ticket_rel.clone(), r.state, r.last_activity / 30)).collect::<Vec<_>>()
                    };
                    if key(&runs) != key(&this.agent_runs) {
                        this.agent_runs = runs;
                        this.apply_agents();
                        this.reindex();
                        this.notify_changes(cx);
                        for (state, _) in this.lists.borrow().values() {
                            state.remeasure();
                        }
                        if let Some(mut d) = this.detail.take() {
                            d.refresh(this, cx);
                            this.detail = Some(d);
                        }
                        cx.notify();
                    }
                    if let Some(p) = this.pending_session.take() {
                        this.open_session(p, cx);
                    }
                    // A running agent's conversation grows between status changes.
                    if this.refresh_conversations(cx) {
                        cx.notify();
                    }
                });
                if alive.is_err() {
                    return;
                }
                cx.background_executor().timer(Duration::from_secs(5)).await;
            }
        }));
    }

    /// Puts what really happened on each ticket, since the files lag
    /// behind (`/implement` writes no status):
    /// - an agent working on it: In progress;
    /// - its agent stopped, worktree still there: In review;
    /// - its agent stopped, worktree gone (the work landed): Done;
    /// - started (in-review, in-progress, claimed) with every sub-task checked: Done.
    ///
    /// `status` keeps what the file says; `inferred` says why.
    /// Open projects a ticket waits on through a spec or map link.
    pub fn blocking_projects(&self, ix: usize) -> Vec<usize> {
        let mut out: Vec<usize> = Vec::new();
        for p in self.board.tickets[ix].blocked_refs.iter().filter_map(|r| r.project) {
            if self.board.project_open(p) && !out.contains(&p) {
                out.push(p);
            }
        }
        out
    }

    /// Projects newest first, by the first commit (else file) of their
    /// oldest ticket.
    pub fn project_order(&self) -> Vec<usize> {
        let mut start = vec![i64::MAX; self.board.projects.len()];
        for t in &self.board.tickets {
            let at = self
                .ages
                .get(&t.path)
                .map(|g| g.created)
                .filter(|&c| c > 0)
                .or_else(|| t.created.map(crate::agents::unix))
                .unwrap_or(i64::MAX);
            start[t.project] = start[t.project].min(at);
        }
        let mut order: Vec<usize> = (0..self.board.projects.len()).collect();
        order.sort_by_key(|&p| (std::cmp::Reverse(if start[p] == i64::MAX { i64::MIN } else { start[p] }), p));
        order
    }

    /// Every ticket of the project is closed.
    pub fn project_finished(&self, p: usize) -> bool {
        let mut it = self.board.tickets.iter().filter(|t| t.project == p).peekable();
        it.peek().is_some() && it.all(|t| t.category.is_closed())
    }

    /// Agent runs on a ticket: running first, then by last activity.
    pub fn runs_of(&self, ix: usize) -> Vec<crate::agents::AgentRun> {
        let mut runs: Vec<crate::agents::AgentRun> =
            self.agent_runs.iter().filter(|r| self.run_ticket(&r.ticket_rel) == Some(ix)).cloned().collect();
        for r in self.history_runs.iter().filter(|r| self.run_ticket(&r.ticket_rel) == Some(ix)) {
            if !runs.iter().any(|x| x.transcript == r.transcript) {
                runs.push(r.clone());
            }
        }
        // A running agent first, then the latest activity.
        runs.sort_by_key(|r| (r.state != crate::agents::RunState::Running, std::cmp::Reverse(r.last_activity)));
        runs
    }

    /// Reads new lines of the open ticket's agent conversations. Returns
    /// whether any changed.
    pub fn refresh_conversations(&mut self, cx: &mut Context<Self>) -> bool {
        let tickets: Vec<usize> = [
            self.session.as_ref().map(|s| s.ticket.clone()),
            self.detail.as_ref().filter(|d| !d.is_doc).map(|d| d.path.clone()),
        ]
        .into_iter()
        .flatten()
        .filter_map(|p| self.board.find_path(&p))
        .collect();
        let mut changed = false;
        let runs: Vec<crate::agents::AgentRun> = tickets.into_iter().flat_map(|ix| self.runs_of(ix)).collect();
        for run in runs {
            let c = self.conversations.entry(run.transcript.clone()).or_default();
            if c.transcript.update(&run.transcript, run.provider) {
                changed = true;
                for i in c.md.len()..c.transcript.entries.len() {
                    let e = &c.transcript.entries[i];
                    let md = matches!(e.kind, crate::transcript::Kind::User | crate::transcript::Kind::Assistant)
                        .then(|| cx.new(|cx| TextViewState::markdown(&e.text, cx)));
                    c.md.push(md);
                }
            }
        }
        changed
    }

    /// The ticket an agent run names: a path, a key (`WS-115`) or a bare
    /// number. A number shared by projects picks the open, newest ticket.
    pub fn run_ticket(&self, name: &str) -> Option<usize> {
        let tickets = &self.board.tickets;
        if name.ends_with(".md") {
            let canon = |p: &Path| std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
            let repo = self.root.as_deref().and_then(crate::agents::repo_root)?;
            let target = canon(&repo.join(name));
            return tickets.iter().position(|t| canon(&t.path) == target);
        }
        if let Some(i) = tickets.iter().position(|t| t.key.eq_ignore_ascii_case(name)) {
            return Some(i);
        }
        let n: u32 = name.parse().ok()?;
        (0..tickets.len())
            .filter(|&i| tickets[i].num == Some(n))
            .max_by_key(|&i| (!tickets[i].category.is_closed(), tickets[i].modified))
    }

    fn apply_agents(&mut self) {
        for t in &mut self.board.tickets {
            if t.agent.take().is_some() || t.inferred.take().is_some() {
                t.status_key = model::normalize(&t.status);
                t.category = model::categorize(&t.status);
                if let Some((role, _)) = self.board.roles.get(&t.status_key) {
                    t.category = model::categorize(role);
                }
                t.apply_ruled_out();
            }
        }
        {
            let found: Vec<Option<usize>> = self.agent_runs.iter().map(|r| self.run_ticket(&r.ticket_rel)).collect();
            for (run, ix) in self.agent_runs.iter().zip(found) {
                let Some(t) = ix.map(|i| &mut self.board.tickets[i]) else {
                    continue;
                };
                if t.agent.is_some() {
                    continue;
                }
                if !t.category.is_closed() {
                    let (key, cat, why) = match run.state {
                        crate::agents::RunState::Running => ("in-progress", Category::InProgress, "An agent is working on it"),
                        crate::agents::RunState::AwaitingReview => {
                            ("in-review", Category::InReview, "Its agent finished; the worktree waits on review")
                        }
                        crate::agents::RunState::Finished => {
                            ("done", Category::Done, "Its agent finished and the worktree is gone: the work landed")
                        }
                    };
                    t.status_key = key.into();
                    t.category = cat;
                    t.inferred = Some(why.into());
                }
                t.agent = Some(run.clone());
            }
        }
        for t in &mut self.board.tickets {
            let started = matches!(t.category, Category::InReview | Category::InProgress);
            let (done, total) = t.checklist_counts();
            if t.agent.is_none() && started && total > 0 && done == total {
                t.status_key = "done".into();
                t.category = Category::Done;
                t.inferred = Some(format!("Every sub-task is checked ({done}/{total})"));
            }
        }
    }

    /// Reads the board in the background. `changed`: paths from the watcher
    /// (empty on the first load).
    pub fn load(&mut self, changed: Vec<PathBuf>, cx: &mut Context<Self>) {
        let Some(root) = self.root.clone() else {
            return;
        };
        self.loading = true;
        let first = changed.is_empty() && self.board.tickets.is_empty();
        self._load_task = Some(cx.spawn(async move |this, cx| {
            let r = root.clone();
            let board = cx.background_spawn(async move { Board::load(&r) }).await;
            let _ = this.update(cx, |this, cx| {
                if this.root.as_deref() != Some(root.as_path()) {
                    return;
                }
                this.apply_board(board, &changed, first, cx);
            });
        }));
        cx.notify();
    }

    fn apply_board(&mut self, board: Board, changed: &[PathBuf], first: bool, cx: &mut Context<Self>) {
        // Which tickets really changed (the watcher also reports our own
        // writes and touches): compare the text.
        let old: HashMap<&Path, &str> = self
            .board
            .tickets
            .iter()
            .map(|t| (t.path.as_path(), t.raw.as_str()))
            .collect();
        let mut real = 0;
        if !first {
            for t in &board.tickets {
                if old.get(t.path.as_path()) != Some(&t.raw.as_str()) {
                    self.flash.insert(t.path.clone(), Instant::now());
                    real += 1;
                }
            }
            real += self
                .board
                .tickets
                .iter()
                .filter(|t| !board.tickets.iter().any(|n| n.path == t.path))
                .count();
        }
        if real > 0 || (!changed.is_empty() && !first) {
            self.last_change = Some((SystemTime::now(), real));
        }
        self.board = board;
        self.loading = false;
        self.apply_agents();
        self.reindex();
        self.notify_changes(cx);
        for (state, _) in self.lists.borrow().values() {
            state.remeasure();
        }
        if first {
            self.saved = store::touch_board(&self.board.root, self.board.tickets.len());
            if self.selected.as_ref().is_some_and(|p| self.board.find_path(p).is_none()) {
                self.selected = None;
            }
        }
        if let Some(d) = &self.detail
            && !d.is_doc
            && self.board.find_path(&d.path).is_none()
        {
            self.detail = None;
        }
        if first || real > 0 {
            self.load_ages(cx);
        }
        if let Some(key) = self.pending_ticket.take()
            && let Some(t) = self.board.tickets.iter().find(|t| t.key.eq_ignore_ascii_case(&key))
        {
            // Opened on its agent session when it has one.
            self.nav_back.clear();
            self.nav_fwd.clear();
            let path = t.path.clone();
            self.show(path.clone(), false, true, cx);
            self.pending_session = Some(path);
        }
        if let Some(mut d) = self.detail.take() {
            d.refresh(self, cx);
            self.detail = Some(d);
        }
        cx.notify();
    }

    fn load_ages(&mut self, cx: &mut Context<Self>) {
        let Some(root) = self.root.clone() else {
            return;
        };
        self._ages_task = Some(cx.spawn(async move |this, cx| {
            let r = root.clone();
            let ages = cx.background_spawn(async move { crate::git::board_ages(&r) }).await;
            let _ = this.update(cx, |this, cx| {
                if this.root.as_deref() == Some(root.as_path()) {
                    this.ages = ages;
                    cx.notify();
                }
            });
        }));
    }

    // ---------- file actions ----------

    pub fn current_path(&self) -> Option<PathBuf> {
        self.detail
            .as_ref()
            .map(|d| d.path.clone())
            .or_else(|| self.selected.clone())
    }

    pub fn open_in_editor(&mut self, path: &Path, _cx: &mut Context<Self>) {
        if let Err(e) = crate::editors::open(path) {
            self.toast(false, e);
        }
    }

    /// The skill command that moves this ticket on, from its state and
    /// kind, if that skill is installed: (skill, command, why).
    /// - wayfinder decision tickets: `/wayfinder <map> <ticket>`;
    /// - not triaged yet (draft, needs-triage, needs-info): `/triage`;
    /// - ready or in progress: `/implement`;
    /// - in review or closed: none.
    pub fn next_command(&self, i: usize) -> Option<(&'static str, String, &'static str)> {
        let t = &self.board.tickets[i];
        // Someone is on it, it waits on review, or it is closed: nothing to
        // hand over.
        if t.category.is_closed() || matches!(t.category, Category::InProgress | Category::InReview) {
            return None;
        }
        let rel = repo_relative(&t.path);
        let project = &self.board.projects[t.project];
        let (skill, cmd, why) = if let Some(map) = &project.map {
            ("wayfinder", format!("/wayfinder {} {rel}", repo_relative(&map.path)), "Resolve this decision on the map")
        } else if t.category == Category::Backlog {
            ("triage", format!("/triage {rel}"), "Triage it into a ready ticket")
        } else {
            ("implement", format!("/implement {rel}"), "Hand it to an agent to build")
        };
        self.skill_installed(skill).then_some((skill, cmd, why))
    }

    /// A skill is in the repo's `.claude/skills` or the user's.
    pub fn skill_installed(&self, name: &str) -> bool {
        let repo = self.root.as_deref().and_then(crate::agents::repo_root);
        repo.iter()
            .map(|r| r.join(".claude/skills").join(name))
            .chain(dirs::home_dir().map(|h| h.join(".claude/skills").join(name)))
            .any(|p| p.join("SKILL.md").is_file())
    }

    /// Starts a Claude Code session on the ticket's suggested command in
    /// the chosen terminal, in the repo folder.
    pub fn start_agent(&mut self, path: &Path, _cx: &mut Context<Self>) {
        let Some((_, cmd, _)) = self.board.find_path(path).and_then(|i| self.next_command(i)) else {
            self.toast(None, "Nothing to hand to an agent: it is in progress, in review or closed.");
            return;
        };
        let Some(repo) = self.root.as_deref().and_then(crate::agents::repo_root) else {
            self.toast(false, "This board is not in a git repo.");
            return;
        };
        match crate::terminals::start_agent(&repo, &cmd) {
            Ok(term) => self.toast(true, format!("Started in {term}: {cmd}")),
            Err(e) => self.toast(false, e),
        }
    }

    /// Copies the next command of a ticket (the `i` key).
    pub fn copy_next(&mut self, path: &Path, cx: &mut Context<Self>) {
        match self.board.find_path(path).and_then(|i| self.next_command(i)) {
            Some((_, cmd, _)) => self.copy("command", cmd, cx),
            None => self.toast(None, "Nothing to hand to an agent: it is in review or closed."),
        }
    }

    /// Picks the editor for Open in Editor and opens the file in it.
    pub fn open_with(&mut self, editor: String, path: &Path, cx: &mut Context<Self>) {
        crate::settings::update(cx, |p| {
            p.editor_app = editor;
            p.editor_command.clear();
        });
        self.open_in_editor(path, cx);
    }

    pub fn copy(&mut self, what: &str, text: String, cx: &mut Context<Self>) {
        cx.write_to_clipboard(ClipboardItem::new_string(text.clone()));
        self.toast(None, format!("Copied {what}: {text}"));
    }

    pub fn copy_current(&mut self, what: &str, cx: &mut Context<Self>) {
        let Some(path) = self.current_path() else {
            return;
        };
        let t = self.board.find_path(&path).map(|ix| self.board.tickets[ix].clone());
        let text = match (what, t) {
            ("ID", Some(t)) => t.key,
            ("title", Some(t)) => t.title,
            _ => path.display().to_string(),
        };
        self.copy(what, text, cx);
    }

    /// A slash command for the ticket, with its path from the repo root,
    /// ready to paste into an agent: `/implement .scratch/x/issues/05-y.md`.
    pub fn copy_command(&mut self, skill: &str, path: &Path, cx: &mut Context<Self>) {
        let rel = repo_relative(path);
        self.copy("command", format!("/{skill} {rel}"), cx);
    }

    // ---------- filters ----------

    pub fn query(&self, cx: &App) -> String {
        self.search.read(cx).value().trim().to_lowercase()
    }

    /// The shown project, if one is picked.
    pub fn project_ix(&self) -> Option<usize> {
        self.view.project.as_ref().and_then(|name| self.board.projects.iter().position(|p| &p.name == name))
    }

    /// Unix seconds a ticket last changed: git status move, else file mtime.
    pub fn updated_at(&self, i: usize) -> i64 {
        let t = &self.board.tickets[i];
        // Git knows when the content changed; a checkout resets the mtime.
        if let Some(g) = self.ages.get(&t.path) {
            return g.updated;
        }
        let mtime = t
            .modified
            .and_then(|m| m.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        mtime
    }

    /// Unix seconds the ticket entered its status (git), else its mtime.
    pub fn status_since(&self, i: usize) -> i64 {
        self.ages.get(&self.board.tickets[i].path).map(|a| a.since).unwrap_or_else(|| self.updated_at(i))
    }

    /// Tickets of the shown project, before any other filter.
    pub fn in_scope(&self) -> usize {
        let p = self.project_ix();
        self.board.tickets.iter().filter(|t| p.is_none_or(|p| t.project == p)).count()
    }

    /// Tickets that pass every filter, in the chosen order.
    pub fn visible(&self, cx: &App) -> Vec<usize> {
        let q = self.query(cx);
        let words: Vec<&str> = q.split_whitespace().collect();
        let project = self.project_ix();
        let window = self.view.completed.window();
        let now = now_unix();
        let mut out: Vec<usize> = (0..self.board.tickets.len())
            .filter(|&i| {
                let t = &self.board.tickets[i];
                if let Some(p) = project
                    && t.project != p
                {
                    return false;
                }
                if t.category.is_closed()
                    && let Some(w) = window
                    && now - self.status_since(i) > w
                {
                    return false;
                }
                for (k, v) in &self.filters {
                    let ok = match k.as_str() {
                        FILTER_STATUS => &t.status_key == v,
                        FILTER_RELATIONS => match v.as_str() {
                            "Blocked" => self.idx.blocked.get(i).copied().unwrap_or(false),
                            "Blocking others" => self.idx.blocking_open.get(i).copied().unwrap_or(0) > 0,
                            _ => t.blocked_by.is_empty() && t.blocks.is_empty(),
                        },
                        FILTER_MODE => self.idx.modes.get(i).copied().flatten().map(|m| m.label()) == Some(v.as_str()),
                        _ => t.field_values(k).iter().any(|x| x == v),
                    };
                    if !ok {
                        return false;
                    }
                }
                let ok = match self.quick {
                    Quick::All => true,
                    Quick::Frontier => self.idx.frontier.get(i).copied().unwrap_or(false),
                    Quick::Blocked => self.idx.blocked.get(i).copied().unwrap_or(false),
                    Quick::Human => self.idx.modes.get(i).copied().flatten() == Some(model::Mode::Hitl),
                    Quick::Agent => self.idx.modes.get(i).copied().flatten() == Some(model::Mode::Afk),
                    Quick::Recent => now - self.updated_at(i) <= 86_400,
                    Quick::Attention => self.idx.attention.get(i).is_some_and(|a| !a.is_empty()),
                };
                if !ok {
                    return false;
                }
                words.is_empty()
                    || self
                        .idx
                        .hay
                        .get(i)
                        .is_some_and(|h| words.iter().all(|w| h.contains(w)))
            })
            .collect();
        let ordering = if self.quick == Quick::Recent { store::Ordering::Updated } else { self.view.ordering };
        match ordering {
            store::Ordering::Number => {}
            store::Ordering::Updated => out.sort_by_key(|&i| std::cmp::Reverse(self.updated_at(i))),
            store::Ordering::TimeInStatus => out.sort_by_key(|&i| self.status_since(i)),
            store::Ordering::Title => out.sort_by_key(|&i| self.board.tickets[i].title.to_lowercase()),
        }
        out
    }

    /// Any filter beyond the project: views, fields, search, completed window.
    pub fn filtering(&self, cx: &App) -> bool {
        self.quick != Quick::All
            || !self.filters.is_empty()
            || !self.query(cx).is_empty()
            || self.view.completed != store::Completed::All
    }

    pub fn clear_filters(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.quick = Quick::All;
        self.filters.clear();
        self.view.completed = store::Completed::All;
        self.search.update(cx, |s, cx| s.set_value("", window, cx));
        self.save_view();
        cx.notify();
    }

    /// Compares the board with the last one seen and posts notifications.
    fn notify_changes(&mut self, cx: &App) {
        let new = crate::notify::snapshot(&self.board, &self.idx.frontier, &self.idx.modes);
        let old = std::mem::replace(&mut self.snaps, new);
        if old.is_empty() {
            return;
        }
        let prefs = crate::settings::get();
        if prefs.notify_background_only && cx.active_window().is_some() {
            return;
        }
        crate::notify::post(crate::notify::changes(&old, &self.snaps));
    }

    /// Recomputes the per-board facts after a load.
    pub fn reindex(&mut self) {
        let b = &self.board;
        let n = b.tickets.len();
        let blocked: Vec<bool> = (0..n).map(|i| b.is_blocked(i)).collect();
        let frontier: Vec<bool> = (0..n).map(|i| b.tickets[i].category == Category::Todo && !blocked[i]).collect();
        let modes: Vec<Option<model::Mode>> = (0..n).map(|i| b.mode(i)).collect();
        let hay = b
            .tickets
            .iter()
            .map(|t| {
                let p = &b.projects[t.project];
                let mut h = format!("{} {} {} {} {}", t.key, t.title, t.status, p.name, p.title);
                for (k, v) in &t.props {
                    if v.len() <= 80 {
                        h.push(' ');
                        h.push_str(k);
                        h.push(' ');
                        h.push_str(v);
                    }
                }
                h.push(' ');
                h.push_str(&t.body);
                h.to_lowercase()
            })
            .collect();
        let attention = attention_of(b, &blocked);
        let attention_n = attention.iter().filter(|a| !a.is_empty()).count();
        let quick = [
            n,
            frontier.iter().filter(|x| **x).count(),
            blocked.iter().filter(|x| **x).count(),
            modes.iter().filter(|m| **m == Some(model::Mode::Hitl)).count(),
            modes.iter().filter(|m| **m == Some(model::Mode::Afk)).count(),
            b.tickets
                .iter()
                .filter(|t| {
                    t.modified
                        .and_then(|m| m.elapsed().ok())
                        .is_some_and(|d| d.as_secs() <= 7 * 86_400)
                })
                .count(),
            attention_n,
        ];
        let open_blockers = (0..n)
            .map(|i| {
                b.tickets[i]
                    .blocked_by
                    .iter()
                    .copied()
                    .filter(|&j| !b.tickets[j].category.is_closed())
                    .collect()
            })
            .collect();
        let blocking_open = (0..n)
            .map(|i| b.tickets[i].blocks.iter().filter(|&&j| !b.tickets[j].category.is_closed()).count())
            .collect();
        self.idx = Index {
            blocked,
            frontier,
            modes,
            hay,
            open_blockers,
            blocking_open,
            quick,
            attention,
        };
    }

    /// The statuses in use in the shown project (all projects when none is
    /// picked), so a project never shows another project's empty columns.
    pub fn columns(&self) -> Vec<Column> {
        let project = self.project_ix();
        let mut cols = self.board.columns_of(project, crate::settings::get().show_empty_columns);
        if self.view.completed == store::Completed::None {
            cols.retain(|c| !c.category.is_closed());
        }
        // Blocked "ready" tickets get their own column, left of the ready
        // ones, so a ready column holds only what can really start.
        if self.split_blocked() {
            let any = (0..self.board.tickets.len()).any(|i| {
                project.is_none_or(|p| self.board.tickets[i].project == p) && self.waits_on_blockers(i)
            });
            if any {
                let at = cols.iter().position(|c| c.category == Category::Todo).unwrap_or(cols.len());
                cols.insert(at, Column { status: WAITING.into(), category: Category::Todo });
            }
        }
        cols
    }

    fn split_blocked(&self) -> bool {
        crate::settings::get().split_blocked
    }

    /// An unstarted ticket whose blockers are still open.
    pub fn waits_on_blockers(&self, i: usize) -> bool {
        self.board.tickets[i].category == Category::Todo && self.idx.blocked.get(i).copied().unwrap_or(false)
    }

    /// The column a ticket shows in.
    pub fn column_key(&self, i: usize) -> &str {
        if self.split_blocked() && self.waits_on_blockers(i) {
            WAITING
        } else {
            &self.board.tickets[i].status_key
        }
    }

    /// Visible tickets grouped by column (column order), card order inside.
    pub fn grouped(&self, cx: &App) -> Vec<(Column, Vec<usize>)> {
        let vis = self.visible(cx);
        self.columns()
            .into_iter()
            .map(|c| {
                let mut items: Vec<usize> = vis.iter().copied().filter(|&i| self.column_key(i) == c.status).collect();
                if c.status == WAITING {
                    // Closest to ready first.
                    items.sort_by_key(|&i| self.idx.open_blockers.get(i).map(Vec::len).unwrap_or(0));
                }
                (c, items)
            })
            .collect()
    }

    pub fn select_first_visible(&mut self, cx: &mut Context<Self>) {
        let g = self.grouped(cx);
        if let Some(&i) = g.iter().find_map(|(_, v)| v.first()) {
            self.selected = Some(self.board.tickets[i].path.clone());
            cx.notify();
        }
    }

    // ---------- keyboard navigation ----------

    /// (column index, row index) of the selection in the grouped view.
    fn selection_pos(&self, g: &[(Column, Vec<usize>)]) -> Option<(usize, usize)> {
        let sel = self.selected.as_ref()?;
        g.iter().enumerate().find_map(|(c, (_, v))| {
            v.iter()
                .position(|&i| &self.board.tickets[i].path == sel)
                .map(|r| (c, r))
        })
    }

    pub fn move_selection(&mut self, dc: i32, dr: i32, window: &mut Window, cx: &mut Context<Self>) {
        let g = self.grouped(cx);
        if g.iter().all(|(_, v)| v.is_empty()) {
            return;
        }
        let Some((c, r)) = self.selection_pos(&g) else {
            self.select_first_visible(cx);
            return;
        };
        if self.view.layout == store::Layout::List {
            // The list reads top to bottom across its groups.
            let flat: Vec<usize> = g
                .iter()
                .filter(|(col, _)| !self.view.collapsed.contains(&col.status))
                .flat_map(|(_, v)| v.iter().copied())
                .collect();
            let cur = self.board.tickets[g[c].1[r]].path.clone();
            let pos = flat.iter().position(|&i| self.board.tickets[i].path == cur).unwrap_or(0) as i32;
            let next = (pos + dr).clamp(0, flat.len().saturating_sub(1) as i32) as usize;
            if let Some(&i) = flat.get(next) {
                let path = self.board.tickets[i].path.clone();
                self.select(path, window, cx);
            }
            return;
        }
        let (mut nc, mut nr) = (c as i32, r as i32 + dr);
        if dc != 0 {
            // Next non-empty, non-collapsed column in that direction.
            loop {
                nc += dc;
                if nc < 0 || nc >= g.len() as i32 {
                    return;
                }
                let (col, v) = &g[nc as usize];
                if !v.is_empty() && !self.view.collapsed.contains(&col.status) {
                    break;
                }
            }
            nr = r.min(g[nc as usize].1.len() - 1) as i32;
        }
        let v = &g[nc as usize].1;
        let nr = nr.clamp(0, v.len() as i32 - 1) as usize;
        let path = self.board.tickets[v[nr]].path.clone();
        self.select(path, window, cx);
    }

    /// Selects a card; an open detail follows it (Linear's peek).
    pub fn select(&mut self, path: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        self.selected = Some(path.clone());
        if self.detail.as_ref().is_some_and(|d| !d.is_doc) {
            let full = self.detail.as_ref().is_some_and(|d| d.full);
            self.open_detail(path, full, window, cx);
        }
        cx.notify();
    }

    /// Opens a ticket as a new root (a card, the palette, j / k): the
    /// navigation stack starts over.
    pub fn open_detail(&mut self, path: PathBuf, full: bool, _window: &mut Window, cx: &mut Context<Self>) {
        if self.detail.as_ref().is_none_or(|d| d.path != path) {
            self.nav_back.clear();
            self.nav_fwd.clear();
        }
        self.show(path, false, full, cx);
    }

    /// Opens a project doc as a new root (the sidebar).
    pub fn open_doc(&mut self, path: PathBuf, _window: &mut Window, cx: &mut Context<Self>) {
        self.nav_back.clear();
        self.nav_fwd.clear();
        self.show(path, true, false, cx);
    }

    /// Follows a link from inside the detail (a relation, a spec, a body
    /// link): the current page goes on the back stack.
    pub fn navigate(&mut self, path: PathBuf, is_doc: bool, cx: &mut Context<Self>) {
        let full = self.detail.as_ref().is_some_and(|d| d.full);
        if let Some(d) = &self.detail
            && d.path != path
        {
            self.nav_back.push((d.path.clone(), d.is_doc));
            self.nav_fwd.clear();
        }
        self.show(path, is_doc, full, cx);
    }

    pub fn nav_back_step(&mut self, cx: &mut Context<Self>) {
        let Some((path, is_doc)) = self.nav_back.pop() else {
            return;
        };
        let full = self.detail.as_ref().is_some_and(|d| d.full);
        if let Some(d) = &self.detail {
            self.nav_fwd.push((d.path.clone(), d.is_doc));
        }
        self.show(path, is_doc, full, cx);
    }

    pub fn nav_forward_step(&mut self, cx: &mut Context<Self>) {
        let Some((path, is_doc)) = self.nav_fwd.pop() else {
            return;
        };
        let full = self.detail.as_ref().is_some_and(|d| d.full);
        if let Some(d) = &self.detail {
            self.nav_back.push((d.path.clone(), d.is_doc));
        }
        self.show(path, is_doc, full, cx);
    }

    /// Jumps to entry `i` of the back stack (a breadcrumb click).
    pub fn nav_jump(&mut self, i: usize, cx: &mut Context<Self>) {
        while self.nav_back.len() > i + 1 {
            self.nav_back_step(cx);
        }
        self.nav_back_step(cx);
    }

    fn show(&mut self, path: PathBuf, is_doc: bool, full: bool, cx: &mut Context<Self>) {
        if !is_doc {
            self.selected = Some(path.clone());
        }
        let same = self.detail.as_ref().is_some_and(|d| d.path == path && d.is_doc == is_doc);
        if same {
            if let Some(d) = &mut self.detail {
                d.full = full;
            }
        } else {
            self.detail = Some(Detail::open(path, is_doc, full, self, cx));
            self.refresh_conversations(cx);
        }
        cx.notify();
    }

    /// Who does a ticket: a working agent, then the people who moved its
    /// status or checked its sub-tasks (git).
    pub fn assignees(&self, i: usize) -> Option<Vec<String>> {
        let t = &self.board.tickets[i];
        let mut out: Vec<String> = Vec::new();
        if t.agent.as_ref().is_some_and(|a| a.state == crate::agents::RunState::Running) {
            out.push(crate::board::AGENT_NAME.to_string());
        }
        if let Some(g) = self.ages.get(&t.path) {
            out.extend(g.doers.iter().cloned());
        }
        Some(out)
    }

    pub fn is_folded(&self, key: &str) -> bool {
        self.view.folded.iter().any(|k| k == key)
    }

    pub fn toggle_fold(&mut self, key: &str) {
        if let Some(i) = self.view.folded.iter().position(|k| k == key) {
            self.view.folded.remove(i);
        } else {
            self.view.folded.push(key.to_string());
        }
        self.save_view();
    }

    /// A board filter changed: a full-screen detail steps aside so the
    /// board it filters is visible.
    pub fn show_board(&mut self) {
        self.view.page = crate::store::Page::Board;
        self.session = None;
        if self.detail.as_ref().is_some_and(|d| d.full) {
            self.detail = None;
        }
    }

    /// Short name of a page for the breadcrumb: the ticket key or the file name.
    pub fn page_label(&self, path: &Path) -> String {
        self.board
            .find_path(path)
            .map(|i| self.board.tickets[i].key.clone())
            .unwrap_or_else(|| path.file_name().unwrap_or_default().to_string_lossy().to_string())
    }

    /// A markdown link from the detail view: a ticket or doc opens inside,
    /// anything else goes to the system.
    pub fn follow_link(&mut self, base: &Path, url: &str, _window: &mut Window, cx: &mut Context<Self>) {
        if url.contains("://") || url.starts_with("mailto:") {
            cx.open_url(url);
            return;
        }
        let file = url.split('#').next().unwrap_or(url);
        let target = base.parent().unwrap_or(Path::new("/")).join(file);
        let target = std::fs::canonicalize(&target).unwrap_or(target);
        if let Some(ix) = self
            .board
            .tickets
            .iter()
            .position(|t| std::fs::canonicalize(&t.path).unwrap_or(t.path.clone()) == target)
        {
            let path = self.board.tickets[ix].path.clone();
            self.navigate(path, false, cx);
        } else if target.extension().is_some_and(|e| e == "md") && target.is_file() {
            self.navigate(target, true, cx);
        } else if target.exists() {
            cx.open_with_system(&target);
        } else {
            self.toast(false, format!("Not found: {file}"));
        }
    }

    // ---------- palette ----------

    pub fn open_palette(&mut self, mode: PaletteMode, window: &mut Window, cx: &mut Context<Self>) {
        let (items, runs) = crate::palette::rows(self, mode);
        let overlay = PaletteOverlay::open(mode, items, window, cx);
        self.palette_runs = runs;
        self.palette = Some(overlay);
        cx.notify();
    }

    pub fn close_palette(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.palette = None;
        self.palette_runs.clear();
        self.refocus(window, cx);
        cx.notify();
    }

    pub fn palette_confirm(&mut self, row: usize, window: &mut Window, cx: &mut Context<Self>) {
        let run = self.palette_runs.get(row).cloned();
        self.palette = None;
        self.palette_runs.clear();
        self.refocus(window, cx);
        if let Some(run) = run {
            run(self, window, cx);
        }
        cx.notify();
    }

    pub fn refocus(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.screen == Screen::Board {
            self.board_focus.focus(window, cx);
        } else {
            self.focus.focus(window, cx);
        }
    }

    /// Red traffic light on a board: back to the welcome screen. On the
    /// welcome screen the window closes (the dock icon reopens it).
    pub fn on_close_request(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if self.screen == Screen::Board {
            self.close_board(window, cx);
            return false;
        }
        true
    }

    fn on_escape(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.palette.is_some() {
            self.close_palette(window, cx);
        } else if self.search.read(cx).focus_handle(cx).is_focused(window) {
            self.board_focus.focus(window, cx);
        } else if self.detail.is_some() {
            // A ticket page or a peek: back to the board.
            self.detail = None;
            self.board_focus.focus(window, cx);
        } else if !self.query(cx).is_empty() {
            self.search.update(cx, |s, cx| s.set_value("", window, cx));
        }
        cx.notify();
    }

    // ---------- welcome screen ----------

    fn section_header(title: &str, muted: Hsla) -> impl IntoElement + use<> {
        div()
            .flex()
            .items_center()
            .gap(px(10.))
            .h(px(20.))
            .child(
                div()
                    .flex_none()
                    .text_size(zrem(10.))
                    .font_family(crate::settings::mono_font())
                    .text_color(muted)
                    .child(title.to_uppercase()),
            )
            .child(div().flex_1().h(px(1.)).bg(muted.opacity(0.12)))
    }

    #[allow(clippy::too_many_arguments)]
    fn welcome_line(
        id: impl Into<ElementId>,
        icon: impl Into<Icon>,
        label: String,
        detail: Option<String>,
        trailing: AnyElement,
        icon_color: Hsla,
        muted: Hsla,
        foreground: Hsla,
    ) -> Stateful<Div> {
        div()
            .id(id.into())
            .cursor_pointer()
            .flex()
            .items_center()
            .w_full()
            .h(px(24.))
            .mt(px(1.))
            .pl(px(5.))
            .pr(px(6.))
            .rounded(px(4.))
            .hover(|this| this.bg(muted.opacity(0.08)))
            .child(
                div()
                    .w(px(17.))
                    .flex_none()
                    .child(icon.into().size(zrem(12.)).text_color(icon_color)),
            )
            .child(
                div()
                    .flex_none()
                    .text_size(zrem(14.))
                    .font_family(crate::settings::ui_font())
                    .text_color(foreground)
                    .child(label),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .pl_2()
                    .truncate()
                    .text_size(zrem(12.))
                    .font_family(crate::settings::ui_font())
                    .text_color(muted.opacity(0.55))
                    .children(detail),
            )
            .child(div().flex_none().flex().items_center().child(trailing))
    }

    fn welcome_row(
        &self,
        id: &'static str,
        icon: impl Into<Icon>,
        label: &str,
        hint: &str,
        cx: &mut Context<Self>,
        on_click: impl Fn(&mut KuzgunApp, &mut Window, &mut Context<KuzgunApp>) + 'static,
    ) -> Stateful<Div> {
        let muted = cx.theme().muted_foreground;
        let fg = cx.theme().foreground;
        Self::welcome_line(
            id,
            icon,
            label.to_string(),
            None,
            crate::kbd::caps_sized(hint, zrem(12.)),
            muted,
            muted,
            fg,
        )
        .on_click(cx.listener(move |this, _, window, cx| on_click(this, window, cx)))
    }

    fn recent_row(&self, n: usize, b: &SavedBoard, cx: &mut Context<Self>) -> AnyElement {
        let (muted, fg) = (cx.theme().muted_foreground, cx.theme().foreground);
        let accent = cx.theme().accent;
        let path = b.path.clone();
        let mut detail = store::tilde(&b.path);
        if b.tickets > 0 {
            detail.push_str(&format!(" · {} tickets", b.tickets));
        }
        let trailing = if n < 5 {
            crate::kbd::caps_sized(&format!("cmd-{}", n + 1), zrem(12.))
        } else {
            div().into_any_element()
        };
        let missing = !b.path.is_dir();
        let open_path = path.clone();
        let menu_path = path.clone();
        let pinned = b.pinned;
        Self::welcome_line(
            ("recent-board", n),
            if b.pinned { IconName::Pin } else { IconName::SquareKanban },
            b.name.clone(),
            Some(if missing { format!("{detail} · missing") } else { detail }),
            trailing,
            if b.pinned { accent } else { muted },
            muted,
            if missing { muted } else { fg },
        )
        .on_click(cx.listener(move |this, _, window, cx| {
            this.open_board(open_path.clone(), window, cx);
        }))
        .context_menu(move |menu, _, _| {
            let p1 = menu_path.clone();
            let p2 = menu_path.clone();
            let p3 = menu_path.clone();
            let p4 = menu_path.clone();
            menu.item(PopupMenuItem::new("Open").on_click(move |_, window, cx| {
                let p = p1.clone();
                let view = cx.global::<KuzgunHandle>().0.clone();
                view.update(cx, |this, cx| this.open_board(p, window, cx));
            }))
            .item(
                PopupMenuItem::new(if pinned { "Unpin" } else { "Pin to Top" }).on_click(
                    move |_, _, cx| {
                        let p = p2.clone();
                        with_app(cx, |this, cx| {
                            if let Some(b) = this.saved.iter_mut().find(|b| b.path == p) {
                                b.pinned = !b.pinned;
                            }
                            store::save_boards(&this.saved);
                            this.saved = store::load_boards();
                            cx.notify();
                        });
                    },
                ),
            )
            .item(PopupMenuItem::new("Reveal in Finder").on_click(move |_, _, cx| {
                cx.reveal_path(&p3);
            }))
            .separator()
            .item(PopupMenuItem::new("Remove from List").on_click(move |_, _, cx| {
                let p = p4.clone();
                with_app(cx, |this, cx| {
                    this.saved.retain(|b| b.path != p);
                    store::save_boards(&this.saved);
                    cx.notify();
                });
            }))
        })
        .into_any_element()
    }

    fn render_welcome(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let muted = cx.theme().muted_foreground;
        let fg = cx.theme().foreground;
        let logo = std::sync::Arc::new(Image::from_bytes(ImageFormat::Png, crate::icons::LOGO.to_vec()));
        let mut col = div()
            .flex()
            .flex_col()
            .w(px(420.))
            .child(
                div()
                    .flex()
                    .justify_center()
                    .items_center()
                    .gap(px(14.))
                    .pb(px(25.))
                    .child(img(logo).size(px(56.)))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .child(
                                div()
                                    .text_size(zrem(20.))
                                    .line_height(zrem(26.))
                                    .text_color(fg)
                                    .child("Welcome back to Kuzgun"),
                            )
                            .child(
                                div()
                                    .text_size(zrem(12.))
                                    .line_height(zrem(18.))
                                    .italic()
                                    .text_color(muted)
                                    .child("A raven's-eye board for your mattpocock/skills tickets"),
                            ),
                    ),
            )
            .when_some(self.welcome_notice.clone(), |d, note| {
                let warn = cx.theme().warning;
                d.child(
                    div()
                        .flex()
                        .items_start()
                        .gap_2()
                        .mb_4()
                        .p_3()
                        .rounded(px(8.))
                        .border_1()
                        .border_color(warn.opacity(0.4))
                        .bg(warn.opacity(0.08))
                        .child(Icon::new(IconName::Info).size(px(16.)).text_color(warn).flex_none().mt(px(2.)))
                        .child(div().flex_1().min_w_0().text_sm().text_color(fg).child(note))
                        .child(
                            Button::new("welcome-notice-close")
                                .ghost()
                                .xsmall()
                                .icon(Icon::new(IconName::X).text_color(muted))
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.welcome_notice = None;
                                    cx.notify();
                                })),
                        ),
                )
            })
            .child(div().pb(px(4.)).child(Self::section_header("Get Started", muted)))
            .child(self.welcome_row(
                "welcome-open",
                IconName::FolderOpen,
                "Open Folder…",
                "cmd-o",
                cx,
                |this, w, cx| this.prompt_open_folder(w, cx),
            ))
            .child(self.welcome_row(
                "welcome-palette",
                IconName::Command,
                "Command Palette…",
                "cmd-shift-p",
                cx,
                |this, w, cx| this.open_palette(PaletteMode::Commands, w, cx),
            ))
            .child(self.welcome_row(
                "welcome-settings",
                IconName::Settings,
                "Settings…",
                "cmd-,",
                cx,
                |_, _, cx| crate::settings::SettingsWindow::open(cx),
            ));
        if !self.saved.is_empty() {
            col = col.child(
                div()
                    .pt(px(18.))
                    .pb(px(4.))
                    .child(Self::section_header("Recent Boards", muted)),
            );
            let saved = self.saved.clone();
            for (n, b) in saved.iter().enumerate().take(12) {
                col = col.child(self.recent_row(n, b, cx));
            }
        }
        col = col.child(
            div()
                .pt(px(18.))
                .text_size(zrem(12.))
                .text_color(muted.opacity(0.6))
                .child("Or drop a folder onto this window. Kuzgun reads the .scratch tracker that mattpocock/skills writes (to-tickets, triage, wayfinder) and follows the files live."),
        );
        div()
            .id("welcome-scroll")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .child(
                div()
                    .min_h_full()
                    .flex()
                    .flex_row()
                    .justify_center()
                    .items_center()
                    .py_8()
                    .child(col),
            )
    }

    // ---------- bars ----------

    fn render_title_bar(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let t = cx.theme();
        let (fg, muted, green) = (t.foreground, t.muted_foreground, t.green);
        if self.screen != Screen::Board {
            return TitleBar::new()
                .child(div().text_sm().text_color(muted).child("Kuzgun"))
                .child(div());
        }
        let root = self.root.clone().unwrap_or_default();
        let name = store::default_name(&root);
        let saved = self.saved.clone();
        let board_menu = Button::new("board-menu")
            .ghost()
            .small()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_1p5()
                    .text_sm()
                    .text_color(fg)
                    .child(Icon::new(IconName::SquareKanban).size(px(13.)).text_color(muted))
                    .child(name)
                    .child(Icon::new(IconName::ChevronDown).size(px(12.)).text_color(muted)),
            )
            .dropdown_menu(move |mut menu, _, _| {
                for b in saved.iter().take(9) {
                    let p = b.path.clone();
                    menu = menu.item(PopupMenuItem::new(b.name.clone()).on_click(move |_, window, cx| {
                        let p = p.clone();
                        let view = cx.global::<KuzgunHandle>().0.clone();
                        view.update(cx, |this, cx| this.open_board(p, window, cx));
                    }));
                }
                menu.separator()
                    .item(PopupMenuItem::new("Open Folder…").on_click(|_, window, cx| {
                        window.dispatch_action(Box::new(OpenFolder), cx);
                    }))
                    .item(PopupMenuItem::new("Reveal in Finder").on_click(|_, _, cx| {
                        let root = cx.global::<KuzgunHandle>().0.read(cx).root.clone();
                        if let Some(r) = root {
                            cx.reveal_path(&r);
                        }
                    }))
                    .item(PopupMenuItem::new("Close Board").on_click(|_, window, cx| {
                        window.dispatch_action(Box::new(CloseBoard), cx);
                    }))
            });
        let project = self
            .view
            .project
            .as_ref()
            .and_then(|n| self.board.projects.iter().find(|p| &p.name == n))
            .map(|p| p.title.clone());
        let projects: Vec<(String, String)> = self.board.projects.iter().map(|p| (p.name.clone(), p.title.clone())).collect();
        let project_menu = Button::new("project-menu")
            .ghost()
            .small()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_1p5()
                    .text_sm()
                    .text_color(fg)
                    .child(project.clone().unwrap_or_else(|| "All projects".into()))
                    .child(Icon::new(IconName::ChevronDown).size(px(12.)).text_color(muted)),
            )
            .dropdown_menu(move |mut menu, _, _| {
                menu = menu.item(PopupMenuItem::new("All projects").on_click(|_, _, cx| {
                    with_app(cx, |this, cx| {
                        this.view.project = None;
                        this.view.page = crate::store::Page::Board;
                        this.detail = None;
                        this.save_view();
                        cx.notify();
                    })
                }));
                menu = menu.separator();
                for (name, title) in &projects {
                    let name = name.clone();
                    menu = menu.item(PopupMenuItem::new(title.clone()).on_click(move |_, _, cx| {
                        let name = name.clone();
                        with_app(cx, |this, cx| {
                            this.view.project = Some(name);
                            this.detail = None;
                            this.save_view();
                            cx.notify();
                        })
                    }));
                }
                menu
            });
        let working = self
            .agent_runs
            .iter()
            .filter(|r| r.state == crate::agents::RunState::Running)
            .count();
        let agents_pill = (working > 0).then(|| {
            div()
                .id("agents-pill")
                .cursor_pointer()
                .hover(|d| d.bg(green.opacity(0.24)))
                .on_click(cx.listener(|this, _, w, cx| {
                    this.view.page = crate::store::Page::Agents;
                    this.detail = None;
                    this.save_view();
                    this.board_focus.focus(w, cx);
                    cx.notify();
                }))
                .flex()
                .items_center()
                .gap_1p5()
                .px_2()
                .h(px(22.))
                .rounded(px(11.))
                .bg(green.opacity(0.14))
                .text_xs()
                .text_color(green)
                .child(Icon::new(IconName::Bot).size(px(12.)))
                .child(format!("{working} agent{} working", if working == 1 { "" } else { "s" }))
        });
        TitleBar::new()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_0p5()
                    .child(
                        Button::new("crumb-root")
                            .ghost()
                            .small()
                            .child(img(std::sync::Arc::new(Image::from_bytes(ImageFormat::Png, crate::icons::LOGO.to_vec()))).size(px(18.)))
                            .tooltip("All boards")
                            .on_click(cx.listener(|this, _, w, cx| this.close_board(w, cx))),
                    )
                    .child(div().text_color(muted.opacity(0.5)).child("/"))
                    .child(board_menu)
                    .child(div().text_color(muted.opacity(0.5)).child("/"))
                    .child(project_menu),
            )
            .child(div().pr_2().flex().items_center().gap_2().children(agents_pill))
    }

    fn render_status_bar(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let t = cx.theme();
        let (muted, border, bg) = (t.muted_foreground, t.border, t.status_bar);
        let light = crate::settings::is_light(cx);
        let mut counts: Vec<(Category, usize)> = Vec::new();
        for c in Category::ALL {
            let n = self.board.tickets.iter().filter(|t| t.category == c).count();
            if n > 0 {
                counts.push((c, n));
            }
        }
        let shown = if self.screen == Screen::Board { self.visible(cx).len() } else { 0 };
        let changed = self.last_change.map(|(at, n)| {
            let ago = at.elapsed().map(|d| d.as_secs() as i64).unwrap_or(0);
            let what = match n {
                0 => "Folder touched".to_string(),
                1 => "1 ticket changed".to_string(),
                n => format!("{n} tickets changed"),
            };
            format!("{what} {}", ago_label(ago))
        });
        div()
            .flex()
            .flex_none()
            .items_center()
            .h(px(28.))
            .px_2()
            .gap_3()
            .border_t_1()
            .border_color(border)
            .bg(bg)
            .text_xs()
            .text_color(muted)
            .when(self.screen == Screen::Board, |d| {
                d.child(
                    Button::new("toggle-sidebar")
                        .ghost()
                        .xsmall()
                        .icon(Icon::new(IconName::PanelLeft).text_color(muted))
                        .tooltip("Toggle Sidebar ⌘B")
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.view.sidebar_hidden = !this.view.sidebar_hidden;
                            this.save_view();
                            cx.notify();
                        })),
                )
                .child(format!("{shown} of {} tickets", self.board.tickets.len()))
                .children(counts.into_iter().map(|(c, n)| {
                    div()
                        .id(SharedString::from(format!("count-{}", c.label())))
                        .tooltip({
                            let tip = format!("{}: {n}", c.label());
                            move |window, cx| gpui_kit::component::tooltip::Tooltip::new(tip.clone()).max_w(px(360.)).build(window, cx)
                        })
                        .flex()
                        .items_center()
                        .gap_1()
                        .child(
                            crate::icons::status_icon(c)
                                .size(px(11.))
                                .text_color(crate::icons::status_color(c, light)),
                        )
                        .child(format!("{n}"))
                }))
                .when(!self.board.errors.is_empty(), |d| {
                    let n = self.board.errors.len();
                    d.child(
                        div()
                            .text_color(cx.theme().red)
                            .child(format!("{n} file{} unreadable", if n == 1 { "" } else { "s" })),
                    )
                })
                .child(div().flex_1())
                .children(changed)
                .child(
                    div()
                        .font_family(crate::settings::mono_font())
                        .child(self.root.as_deref().map(store::tilde).unwrap_or_default()),
                )
            })
            .when(self.screen != Screen::Board, |d| {
                d.child(div().flex_1()).child(format!("Kuzgun {}", env!("CARGO_PKG_VERSION")))
            })
    }

    fn on_external_drop(&mut self, paths: &ExternalPaths, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(p) = paths.paths().first() {
            let dir = if p.is_dir() {
                p.clone()
            } else {
                p.parent().map(Path::to_path_buf).unwrap_or_default()
            };
            self.open_board(dir, window, cx);
        }
    }
}

/// The path from the enclosing git repo's root (else from the home folder).
pub fn repo_relative(path: &Path) -> String {
    let mut dir = path.parent();
    while let Some(d) = dir {
        if d.join(".git").exists() {
            return path.strip_prefix(d).map(|p| p.display().to_string()).unwrap_or_default();
        }
        dir = d.parent();
    }
    store::tilde(path)
}

/// What looks wrong with each ticket, for the Needs attention view.
fn attention_of(b: &Board, _blocked: &[bool]) -> Vec<Vec<String>> {
    let n = b.tickets.len();
    let mut out: Vec<Vec<String>> = vec![Vec::new(); n];
    let now = now_unix();
    // Blocker cycles: a ticket that reaches itself through blocked_by.
    for (i, reasons) in out.iter_mut().enumerate() {
        let mut stack: Vec<usize> = b.tickets[i].blocked_by.clone();
        let mut seen = vec![false; n];
        while let Some(j) = stack.pop() {
            if j == i {
                reasons.push("Blocked by a cycle: it waits on itself through its blockers".into());
                break;
            }
            if !seen[j] {
                seen[j] = true;
                stack.extend(b.tickets[j].blocked_by.iter().copied());
            }
        }
    }
    for (i, t) in b.tickets.iter().enumerate() {
        let r = &mut out[i];
        if let Some(why) = &t.inferred {
            r.push(format!("The file says \"{}\" but {}", t.status, why.to_lowercase()));
        }
        let dangling = t
            .blocked_refs
            .iter()
            .filter(|rf| {
                let doc = rf.path.as_ref().is_some_and(|p| p.is_file() && b.find_path(p).is_none());
                !doc && rf.hit.is_none() && rf.project.is_none()
            })
            .count();
        if dangling > 0 {
            r.push(format!("{dangling} blocker{} not found on this board", if dangling == 1 { " is" } else { "s are" }));
        }
        let project = &b.projects[t.project];
        if project.map.is_some() && model::normalize(&t.status) == "resolved" && !t.raw.contains("\n## Answer") {
            r.push("Resolved without an ## Answer section".into());
        }
        if t.category == Category::InProgress && t.agent.is_none() {
            let since = t
                .modified
                .and_then(|m| m.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_secs() as i64)
                .unwrap_or(now);
            if now - since > 86_400 {
                r.push(format!("In progress for {} with no agent working on it", ago_label(now - since).trim_end_matches(" ago")));
            }
        }
        if t.category.is_closed() {
            let open_blockers = t.blocked_by.iter().filter(|&&j| !b.tickets[j].category.is_closed()).count();
            if open_blockers > 0 {
                r.push(format!("Closed while {open_blockers} of its blockers are still open"));
            }
        }
    }
    out
}

/// `42` → `42s ago`, `3700` → `1h ago`.
pub fn ago_label(secs: i64) -> String {
    let s = secs.max(0);
    match s {
        0..=4 => "just now".into(),
        5..=59 => format!("{s}s ago"),
        60..=3599 => format!("{}m ago", s / 60),
        3600..=86_399 => format!("{}h ago", s / 3600),
        86_400..=2_591_999 => format!("{}d ago", s / 86_400),
        _ => format!("{}mo ago", s / 2_592_000),
    }
}

pub fn ago_since(t: Option<SystemTime>) -> String {
    t.and_then(|t| t.elapsed().ok())
        .map(|d| ago_label(d.as_secs() as i64))
        .unwrap_or_default()
}

pub fn now_unix() -> i64 {
    chrono::Utc::now().timestamp()
}

/// Sizes in the UI font's rem (the kit Root sets rem = UI font size).
pub fn zrem(px16: f32) -> Pixels {
    px(px16 / 16. * crate::settings::get().ui_font_size)
}

impl Render for KuzgunApp {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        for (ok, msg) in std::mem::take(&mut self.toasts) {
            crate::toast::push(window, cx, ok, msg);
        }
        let t = cx.theme();
        let (bg, fg) = (t.background, t.foreground);
        let content: AnyElement = match self.screen {
            Screen::Welcome => self.render_welcome(cx).into_any_element(),
            Screen::Board => self.render_board(window, cx).into_any_element(),
        };
        div()
            .id("kuzgun-app")
            .key_context(APP)
            .track_focus(&self.focus)
            .size_full()
            .flex()
            .flex_col()
            .bg(bg)
            .text_color(fg)
            .font_family(crate::settings::ui_font())
            .on_mouse_move(cx.listener(|this, e: &MouseMoveEvent, window, cx| {
                if let Some((x0, w0)) = this.detail_drag {
                    let max = (f32::from(window.viewport_size().width) - 320.).max(360.);
                    let w = (w0 - (f32::from(e.position.x) - x0)).clamp(360., max);
                    this.view.detail_w = Some(w);
                    cx.notify();
                }
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _: &MouseUpEvent, _, _| {
                    if this.detail_drag.take().is_some() {
                        this.save_view();
                    }
                }),
            )
            .on_drop(cx.listener(Self::on_external_drop))
            .on_action(cx.listener(|this, _: &TogglePalette, w, cx| {
                if this.palette.is_some() {
                    this.close_palette(w, cx);
                } else {
                    this.open_palette(PaletteMode::Commands, w, cx);
                }
            }))
            .on_action(cx.listener(|this, _: &QuickOpenTicket, w, cx| {
                if this.screen == Screen::Board {
                    this.open_palette(PaletteMode::Tickets, w, cx);
                } else {
                    this.open_palette(PaletteMode::Boards, w, cx);
                }
            }))
            .on_action(cx.listener(|this, _: &OpenFolder, w, cx| this.prompt_open_folder(w, cx)))
            .on_action(cx.listener(|this, _: &OpenRecent1, w, cx| this.open_recent(0, w, cx)))
            .on_action(cx.listener(|this, _: &OpenRecent2, w, cx| this.open_recent(1, w, cx)))
            .on_action(cx.listener(|this, _: &OpenRecent3, w, cx| this.open_recent(2, w, cx)))
            .on_action(cx.listener(|this, _: &OpenRecent4, w, cx| this.open_recent(3, w, cx)))
            .on_action(cx.listener(|this, _: &OpenRecent5, w, cx| this.open_recent(4, w, cx)))
            .on_action(cx.listener(|this, _: &CloseBoard, w, cx| this.close_board(w, cx)))
            .on_action(cx.listener(|this, _: &Reload, _, cx| {
                this.load(Vec::new(), cx);
                this.toast(None, "Reloaded");
            }))
            .on_action(cx.listener(|this, _: &CloseOverlay, w, cx| this.on_escape(w, cx)))
            .on_action(cx.listener(|this, _: &FocusSearch, w, cx| {
                if this.screen == Screen::Board {
                    this.search.update(cx, |s, cx| s.focus(w, cx));
                }
            }))
            .on_action(cx.listener(|this, _: &ToggleSidebar, _, cx| {
                this.view.sidebar_hidden = !this.view.sidebar_hidden;
                this.save_view();
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &ToggleSwimlanes, _, cx| {
                this.view.swimlanes = !this.view.swimlanes;
                this.save_view();
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &ToggleHideClosed, _, cx| {
                this.view.completed = if this.view.completed == store::Completed::None {
                    store::Completed::All
                } else {
                    store::Completed::None
                };
                this.save_view();
                cx.notify();
            }))
            .on_action(cx.listener(|_, _: &ToggleEmptyColumns, _, cx| {
                crate::settings::update(cx, |p| p.show_empty_columns = !p.show_empty_columns);
            }))
            .on_action(cx.listener(|this, _: &OpenInEditor, _, cx| {
                if let Some(p) = this.current_path() {
                    this.open_in_editor(&p, cx);
                }
            }))
            .on_action(cx.listener(|this, _: &RevealInFinder, _, cx| {
                if let Some(p) = this.current_path() {
                    cx.reveal_path(&p);
                }
            }))
            .on_action(cx.listener(|this, _: &CopyId, _, cx| this.copy_current("ID", cx)))
            .on_action(cx.listener(|this, _: &CopyTitle, _, cx| this.copy_current("title", cx)))
            .on_action(cx.listener(|this, _: &CopyPath, _, cx| this.copy_current("path", cx)))
            .on_action(cx.listener(|this, _: &FilterAll, _, cx| {
                this.quick = Quick::All;
                this.show_board();
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &NavBack, _, cx| this.nav_back_step(cx)))
            .on_action(cx.listener(|this, _: &NavForward, _, cx| this.nav_forward_step(cx)))
            .on_action(cx.listener(|this, _: &FilterAgent, _, cx| {
                this.quick = Quick::Agent;
                this.show_board();
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &CopyImplement, _, cx| {
                if let Some(p) = this.current_path() {
                    this.copy_next(&p, cx);
                }
            }))
            .on_action(cx.listener(|this, _: &CopyTriage, _, cx| {
                if let Some(p) = this.current_path() {
                    this.copy_command("triage", &p, cx);
                }
            }))
            .on_action(cx.listener(|this, _: &CopyWayfinder, _, cx| {
                if let Some(p) = this.current_path() {
                    this.copy_command("wayfinder", &p, cx);
                }
            }))
            .on_action(cx.listener(|this, _: &FilterFrontier, _, cx| {
                this.quick = Quick::Frontier;
                this.show_board();
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &FilterBlocked, _, cx| {
                this.quick = Quick::Blocked;
                this.show_board();
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &FilterHuman, _, cx| {
                this.quick = Quick::Human;
                this.show_board();
                cx.notify();
            }))
            .child(self.render_title_bar(cx))
            .child(div().flex_1().min_h_0().flex().flex_col().child(content))
            .child(self.render_status_bar(cx))
            .when(self.palette.is_some(), |d| d.child(self.render_palette(cx)))
    }
}
