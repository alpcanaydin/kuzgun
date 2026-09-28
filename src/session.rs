//! The agent session page: one ticket's agent runs as a readable story.
//! The prompt sits in a bubble, replies read as documents, and tool calls
//! gather into "Worked for 4m · 23 steps" blocks written as plain verbs
//! ("Read app.rs", "Ran cargo test"). A running agent streams in and the
//! page follows it to the bottom.

use std::collections::HashSet;
use std::path::PathBuf;

use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::shimmer::ShimmerText;
use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::text::TextView;
use gpui_kit::component::theme::ActiveTheme as _;
use gpui_kit::component::{Icon, Sizable as _};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::agents::{AgentRun, RunState};
use crate::app::{KuzgunApp, ago_label, now_unix, repo_relative};
use crate::transcript::{Entry, Kind, Provider};

/// The open session page.
pub struct SessionView {
    /// The ticket file.
    pub ticket: PathBuf,
    /// The transcript of the run on screen.
    pub run: PathBuf,
    /// Work blocks and steps the person opened (by entry index).
    pub open: HashSet<usize>,
    /// Keep the newest step in view.
    pub follow: bool,
    pub scroll: ScrollHandle,
    /// Entries seen at the last render, to follow new ones.
    pub seen: usize,
    /// The list of background commands is open.
    pub bg_open: bool,
    /// Background commands whose output is open, by id.
    pub bg_shown: HashSet<String>,
}

/// A row of the story.
enum Item {
    Prompt(usize),
    Reply(usize),
    /// Tool calls and thinking between two messages: entry range.
    Work(usize, usize),
}

fn items(entries: &[Entry]) -> Vec<Item> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < entries.len() {
        match entries[i].kind {
            Kind::User => {
                out.push(Item::Prompt(i));
                i += 1;
            }
            Kind::Assistant => {
                out.push(Item::Reply(i));
                i += 1;
            }
            Kind::Tool | Kind::Thinking => {
                let from = i;
                while i < entries.len() && matches!(entries[i].kind, Kind::Tool | Kind::Thinking) {
                    i += 1;
                }
                out.push(Item::Work(from, i));
            }
        }
    }
    out
}

/// A tool call as a verb, an icon and its target.
fn step(e: &Entry) -> (IconName, &'static str, String) {
    let name = e.tool.as_deref().unwrap_or("tool");
    let target = e.text.clone();
    let file = || {
        std::path::Path::new(&target)
            .file_name()
            .map(|f| f.to_string_lossy().to_string())
            .filter(|_| target.contains('/'))
            .unwrap_or_else(|| target.clone())
    };
    let lower = name.to_lowercase();
    match lower.as_str() {
        "read" | "notebookread" | "view" => (IconName::FileText, "Read", file()),
        "edit" | "multiedit" | "write" | "notebookedit" | "apply_patch" | "str_replace_based_edit_tool" => {
            (IconName::FilePen, "Edited", file())
        }
        "bash" | "shell" | "exec" | "exec_command" | "local_shell" | "bashoutput" => (IconName::Terminal, "Ran", target),
        "grep" | "glob" | "ls" => (IconName::Search, "Searched", target),
        "websearch" | "web_search" => (IconName::Search, "Searched the web for", target),
        "webfetch" | "web_fetch" => (IconName::Link, "Read", target),
        "agent" | "task" => (IconName::Bot, "Started a subagent:", target),
        "todowrite" | "update_plan" => (IconName::ListChecks, "Updated the plan", String::new()),
        "skill" => (IconName::Sparkles, "Used the skill", target),
        "notice" => (IconName::Bell, "", target),
        _ if lower.starts_with("mcp__") => {
            let short = name.rsplit("__").next().unwrap_or(name).replace('_', " ");
            (IconName::Settings2, "Called", format!("{short} {target}").trim().to_string())
        }
        _ => (IconName::Settings2, "Used", format!("{name} {target}").trim().to_string()),
    }
}

fn span(secs: i64) -> String {
    let s = secs.max(0);
    match s {
        0..=59 => format!("{s}s"),
        60..=3599 => format!("{}m {}s", s / 60, s % 60),
        _ => format!("{}h {}m", s / 3600, (s % 3600) / 60),
    }
}

