//! Agents: every Claude Code run on this board's tickets in the last three
//! days, from the session transcripts. Running ones first.

use gpui_kit::assets::IconName;
use gpui_kit::component::Icon;
use gpui_kit::component::theme::ActiveTheme as _;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::agents::RunState;
use crate::app::{KuzgunApp, ago_label, now_unix, repo_relative};
use crate::board::status_label;
use crate::icons;

fn duration(secs: i64) -> String {
    let s = secs.max(0);
    match s {
        0..=59 => format!("{s}s"),
        60..=3599 => format!("{}m", s / 60),
        _ => format!("{}h {}m", s / 3600, (s % 3600) / 60),
    }
}

impl KuzgunApp {
    pub fn render_agents_page(&mut self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let t = cx.theme().clone();
        let (muted, fg, border) = (t.muted_foreground, t.foreground, t.border);
        let light = crate::settings::is_light(cx);
        let now = now_unix();

        let mut runs = self.agent_runs.clone();
        runs.sort_by_key(|r| (r.state != RunState::Running, std::cmp::Reverse(r.last_activity)));
        let running = runs.iter().filter(|r| r.state == RunState::Running).count();
        let today = runs.iter().filter(|r| now - r.started <= 86_400).count();
        let mut durations: Vec<i64> = runs
            .iter()
            .filter(|r| r.state != RunState::Running)
            .map(|r| r.last_activity - r.started)
            .collect();
        durations.sort();
        let median = durations.get(durations.len() / 2).copied();

        let stat = |label: &str, value: String| {
            div()
                .flex()
                .flex_col()
                .gap_0p5()
                .px_4()
                .py_3()
                .rounded(px(10.))
                .border_1()
                .border_color(border)
                .min_w(px(150.))
                .child(div().text_xs().text_color(muted).child(label.to_string()))
                .child(div().text_size(px(20.)).font_weight(FontWeight::SEMIBOLD).text_color(fg).child(value))
        };

        let rows: Vec<AnyElement> = runs
            .iter()
            .enumerate()
            .map(|(n, r)| {
                let ticket = self.run_ticket(&r.ticket_rel);
                let (state_label, color) = match r.state {
                    RunState::Running => ("Running", t.green),
                    RunState::AwaitingReview => ("Awaiting review", t.yellow),
                    RunState::Finished => ("Finished", muted),
                };
                let (key, title, status) = ticket
                    .map(|i| {
                        let tk = &self.board.tickets[i];
                        (tk.key.clone(), tk.title.clone(), Some((tk.category, status_label(&tk.status_key))))
                    })
                    .unwrap_or_else(|| (String::new(), r.ticket_rel.clone(), None));
                let path = ticket.map(|i| self.board.tickets[i].path.clone());
                let worktree = r.worktree.clone();
                div()
                    .id(("run", n))
                    .flex()
                    .items_center()
                    .gap_3()
                    .px_3()
                    .h(px(52.))
                    .when(n > 0, |d| d.border_t_1().border_color(border))
                    .cursor_pointer()
                    .hover(|d| d.bg(muted.opacity(0.07)))
                    .child(
                        div()
                            .flex()
                            .flex_none()
                            .items_center()
                            .gap_1p5()
                            .w(px(140.))
                            .child(div().size(px(8.)).rounded_full().bg(color))
                            .child(div().text_sm().text_color(fg).child(state_label)),
                    )
                    .child(div().w(px(64.)).flex_none().text_xs().font_family(crate::settings::mono_font()).text_color(muted).child(key))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .child(div().truncate().text_sm().text_color(fg).child(title))
                            .child(div().truncate().text_xs().text_color(muted).child(r.description.clone())),
                    )
                    .when_some(status, |d, (cat, label)| {
                        d.child(
                            div()
                                .flex()
                                .flex_none()
                                .items_center()
                                .gap_1()
                                .w(px(130.))
                                .text_xs()
                                .text_color(muted)
                                .child(icons::status_icon(cat).size(px(12.)).text_color(icons::status_color(cat, light)))
                                .child(label),
                        )
                    })
                    .child(div().w(px(90.)).flex_none().text_xs().text_color(muted).child(format!("ran {}", duration(r.last_activity - r.started))))
                    .child(div().w(px(90.)).flex_none().text_xs().text_color(muted).child(ago_label(now - r.last_activity)))
                    .child(
                        div()
                            .w(px(28.))
                            .flex_none()
                            .when_some(worktree, |d, w| {
                                d.child(
                                    div()
                                        .id(("run-wt", n))
                                        .child(Icon::new(IconName::FolderGit2).size(px(14.)).text_color(muted))
                                        .tooltip(move |window, cx| {
                                            gpui_kit::component::tooltip::Tooltip::new(format!("Worktree: {}", repo_relative(&w).trim_end_matches('/'))).max_w(px(360.)).build(window, cx)
                                        }),
                                )
                            }),
                    )
                    .on_click(cx.listener(move |this, _, w, cx| {
                        if let Some(p) = path.clone() {
                            this.open_detail(p, true, w, cx);
                        }
                    }))
                    .into_any_element()
            })
            .collect();

        div()
            .id("agents-page")
            .flex_1()
            .min_w_0()
            .min_h_0()
            .overflow_y_scroll()
            .child(
                div().flex().justify_center().px_8().py_8().child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_6()
                        .w_full()
                        .max_w(px(1040.))
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .gap_1()
                                .child(div().text_size(px(22.)).font_weight(FontWeight::SEMIBOLD).text_color(fg).child("Agents"))
                                .child(
                                    div()
                                        .text_sm()
                                        .text_color(muted)
                                        .child("Claude Code runs on this board's tickets in the last three days, read from the session transcripts."),
                                ),
                        )
                        .child(
                            div()
                                .flex()
                                .flex_wrap()
                                .gap_3()
                                .child(stat("Running now", running.to_string()))
                                .child(stat("Runs in the last 24h", today.to_string()))
                                .child(stat("All runs (3 days)", runs.len().to_string()))
                                .child(stat("Median run", median.map(duration).unwrap_or_else(|| "–".into()))),
                        )
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .rounded(px(10.))
                                .border_1()
                                .border_color(border)
                                .overflow_hidden()
                                .when(rows.is_empty(), |d| {
                                    d.child(div().p_4().text_sm().text_color(muted).child("No agent has worked on these tickets in the last three days."))
                                })
                                .children(rows),
                        ),
                ),
            )
    }
}
