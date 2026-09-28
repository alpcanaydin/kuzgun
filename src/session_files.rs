//! The Files tab of the session page: the tree at that moment with the
//! changed files marked, and the picked file read-only, as its diff or its
//! full text. Git and file reads run off the main thread.

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;
use std::time::Instant;

use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Editor, EditorState};
use gpui_kit::component::list::ListItem;
use gpui_kit::component::theme::ActiveTheme as _;
use gpui_kit::component::tree::{TreeItem, TreeState, tree};
use gpui_kit::component::{Icon, Sizable as _};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::agents::RunState;
use crate::app::{KuzgunApp, with_app};
use crate::files::{self, Snapshot};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tab {
    Session,
    Files,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ViewMode {
    /// The file's diff.
    Changes,
    /// The file's full text.
    File,
}

/// The Files tab state of the session page.
pub struct FilesView {
    pub snap: Option<Arc<Snapshot>>,
    /// The run the snapshot belongs to.
    pub run: Option<std::path::PathBuf>,
    pub loaded_at: Option<Instant>,
    pub tree: Option<Entity<TreeState>>,
    pub changed_only: bool,
    pub selected: Option<String>,
    pub mode: ViewMode,
    /// The text on screen: (path, mode, text).
    pub text: Option<(String, ViewMode, String)>,
    pub editor: Option<(String, ViewMode, Entity<EditorState>)>,
    _snap_task: Option<Task<()>>,
    _text_task: Option<Task<()>>,
}

impl Default for FilesView {
    fn default() -> Self {
        Self {
            snap: None,
            run: None,
            loaded_at: None,
            tree: None,
            changed_only: false,
            selected: None,
            mode: ViewMode::Changes,
            text: None,
            editor: None,
            _snap_task: None,
            _text_task: None,
        }
    }
}

/// A folder of the tree while it is built.
#[derive(Default)]
struct Node {
    dirs: BTreeMap<String, Node>,
    files: Vec<String>,
}

fn build_tree(snap: &Snapshot, changed_only: bool) -> Vec<TreeItem> {
    let mut root = Node::default();
    let paths: Vec<&String> = if changed_only { snap.changes.iter().map(|c| &c.path).collect() } else { snap.files.iter().collect() };
    for p in paths {
        let mut node = &mut root;
        let parts: Vec<&str> = p.split('/').collect();
        for d in &parts[..parts.len().saturating_sub(1)] {
            node = node.dirs.entry(d.to_string()).or_default();
        }
        node.files.push(p.clone());
    }
    fn items(node: &Node, prefix: &str, snap: &Snapshot, open_all: bool) -> Vec<TreeItem> {
        let mut out = Vec::new();
        for (name, child) in &node.dirs {
            let path = if prefix.is_empty() { name.clone() } else { format!("{prefix}/{name}") };
            let dir = format!("{path}/");
            let changed = snap.changes.iter().any(|c| c.path.starts_with(&dir));
            out.push(TreeItem::new(format!("d:{path}"), name.clone()).expanded(open_all || changed).children(items(child, &path, snap, open_all)));
        }
        for f in &node.files {
            let name = f.rsplit('/').next().unwrap_or(f).to_string();
            out.push(TreeItem::new(format!("f:{f}"), name));
        }
        out
    }
    items(&root, "", snap, changed_only)
}

impl KuzgunApp {
    /// Reads the tree and the changes of the run on the session page.
    pub fn load_files(&mut self, cx: &mut Context<Self>) {
        let Some(s) = self.session.as_ref() else {
            return;
        };
        let Some(ix) = self.board.find_path(&s.ticket) else {
            return;
        };
        let runs = self.runs_of(ix);
        let Some(run) = runs.iter().find(|r| r.transcript == s.run).or(runs.first()).cloned() else {
            return;
        };
        let Some(repo) = self.root.as_deref().and_then(crate::agents::repo_root) else {
            return;
        };
        let entries = self.conversations.get(&run.transcript).map(|c| c.transcript.entries.clone()).unwrap_or_default();
        let inputs = files::inputs(&repo, run.worktree.clone(), run.state == RunState::Running, (run.started, run.last_activity), &entries);
        let run_path = run.transcript.clone();
        let task = cx.spawn(async move |this, cx| {
            let snap = cx.background_spawn(async move { files::snapshot(&inputs) }).await;
            let _ = this.update(cx, |this, cx| {
                let Some(s) = this.session.as_mut() else {
                    return;
                };
                if s.run != run_path {
                    return;
                }
                let first = s.files.snap.is_none();
                s.files.snap = Some(Arc::new(snap));
                s.files.run = Some(run_path.clone());
                s.files.loaded_at = Some(Instant::now());
                this.rebuild_tree(cx);
                // The first load opens the first changed file.
                let pick = this.session.as_ref().and_then(|s| {
                    let snap = s.files.snap.as_ref()?;
                    match &s.files.selected {
                        Some(p) if snap.files.contains(p) || snap.change(p).is_some() => Some(p.clone()),
                        _ if first => snap.changes.first().map(|c| c.path.clone()),
                        _ => None,
                    }
                });
                if let Some(p) = pick {
                    this.select_file(p, false, cx);
                }
                cx.notify();
            });
        });
        if let Some(s) = self.session.as_mut() {
            s.files._snap_task = Some(task);
        }
    }

    fn rebuild_tree(&mut self, cx: &mut Context<Self>) {
        let Some(s) = self.session.as_mut() else {
            return;
        };
        let Some(snap) = s.files.snap.clone() else {
            return;
        };
        let items = build_tree(&snap, s.files.changed_only);
        match &s.files.tree {
            Some(t) => t.update(cx, |t, cx| t.set_items(items, cx)),
            None => s.files.tree = Some(cx.new(|cx| TreeState::new(cx).items(items))),
        }
    }

    /// Opens a file of the run: its diff or its text, read off the main
    /// thread. `force` reloads the text even when it is on screen.
    pub fn select_file(&mut self, path: String, force: bool, cx: &mut Context<Self>) {
        let Some(s) = self.session.as_mut() else {
            return;
        };
        s.tab = Tab::Files;
        let mode = s.files.mode;
        let same = s.files.selected.as_ref() == Some(&path) && s.files.text.as_ref().is_some_and(|(p, m, _)| p == &path && *m == mode);
        s.files.selected = Some(path.clone());
        if same && !force {
            cx.notify();
            return;
        }
        let Some(snap) = s.files.snap.clone() else {
            cx.notify();
            return;
        };
        let task = cx.spawn(async move |this, cx| {
            let p = path.clone();
            let text = cx
                .background_spawn(async move {
                    match mode {
                        ViewMode::Changes => files::diff(&snap, &p),
                        ViewMode::File => files::content(&snap, &p),
                    }
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                if let Some(s) = this.session.as_mut()
                    && s.files.selected.as_ref() == Some(&path)
                    && s.files.mode == mode
                {
                    let unchanged = s.files.text.as_ref().is_some_and(|(p, m, t)| p == &path && *m == mode && t == &text);
                    if !unchanged {
                        s.files.text = Some((path.clone(), mode, text));
                        s.files.editor = None;
                    }
                    cx.notify();
                }
            });
        });
        if let Some(s) = self.session.as_mut() {
            s.files._text_task = Some(task);
        }
        cx.notify();
    }

    /// While the agent works, the tree and the open file follow it.
    pub fn refresh_files(&mut self, cx: &mut Context<Self>) {
        let Some(s) = self.session.as_ref() else {
            return;
        };
        if s.tab != Tab::Files {
            return;
        }
        let Some(ix) = self.board.find_path(&s.ticket) else {
            return;
        };
        let running = self.runs_of(ix).iter().any(|r| r.transcript == s.run && r.state == RunState::Running);
        let stale = s.files.loaded_at.is_none_or(|t| t.elapsed().as_secs() >= 4);
        if running && stale {
            self.load_files(cx);
            if let Some(p) = self.session.as_ref().and_then(|s| s.files.selected.clone()) {
                self.select_file(p, true, cx);
            }
        }
    }

    pub fn render_files(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme().clone();
        let (muted, fg, border) = (theme.muted_foreground, theme.foreground, theme.border);
        let Some(s) = self.session.as_mut() else {
            return div().into_any_element();
        };
        let Some(snap) = s.files.snap.clone() else {
            return div()
                .flex_1()
                .flex()
                .items_center()
                .justify_center()
                .text_sm()
                .text_color(muted)
                .child("Reading the files…")
                .into_any_element();
        };
        // The editor needs the window: build it once its text arrives.
        if let Some((path, mode, text)) = s.files.text.clone()
            && s.files.editor.as_ref().is_none_or(|(p, m, _)| p != &path || *m != mode)
        {
            let lang = match mode {
                ViewMode::Changes => "diff".to_string(),
                ViewMode::File => files::language(&path),
            };
            let state = cx.new(|cx| EditorState::new(window, cx).language(lang).line_number(true).default_value(text));
            s.files.editor = Some((path, mode, state));
        }
        let status: Arc<HashMap<String, (char, usize, usize)>> =
            Arc::new(snap.changes.iter().map(|c| (c.path.clone(), (c.status, c.added, c.removed))).collect());
        let (green, red, yellow) = (theme.green, theme.red, theme.yellow);
        let color_of = move |c: char| match c {
            'A' => green,
            'D' => red,
            _ => yellow,
        };

        // ---- left: the tree ----
        let changed_only = s.files.changed_only;
        let n_changed = snap.changes.len();
        let tree_el = s.files.tree.clone().map(|state| {
            let status = status.clone();
            tree(&state, move |ix, entry, _selected, _window, _cx| {
                let id = entry.item().id.to_string();
                let folder = entry.is_folder();
                let path = id[2..].to_string();
                let mark = if folder {
                    let dir = format!("{path}/");
                    status.keys().any(|k| k.starts_with(&dir)).then_some('•')
                } else {
                    status.get(&path).map(|(c, _, _)| *c)
                };
                let tint = mark.map(|c| if c == '•' { muted } else { color_of(c) });
                let label = entry.item().label.clone();
                let pick = path.clone();
                ListItem::new(ix)
                    .h(px(26.))
                    .pl(px(8. + entry.depth() as f32 * 14.))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_1p5()
                            .w_full()
                            .min_w_0()
                            .text_sm()
                            .child(
                                Icon::new(if folder {
                                    if entry.is_expanded() { IconName::FolderOpen } else { IconName::Folder }
                                } else {
                                    IconName::FileText
                                })
                                .size(px(14.))
                                .text_color(muted),
                            )
                            .child(div().flex_1().min_w_0().truncate().text_color(tint.filter(|_| !folder).unwrap_or(fg)).child(label))
                            .when_some(mark.filter(|c| *c != '•'), |d, c| {
                                d.child(div().flex_none().text_xs().font_family(crate::settings::mono_font()).text_color(color_of(c)).child(c.to_string()))
                            })
                            .when(mark == Some('•'), |d| d.child(div().flex_none().size(px(5.)).rounded_full().bg(muted.opacity(0.7)))),
                    )
                    .when(!folder, |item| {
                        item.on_click(move |_, _, cx| {
                            let p = pick.clone();
                            with_app(cx, |this, cx| this.select_file(p, false, cx));
                        })
                    })
            })
            .flex_1()
            .into_any_element()
        });
        let left = div()
            .w(px(300.))
            .flex_none()
            .flex()
            .flex_col()
            .border_r_1()
            .border_color(border)
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .px_3()
                    .py_2()
                    .border_b_1()
                    .border_color(border)
                    .child(div().text_xs().text_color(muted).truncate().child(snap.source.label()))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .text_sm()
                            .child(div().flex_1().text_color(fg).child(format!("{n_changed} changed · {} files", snap.files.len())))
                            .child(
                                Button::new("files-changed-only")
                                    .xsmall()
                                    .when(changed_only, |b| b.primary())
                                    .label("Changed only")
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        if let Some(s) = this.session.as_mut() {
                                            s.files.changed_only = !s.files.changed_only;
                                        }
                                        this.rebuild_tree(cx);
                                        cx.notify();
                                    })),
                            ),
                    ),
            )
            .children(tree_el);

        // ---- right: the file ----
        let selected = s.files.selected.clone();
        let mode = s.files.mode;
        let editor = s.files.editor.clone();
        let changed: Vec<String> = snap.changes.iter().map(|c| c.path.clone()).collect();
        let right = match selected {
            None => div()
                .flex_1()
                .flex()
                .items_center()
                .justify_center()
                .text_sm()
                .text_color(muted)
                .child("Pick a file on the left.")
                .into_any_element(),
            Some(path) => {
                let change = snap.change(&path).cloned();
                let pos = changed.iter().position(|p| p == &path);
                let prev = pos.and_then(|i| i.checked_sub(1)).and_then(|i| changed.get(i).cloned()).or_else(|| changed.last().cloned());
                let next = pos.map(|i| i + 1).and_then(|i| changed.get(i).cloned()).or_else(|| changed.first().cloned());
                let open_path = self.session_file_on_disk(&path);
                let seg = |id: &'static str, label: &'static str, m: ViewMode| {
                    Button::new(id)
                        .xsmall()
                        .when(mode == m, |b| b.primary())
                        .when(mode != m, |b| b.ghost())
                        .label(label)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            let p = this.session.as_mut().and_then(|s| {
                                s.files.mode = m;
                                s.files.selected.clone()
                            });
                            if let Some(p) = p {
                                this.select_file(p, true, cx);
                            }
                        }))
                };
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .child(
                        div()
                            .flex()
                            .flex_none()
                            .items_center()
                            .gap_2()
                            .h(px(44.))
                            .px_3()
                            .border_b_1()
                            .border_color(border)
                            .child(div().flex_1().min_w_0().truncate().text_sm().font_family(crate::settings::mono_font()).text_color(fg).child(path.clone()))
                            .when_some(change, |d, c| {
                                d.child(div().text_xs().font_family(crate::settings::mono_font()).text_color(color_of(c.status)).child(c.status.to_string()))
                                    .child(div().text_xs().text_color(theme.green).child(format!("+{}", c.added)))
                                    .child(div().text_xs().text_color(theme.red).child(format!("−{}", c.removed)))
                            })
                            .child(seg("files-mode-changes", "Changes", ViewMode::Changes))
                            .child(seg("files-mode-file", "File", ViewMode::File))
                            .when(!changed.is_empty(), |d| {
                                d.child(
                                    Button::new("files-prev")
                                        .xsmall()
                                        .ghost()
                                        .icon(Icon::new(IconName::ChevronUp))
                                        .tooltip("Previous changed file")
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            if let Some(p) = prev.clone() {
                                                this.select_file(p, false, cx);
                                            }
                                        })),
                                )
                                .child(
                                    Button::new("files-next")
                                        .xsmall()
                                        .ghost()
                                        .icon(Icon::new(IconName::ChevronDown))
                                        .tooltip("Next changed file")
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            if let Some(p) = next.clone() {
                                                this.select_file(p, false, cx);
                                            }
                                        })),
                                )
                            })
                            .when_some(open_path, |d, p| {
                                d.child(
                                    Button::new("files-open")
                                        .xsmall()
                                        .ghost()
                                        .icon(Icon::new(IconName::SquarePen))
                                        .tooltip("Open in the editor")
                                        .on_click(cx.listener(move |this, _, _, cx| this.open_in_editor(&p, cx))),
                                )
                            }),
                    )
                    .child(match editor {
                        Some((p, m, state)) if p == path && m == mode => div()
                            .flex_1()
                            .min_h_0()
                            .child(Editor::new(&state).readonly(true).bordered(false).h(relative(1.)))
                            .into_any_element(),
                        _ => div().flex_1().p_4().text_sm().text_color(muted).child("Reading…").into_any_element(),
                    })
                    .into_any_element()
            }
        };
        div().flex_1().min_h_0().flex().child(left).child(right).into_any_element()
    }

    /// The file on disk, when the snapshot reads a working tree.
    fn session_file_on_disk(&self, path: &str) -> Option<std::path::PathBuf> {
        let snap = self.session.as_ref()?.files.snap.as_ref()?;
        let dir = match &snap.source {
            files::Source::Tree { dir, .. } => dir.clone(),
            files::Source::Edits { repo } | files::Source::Commits { repo, .. } | files::Source::Landed { repo, .. } => repo.clone(),
        };
        let p = dir.join(path);
        p.is_file().then_some(p)
    }
}