/// The session id and the command that resumes the run in a terminal.
pub fn resume_command(run: &AgentRun) -> Option<String> {
    let stem = run.transcript.file_stem()?.to_string_lossy().to_string();
    Some(match run.provider {
        Provider::Claude => format!("claude --resume {stem}"),
        // `rollout-<date>-<uuid>`: the last five dash groups are the id.
        Provider::Codex => {
            let parts: Vec<&str> = stem.split('-').collect();
            let id = parts[parts.len().saturating_sub(5)..].join("-");
            format!("codex resume {id}")
        }
    })
}

impl KuzgunApp {
    /// Opens the session page of a ticket, on its running (else latest) run.
    pub fn open_session(&mut self, ticket: PathBuf, cx: &mut Context<Self>) {
        let Some(ix) = self.board.find_path(&ticket) else {
            return;
        };
        let Some(run) = self.runs_of(ix).into_iter().next() else {
            self.toast(None, "No agent session found for this ticket.");
            return;
        };
        let follow = run.state == RunState::Running;
        self.session = Some(SessionView {
            ticket,
            run: run.transcript,
            open: HashSet::new(),
            follow,
            scroll: ScrollHandle::new(),
            seen: 0,
            bg_open: false,
            bg_shown: HashSet::new(),
        });
        self.refresh_conversations(cx);
        cx.notify();
    }

    pub fn render_session(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let Some(s) = self.session.as_ref() else {
            return div().into_any_element();
        };
        let Some(ix) = self.board.find_path(&s.ticket) else {
            return div().into_any_element();
        };
        let theme = cx.theme().clone();
        let (muted, fg, border, accent) = (theme.muted_foreground, theme.foreground, theme.border, theme.accent);
        let now = now_unix();
        let tk = &self.board.tickets[ix];
        let runs = self.runs_of(ix);
        let run = runs.iter().find(|r| r.transcript == s.run).or(runs.first()).cloned();
        let entries: &[Entry] = run
            .as_ref()
            .and_then(|r| self.conversations.get(&r.transcript))
            .map(|c| c.transcript.entries.as_slice())
            .unwrap_or(&[]);
        let running = run.as_ref().is_some_and(|r| r.state == RunState::Running);

        // Follow a running agent: new entries scroll the page to the end.
        if s.follow && entries.len() != s.seen {
            s.scroll.scroll_to_bottom();
        }
        let seen = entries.len();

        // ---- header: back, ticket, run cards ----
        let ticket_path = s.ticket.clone();
        let header = div()
            .flex()
            .flex_none()
            .items_center()
            .gap_3()
            .h(px(46.))
            .px_4()
            .border_b_1()
            .border_color(border)
            .child(
                Button::new("session-back")
                    .ghost()
                    .small()
                    .icon(Icon::new(IconName::ArrowLeft))
                    .label(tk.key.clone())
                    .on_click(cx.listener(move |this, _, w, cx| {
                        this.session = None;
                        this.open_detail(ticket_path.clone(), true, w, cx);
                    })),
            )
            .child(div().flex_1().min_w_0().truncate().text_sm().font_weight(FontWeight::SEMIBOLD).text_color(fg).child(tk.title.clone()));

        let cards = div().flex().flex_wrap().gap_2().children(runs.iter().enumerate().map(|(i, r)| {
            let on = run.as_ref().is_some_and(|x| x.transcript == r.transcript);
            let waiting = r.state != RunState::Running
                && self.conversations.get(&r.transcript).is_some_and(|c| c.transcript.background.iter().any(|b| !b.done));
            let (state, color) = match r.state {
                _ if waiting => ("Waiting on background".to_string(), theme.yellow),
                RunState::Running => (format!("Working for {}", span(now - r.started)), theme.green),
                RunState::AwaitingReview => ("Awaiting review".to_string(), theme.yellow),
                RunState::Finished => (format!("Finished {}", ago_label(now - r.last_activity)), muted),
            };
            let path = r.transcript.clone();
            let live = r.state == RunState::Running;
            div()
                .id(("session-run", i))
                .flex()
                .flex_col()
                .gap_0p5()
                .px_3()
                .py_2()
                .min_w(px(180.))
                .rounded(px(8.))
                .border_1()
                .border_color(if on { muted.opacity(0.35) } else { border })
                .when(on, |d| d.bg(muted.opacity(0.1)))
                .cursor_pointer()
                .hover(|d| d.bg(muted.opacity(0.08)))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(crate::icons::harness_logo(r.provider, 16.))
                        .child(div().text_sm().font_weight(FontWeight::MEDIUM).text_color(fg).child(r.provider.label())),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_1p5()
                        .text_xs()
                        .text_color(muted)
                        .child(if live {
                            Spinner::new().xsmall().color(color).into_any_element()
                        } else {
                            div().size(px(7.)).rounded_full().bg(color).into_any_element()
                        })
                        .child(state),
                )
                .on_click(cx.listener(move |this, _, _, cx| {
                    if let Some(s) = &mut this.session {
                        s.run = path.clone();
                        s.open.clear();
                        s.follow = live;
                        s.seen = 0;
                        s.scroll.set_offset(point(px(0.), px(0.)));
                    }
                    cx.notify();
                }))
        }));

        // ---- the story ----
        let style = crate::detail::md_style(cx);
        let md_of = |i: usize| run.as_ref().and_then(|r| self.conversations.get(&r.transcript)).and_then(|c| c.md.get(i).cloned().flatten());
        let all = items(entries);
        let last_work = all.iter().rposition(|it| matches!(it, Item::Work(..)));
        let mut story: Vec<AnyElement> = Vec::new();
        for (n, item) in all.iter().enumerate() {
            match *item {
                Item::Prompt(i) => {
                    let e = &entries[i];
                    let long = e.text.len() > 700;
                    let open = !long || s.open.contains(&i);
                    story.push(
                        div()
                            .flex()
                            .justify_end()
                            .child(
                                div()
                                    .max_w(relative(0.85))
                                    .flex()
                                    .flex_col()
                                    .gap_1()
                                    .px_4()
                                    .py_3()
                                    .rounded(px(14.))
                                    .bg(muted.opacity(0.12))
                                    .child(div().when(!open, |d| d.max_h(px(150.)).overflow_hidden()).child(match md_of(i) {
                                        Some(md) => TextView::new(&md).selectable(true).style(style.clone()).into_any_element(),
                                        None => div().text_sm().text_color(fg).child(e.text.clone()).into_any_element(),
                                    }))
                                    .child(
                                        div()
                                            .flex()
                                            .items_center()
                                            .gap_2()
                                            .text_xs()
                                            .text_color(muted)
                                            .when(e.at > 0, |d| d.child(ago_label(now - e.at)))
                                            .when(long, |d| {
                                                d.child(
                                                    div()
                                                        .id(("prompt-more", i))
                                                        .text_color(accent)
                                                        .cursor_pointer()
                                                        .hover(|d| d.underline())
                                                        .child(if open { "Show less" } else { "Show all" })
                                                        .on_click(cx.listener(move |this, _, _, cx| {
                                                            if let Some(s) = &mut this.session
                                                                && !s.open.remove(&i)
                                                            {
                                                                s.open.insert(i);
                                                            }
                                                            cx.notify();
                                                        })),
                                                )
                                            }),
                                    ),
                            )
                            .into_any_element(),
                    );
                }
                Item::Reply(i) => {
                    let e = &entries[i];
                    story.push(match md_of(i) {
                        Some(md) => div().child(TextView::new(&md).selectable(true).style(style.clone())).into_any_element(),
                        None => div().text_sm().text_color(fg).child(e.text.clone()).into_any_element(),
                    });
                }
                Item::Work(from, to) => {
                    let live = running && Some(n) == last_work && n + 1 == all.len();
                    let open = live || s.open.contains(&from);
                    let steps = entries[from..to].iter().filter(|e| e.kind == Kind::Tool).count();
                    let start = entries[from].at;
                    let end = entries.get(to).map(|e| e.at).filter(|&t| t > 0).unwrap_or(entries[to - 1].at);
                    let worked = if start > 0 && end >= start { span(end - start) } else { String::new() };
                    let title = if live {
                        let since = if start > 0 { format!(" for {}", span(now - start)) } else { String::new() };
                        format!("Working{since} · {steps} step{}", if steps == 1 { "" } else { "s" })
                    } else if worked.is_empty() {
                        format!("{steps} step{}", if steps == 1 { "" } else { "s" })
                    } else {
                        format!("Worked for {worked} · {steps} step{}", if steps == 1 { "" } else { "s" })
                    };
                    let mut block = div().flex().flex_col().child(
                        div()
                            .id(("work", from))
                            .flex()
                            .items_center()
                            .gap_2()
                            .h(px(34.))
                            .px_3()
                            .rounded(px(8.))
                            .border_1()
                            .border_color(border)
                            .text_sm()
                            .text_color(muted)
                            .cursor_pointer()
                            .hover(|d| d.bg(muted.opacity(0.06)).text_color(fg))
                            // A spinner and a shimmer read as "still going"; a
                            // green dot read as done.
                            .child(if live {
                                Spinner::new().xsmall().color(accent).into_any_element()
                            } else {
                                Icon::new(IconName::ListChecks).size(px(14.)).into_any_element()
                            })
                            .child(if live {
                                div().flex_1().text_color(fg).child(ShimmerText::new(title).id(("work-live", from))).into_any_element()
                            } else {
                                div().flex_1().child(title).into_any_element()
                            })
                            .child(Icon::new(if open { IconName::ChevronDown } else { IconName::ChevronRight }).size(px(14.)))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                if let Some(s) = &mut this.session
                                    && !s.open.remove(&from)
                                {
                                    s.open.insert(from);
                                }
                                cx.notify();
                            })),
                    );
                    if open {
                        let mut list = div().ml(px(18.)).mt_1().pl_4().border_l_1().border_color(border).flex().flex_col().gap_0p5().py_1();
                        for j in from..to {
                            let e = &entries[j];
                            let key = j + 1_000_000;
                            let expanded = s.open.contains(&key);
                            let row = if e.kind == Kind::Thinking {
                                let next = entries.get(j + 1).map(|x| x.at).unwrap_or(e.at);
                                let label = if e.at > 0 && next > e.at { format!("Thought for {}", span(next - e.at)) } else { "Thought".into() };
                                div()
                                    .flex()
                                    .flex_col()
                                    .child(
                                        div()
                                            .id(("think", j))
                                            .flex()
                                            .items_center()
                                            .gap_2()
                                            .h(px(26.))
                                            .cursor_pointer()
                                            .text_sm()
                                            .text_color(muted)
                                            .hover(|d| d.text_color(fg))
                                            .child(Icon::new(IconName::Sparkles).size(px(13.)))
                                            .child(label)
                                            .on_click(cx.listener(move |this, _, _, cx| {
                                                if let Some(s) = &mut this.session
                                                    && !s.open.remove(&key)
                                                {
                                                    s.open.insert(key);
                                                }
                                                cx.notify();
                                            })),
                                    )
                                    .when(expanded, |d| {
                                        d.child(div().ml(px(21.)).mb_1().text_sm().italic().text_color(muted).child(e.text.clone()))
                                    })
                                    .into_any_element()
                            } else {
                                let (icon, verb, target) = step(e);
                                // The call that has no result yet is the one running now.
                                let busy = live && e.output.is_none() && j + 1 == to;
                                div()
                                    .flex()
                                    .flex_col()
                                    .child(
                                        div()
                                            .id(("step", j))
                                            .flex()
                                            .items_center()
                                            .gap_2()
                                            .h(px(26.))
                                            .min_w_0()
                                            .cursor_pointer()
                                            .text_sm()
                                            .hover(|d| d.bg(muted.opacity(0.06)))
                                            .child(if busy {
                                                Spinner::new().xsmall().color(accent).into_any_element()
                                            } else {
                                                Icon::new(icon).size(px(13.)).text_color(muted).into_any_element()
                                            })
                                            .child(div().flex_none().text_color(fg).child(verb))
                                            .child(div().flex_1().min_w_0().truncate().text_color(muted).child(target))
                                            .on_click(cx.listener(move |this, _, _, cx| {
                                                if let Some(s) = &mut this.session
                                                    && !s.open.remove(&key)
                                                {
                                                    s.open.insert(key);
                                                }
                                                cx.notify();
                                            })),
                                    )
                                    .when(expanded, |d| {
                                        d.child(
                                            div()
                                                .ml(px(21.))
                                                .mb_1()
                                                .p_2()
                                                .rounded(px(6.))
                                                .bg(muted.opacity(0.08))
                                                .text_xs()
                                                .font_family(crate::settings::mono_font())
                                                .text_color(fg)
                                                .child(e.output.clone().unwrap_or_else(|| "No output yet.".into())),
                                        )
                                    })
                                    .into_any_element()
                            };
                            list = list.child(row);
                        }
                        block = block.child(list);
                    }
                    story.push(block.into_any_element());
                }
            }
        }
        if entries.is_empty() {
            story.push(div().text_sm().text_color(muted).child("Reading the conversation…").into_any_element());
        } else if running && !matches!(all.last(), Some(Item::Work(..))) {
            story.push(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .text_sm()
                    .text_color(muted)
                    .child(Spinner::new().xsmall().color(accent))
                    .child(ShimmerText::new("Working…").id("work-tail"))
                    .into_any_element(),
            );
        } else if let Some(r) = run.as_ref().filter(|r| r.state != RunState::Running) {
            story.push(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .pt_2()
                    .text_xs()
                    .text_color(muted)
                    .child(Icon::new(IconName::CircleCheck).size(px(13.)))
                    .child(format!("Stopped {} · ran {}", ago_label(now - r.last_activity), span(r.last_activity - r.started)))
                    .into_any_element(),
            );
        }

        // ---- live status: a floating pill, with the background commands ----
        let conv = run.as_ref().and_then(|r| self.conversations.get(&r.transcript));
        let background: Vec<crate::transcript::Background> =
            conv.map(|c| c.transcript.background.iter().filter(|b| !b.done).cloned().collect()).unwrap_or_default();
        let footer = (running || !background.is_empty()).then(|| {
            let (turn_start, tokens) = conv.map(|c| (c.transcript.turn_start, c.transcript.turn_tokens)).unwrap_or((0, 0));
            let (verb, doing) = if !running {
                ("Waiting", "on background commands".to_string())
            } else {
                match entries.last() {
                    Some(e) if e.kind == Kind::Thinking => ("Thinking", "thinking".to_string()),
                    Some(e) if e.kind == Kind::Tool && e.output.is_none() => {
                        let (_, v, _) = step(e);
                        let verb = match v {
                            "Read" => "Reading",
                            "Edited" => "Editing",
                            "Ran" => "Running",
                            "Searched" | "Searched the web for" => "Searching",
                            "Started a subagent:" => "Waiting on a subagent",
                            _ => "Working",
                        };
                        (verb, e.text.clone())
                    }
                    Some(e) if e.kind == Kind::Assistant => ("Writing", "replying".to_string()),
                    _ => ("Working", "working".to_string()),
                }
            };
            let mut facts = Vec::new();
            if turn_start > 0 {
                facts.push(span(now - turn_start));
            }
            if tokens > 0 {
                facts.push(if tokens >= 1000 { format!("↓ {:.1}k tokens", tokens as f64 / 1000.) } else { format!("↓ {tokens} tokens") });
            }
            facts.push(doing);
            let bg_open = s.bg_open;
            let panel = (bg_open && !background.is_empty()).then(|| {
                div()
                    .w(px(620.))
                    .max_w(relative(1.))
                    .flex()
                    .flex_col()
                    .gap_1()
                    .p_2()
                    .rounded(px(12.))
                    .border_1()
                    .border_color(border)
                    .bg(theme.popover)
                    .shadow_lg()
                    .children(background.iter().enumerate().map(|(i, b)| {
                        let shown = s.bg_shown.contains(&b.id);
                        let id = b.id.clone();
                        let out = conv.and_then(|c| c.tails.get(&b.id).cloned()).unwrap_or_default();
                        div()
                            .flex()
                            .flex_col()
                            .child(
                                div()
                                    .id(("bg", i))
                                    .flex()
                                    .items_center()
                                    .gap_2()
                                    .px_2()
                                    .h(px(30.))
                                    .rounded(px(6.))
                                    .cursor_pointer()
                                    .hover(|d| d.bg(muted.opacity(0.08)))
                                    .text_sm()
                                    .child(Spinner::new().xsmall().color(accent))
                                    .child(div().flex_1().min_w_0().truncate().text_color(fg).child(b.label.clone()))
                                    .child(div().text_xs().text_color(muted).child(if b.started > 0 { span(now - b.started) } else { String::new() }))
                                    .child(Icon::new(if shown { IconName::ChevronDown } else { IconName::ChevronRight }).size(px(13.)).text_color(muted))
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        if let Some(s) = &mut this.session
                                            && !s.bg_shown.remove(&id)
                                        {
                                            s.bg_shown.insert(id.clone());
                                        }
                                        cx.notify();
                                    })),
                            )
                            .when(shown, |d| {
                                d.child(
                                    div()
                                        .id(("bg-out", i))
                                        .mx_2()
                                        .mb_1()
                                        .p_2()
                                        .max_h(px(220.))
                                        .overflow_y_scroll()
                                        .rounded(px(6.))
                                        .bg(muted.opacity(0.08))
                                        .text_xs()
                                        .font_family(crate::settings::mono_font())
                                        .text_color(fg)
                                        .child(if out.is_empty() { "No output yet.".to_string() } else { out }),
                                )
                            })
                    }))
            });
            let n = background.len();
            div()
                .absolute()
                .bottom(px(16.))
                .left_0()
                .right_0()
                .px_8()
                .flex()
                .flex_col()
                .items_center()
                .gap_2()
                .children(panel)
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .max_w(relative(1.))
                        .pl_3()
                        .pr_2()
                        .h(px(38.))
                        .rounded_full()
                        .border_1()
                        .border_color(border)
                        .bg(theme.popover)
                        .shadow_lg()
                        .text_sm()
                        .child(Spinner::new().xsmall().color(accent))
                        .child(div().flex_none().text_color(accent).child(ShimmerText::new(format!("{verb}…")).id("session-status")))
                        .child(div().min_w_0().truncate().text_color(muted).child(format!("({})", facts.join(" · "))))
                        .when(n > 0, |d| {
                            d.child(
                                div()
                                    .id("bg-toggle")
                                    .flex()
                                    .flex_none()
                                    .items_center()
                                    .gap_1p5()
                                    .ml_1()
                                    .px_2p5()
                                    .h(px(26.))
                                    .rounded_full()
                                    .bg(muted.opacity(0.12))
                                    .cursor_pointer()
                                    .hover(|d| d.bg(muted.opacity(0.2)))
                                    .text_xs()
                                    .text_color(fg)
                                    .child(Icon::new(IconName::Terminal).size(px(12.)))
                                    .child(format!("{n} in background"))
                                    .child(Icon::new(if bg_open { IconName::ChevronDown } else { IconName::ChevronUp }).size(px(12.)))
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        if let Some(s) = &mut this.session {
                                            s.bg_open = !s.bg_open;
                                        }
                                        cx.notify();
                                    })),
                            )
                        }),
                )
        });

        let entries_bg_open = !background.is_empty();
        // ---- right rail: facts and actions ----
        let rail = run.as_ref().map(|r| {
            let tools: Vec<&Entry> = entries.iter().filter(|e| e.kind == Kind::Tool).collect();
            let edited: HashSet<String> = tools
                .iter()
                .filter(|e| step(e).1 == "Edited")
                .map(|e| e.text.clone())
                .collect();
            let ran = tools.iter().filter(|e| step(e).1 == "Ran").count();
            let fact = |label: &str, value: String| {
                div()
                    .flex()
                    .items_center()
                    .h(px(30.))
                    .gap_2()
                    .text_sm()
                    .child(div().w(px(96.)).flex_none().text_color(muted).child(label.to_string()))
                    .child(div().flex_1().min_w_0().truncate().text_color(fg).child(value))
            };
            let waiting = r.state != RunState::Running && entries_bg_open;
            let state = match r.state {
                _ if waiting => "Waiting on background",
                RunState::Running => "Working",
                RunState::AwaitingReview => "Awaiting review",
                RunState::Finished => "Finished",
            };
            let resume = resume_command(r);
            let transcript = r.transcript.clone();
            let worktree = r.worktree.clone();
            let follow = s.follow;
            div()
                .w(px(300.))
                .flex_none()
                .flex()
                .flex_col()
                .gap_1()
                .p_4()
                .border_l_1()
                .border_color(border)
                .child(div().pb_1().text_xs().font_weight(FontWeight::MEDIUM).text_color(muted).child("SESSION"))
                .child(fact("Status", state.into()))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .h(px(30.))
                        .gap_2()
                        .text_sm()
                        .child(div().w(px(96.)).flex_none().text_color(muted).child("Harness"))
                        .child(crate::icons::harness_logo(r.provider, 16.))
                        .child(div().text_color(fg).child(r.provider.label())),
                )
                .child(fact("Started", ago_label(now - r.started)))
                .child(fact("Last activity", ago_label(now - r.last_activity)))
                .child(fact("Ran for", span(r.last_activity - r.started)))
                .child(fact("Steps", tools.len().to_string()))
                .child(fact("Files edited", edited.len().to_string()))
                .child(fact("Commands", ran.to_string()))
                .when_some(worktree.clone(), |d, w| d.child(fact("Worktree", repo_relative(&w).trim_end_matches('/').to_string())))
                .child(div().h(px(12.)))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_2()
                        .when(running, |d| {
                            d.child(
                                Button::new("session-follow")
                                    .w_full()
                                    .when(follow, |b| b.primary())
                                    .icon(Icon::new(IconName::ArrowRight))
                                    .label(if follow { "Following live" } else { "Follow live" })
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        if let Some(s) = &mut this.session {
                                            s.follow = !s.follow;
                                            if s.follow {
                                                s.scroll.scroll_to_bottom();
                                            }
                                        }
                                        cx.notify();
                                    })),
                            )
                        })
                        .when_some(resume, |d, cmd| {
                            d.child(
                                Button::new("session-resume")
                                    .w_full()
                                    .icon(Icon::new(IconName::Terminal))
                                    .label("Copy resume command")
                                    .tooltip(cmd.clone())
                                    .on_click(cx.listener(move |this, _, _, cx| this.copy("resume command", cmd.clone(), cx))),
                            )
                        })
                        .when_some(worktree, |d, w| {
                            d.child(
                                Button::new("session-worktree")
                                    .w_full()
                                    .icon(Icon::new(IconName::FolderGit2))
                                    .label("Reveal worktree")
                                    .on_click(move |_, _, cx| cx.reveal_path(&w)),
                            )
                        })
                        .child(
                            Button::new("session-file")
                                .w_full()
                                .icon(Icon::new(IconName::FileText))
                                .label("Reveal transcript")
                                .on_click(move |_, _, cx| cx.reveal_path(&transcript)),
                        ),
                )
        });

        if let Some(s) = &mut self.session {
            s.seen = seen;
        }
        let scroll = self.session.as_ref().map(|s| s.scroll.clone()).unwrap_or_default();
        div()
            .flex_1()
            .min_w_0()
            .min_h_0()
            .flex()
            .flex_col()
            .child(header)
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .child(
                        div().flex_1().min_w_0().relative().flex().flex_col().child(
                        div()
                            .id("session-scroll")
                            .flex_1()
                            .min_h_0()
                            .overflow_y_scroll()
                            .track_scroll(&scroll)
                            // Scrolling up by hand stops following.
                            .on_scroll_wheel(cx.listener(|this, e: &ScrollWheelEvent, w, cx| {
                                if f32::from(e.delta.pixel_delta(w.line_height()).y) > 0.
                                    && let Some(s) = &mut this.session
                                    && s.follow
                                {
                                    s.follow = false;
                                    cx.notify();
                                }
                            }))
                            .child(
                                div().flex().justify_center().px_8().pt_6().pb(px(96.)).child(
                                    div()
                                        .w_full()
                                        .max_w(px(780.))
                                        .flex()
                                        .flex_col()
                                        .gap_4()
                                        .child(cards)
                                        .children(story),
                                ),
                            ),
                        )
                        .children(footer),
                    )
                    .children(rail),
            )
            .into_any_element()
    }
}
