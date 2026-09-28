//! Home: what needs the person now, what agents are doing, what can start
//! next, what just finished and how each project stands. The common shape
//! of Linear, Jira, Asana and Plane homes, for one person and many agents.

use gpui_kit::assets::IconName;
use gpui_kit::component::{Icon, Sizable as _};
use gpui_kit::component::theme::ActiveTheme as _;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::agents::RunState;
use crate::app::{KuzgunApp, ago_label, now_unix, zrem};
use crate::icons;
use crate::model::{self, Category, Mode};

impl KuzgunApp {
    pub fn render_home(&mut self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let t = cx.theme().clone();
        let (muted, fg) = (t.muted_foreground, t.foreground);
        let now = now_unix();
        let n = self.board.tickets.len();
        // Home covers the whole board, whatever project the board shows.
        let in_scope = |_: usize| true;

        let running: Vec<usize> = (0..n)
            .filter(|&i| in_scope(i))
            .filter(|&i| self.board.tickets[i].agent.as_ref().is_some_and(|a| a.state == RunState::Running))
            .collect();
        let frontier = |i: usize| self.idx.frontier.get(i).copied().unwrap_or(false);
        let mode = |i: usize| self.idx.modes.get(i).copied().flatten();
        let mut needs: Vec<(usize, &'static str)> = Vec::new();
        for i in (0..n).filter(|&i| in_scope(i)) {
            let tk = &self.board.tickets[i];
            // Only work that waits on a decision from a person: a review
            // or a triage. Unstarted work is in Ready next.
            if tk.category == Category::InReview {
                needs.push((i, "Review"));
            } else if tk.status_key == "needs-triage" {
                needs.push((i, "Triage"));
            }
        }
        let ready: Vec<usize> = (0..n).filter(|&i| in_scope(i) && frontier(i)).collect();
        let week = 7 * 86_400;
        let mut finished: Vec<(usize, i64)> = (0..n)
            .filter(|&i| in_scope(i) && self.board.tickets[i].category == Category::Done)
            .map(|i| (i, self.status_since(i)))
            .filter(|(_, at)| now - at <= week)
            .collect();
        finished.sort_by_key(|(_, at)| std::cmp::Reverse(*at));
        let day_start = {
            use chrono::{Local, TimeZone};
            let today = Local::now().date_naive().and_hms_opt(0, 0, 0).unwrap_or_default();
            Local.from_local_datetime(&today).single().map(|d| d.timestamp()).unwrap_or(now - 86_400)
        };
        let done_today = finished.iter().filter(|(_, at)| *at >= day_start).count();

        // ---- header ----
        let hour = chrono::Local::now().format("%H").to_string().parse::<u32>().unwrap_or(12);
        let greeting = match hour {
            5..=11 => "Good morning",
            12..=17 => "Good afternoon",
            _ => "Good evening",
        };
        let stat = |icon: IconName, color: Hsla, text: String| {
            div()
                .flex()
                .items_center()
                .gap_1p5()
                .px_2p5()
                .h(px(28.))
                .rounded(px(14.))
                .bg(color.opacity(0.12))
                .text_sm()
                .text_color(fg)
                .child(Icon::new(icon).size(px(14.)).text_color(color))
                .child(text)
        };
        let plural = |k: usize, one: &str, many: &str| format!("{k} {}", if k == 1 { one } else { many });
        let header = div()
            .flex()
            .flex_col()
            .gap_3()
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_0p5()
                    .child(div().text_sm().text_color(muted).child(chrono::Local::now().format("%A, %e %B").to_string()))
                    .child(div().text_size(zrem(24.)).font_weight(FontWeight::SEMIBOLD).text_color(fg).child(greeting)),
            )
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .gap_2()
                    .child(stat(IconName::Bot, t.green, plural(running.len(), "agent working", "agents working")))
                    .child(stat(IconName::Hand, t.accent, format!("{} need you", needs.len())))
                    .child(stat(IconName::Compass, t.blue, format!("{} ready", ready.len())))
                    .child(stat(IconName::CircleCheck, icons::status_color(Category::Done, false), format!("{done_today} done today"))),
            );

        let mut col = div().flex().flex_col().gap_8().w_full().max_w(px(960.)).child(header);

        // ---- needs you ----
        if !needs.is_empty() {
            let rows: Vec<AnyElement> = needs
                .iter()
                .map(|&(i, why)| self.home_row(i, Some(why), cx))
                .collect();
            col = col.child(self.home_section("Needs you", needs.len(), IconName::Hand, rows, cx));
        }
        // ---- running now ----
        {
            let rows: Vec<AnyElement> = if running.is_empty() {
                vec![div().px_3().py_2().text_sm().text_color(muted).child("No agent is working right now.").into_any_element()]
            } else {
                running
                    .iter()
                    .map(|&i| {
                        let run = self.board.tickets[i].agent.clone();
                        let extra = run.map(|r| {
                            format!("started {} · last activity {}", ago_label(now - r.started), ago_label(now - r.last_activity))
                        });
                        self.home_row_with(i, None, extra, None, cx)
                    })
                    .collect()
            };
            col = col.child(self.home_section("Running now", running.len(), IconName::Bot, rows, cx));
        }
        // ---- ready next ----
        if !ready.is_empty() {
            let rows: Vec<AnyElement> = ready
                .iter()
                .take(8)
                .map(|&i| self.home_row(i, (mode(i) == Some(Mode::Hitl)).then_some("Needs a person"), cx))
                .collect();
            let cmds: Vec<String> = ready.iter().filter_map(|&i| self.next_command(i).map(|(_, c, _)| c)).collect();
            let section = self.home_section("Ready next", ready.len(), IconName::Compass, rows, cx);
            col = col.child(
                div()
                    .relative()
                    .child(section)
                    .when(!cmds.is_empty(), |d| {
                        let n = cmds.len();
                        d.child(
                            div().absolute().top_0().right_0().child(
                                gpui_kit::component::button::Button::new("copy-ready")
                                    .small()
                                    .icon(Icon::new(IconName::Terminal))
                                    .label(format!("Copy {n} command{}", if n == 1 { "" } else { "s" }))
                                    .tooltip("One line per ticket, to hand them to agents in parallel")
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        let text = cmds.join("\n");
                                        cx.write_to_clipboard(ClipboardItem::new_string(text));
                                        this.toast(None, format!("Copied {n} commands"));
                                    })),
                            ),
                        )
                    }),
            );
        }
        // ---- recently finished ----
        if !finished.is_empty() {
            let mut rows: Vec<AnyElement> = Vec::new();
            let mut last_group = "";
            for &(i, at) in finished.iter().take(12) {
                let group = if at >= day_start {
                    "Today"
                } else if at >= day_start - 86_400 {
                    "Yesterday"
                } else {
                    "Earlier this week"
                };
                if group != last_group {
                    rows.push(
                        div()
                            .px_3()
                            .pt_2()
                            .pb_1()
                            .text_xs()
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(muted)
                            .child(group)
                            .into_any_element(),
                    );
                    last_group = group;
                }
                rows.push(self.home_row_with(i, None, None, Some(ago_label(now - at)), cx));
            }
            col = col.child(self.home_section("Recently finished", finished.len(), IconName::CircleCheck, rows, cx));
        }
        // ---- projects ----
        let projects: Vec<AnyElement> = self
            .project_order()
            .into_iter()
            .filter(|&p| !self.project_finished(p))
            .map(|p| self.home_project(p, cx))
            .collect();
        col = col.child(self.home_section("Projects", projects.len(), IconName::FolderKanban, projects, cx));

        div()
            .id("home")
            .flex_1()
            .min_w_0()
            .min_h_0()
            .overflow_y_scroll()
            .child(div().flex().justify_center().px_8().py_8().child(col))
    }

    fn home_section(&self, title: &str, n: usize, icon: IconName, rows: Vec<AnyElement>, cx: &mut Context<Self>) -> AnyElement {
        let t = cx.theme();
        let (muted, fg, border) = (t.muted_foreground, t.foreground, t.border);
        div()
            .flex()
            .flex_col()
            .gap_2()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(Icon::new(icon).size(px(16.)).text_color(muted))
                    .child(div().text_base().font_weight(FontWeight::SEMIBOLD).text_color(fg).child(title.to_string()))
                    .child(div().text_sm().text_color(muted).child(n.to_string())),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .rounded(px(10.))
                    .border_1()
                    .border_color(border)
                    .overflow_hidden()
                    .children(rows.into_iter().enumerate().map(|(i, r)| {
                        div().when(i > 0, |d| d.border_t_1().border_color(border)).child(r)
                    })),
            )
            .into_any_element()
    }

    fn home_row(&self, i: usize, why: Option<&'static str>, cx: &mut Context<Self>) -> AnyElement {
        self.home_row_with(i, why, None, None, cx)
    }

    fn home_row_with(
        &self,
        i: usize,
        why: Option<&'static str>,
        extra: Option<String>,
        aside: Option<String>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let tk = &self.board.tickets[i];
        let t = cx.theme();
        let (muted, fg, accent) = (t.muted_foreground, t.foreground, t.accent);
        let light = crate::settings::is_light(cx);
        let project = &self.board.projects[tk.project];
        let (done, total) = tk.checklist_counts();
        let path = tk.path.clone();
        let people = self.assignees(i).unwrap_or_default();
        div()
            .id(("home-row", i))
            .flex()
            .items_center()
            .gap_3()
            .px_3()
            .py_2()
            .min_h(px(44.))
            .cursor_pointer()
            .hover(|d| d.bg(muted.opacity(0.07)))
            .child(icons::status_icon(tk.category).size(px(15.)).text_color(icons::status_color(tk.category, light)))
            .child(
                div()
                    .w(px(64.))
                    .flex_none()
                    .text_xs()
                    .font_family(crate::settings::mono_font())
                    .text_color(muted)
                    .child(tk.key.clone()),
            )
            .child(
                div()
                    .flex_1()
                    .min_w(px(160.))
                    .flex()
                    .flex_col()
                    .child(div().truncate().text_sm().text_color(fg).child(tk.title.clone()))
                    .when_some(extra, |d, e| d.child(div().truncate().text_xs().text_color(muted).child(e))),
            )
            .when_some(why, |d, w| {
                d.child(
                    div()
                        .flex_none()
                        .px_2()
                        .py_0p5()
                        .rounded(px(5.))
                        .bg(accent.opacity(0.14))
                        .text_xs()
                        .text_color(accent)
                        .child(w),
                )
            })
            .when(total > 0, |d| {
                d.child(
                    div()
                        .flex()
                        .flex_none()
                        .items_center()
                        .gap_1()
                        .text_xs()
                        .text_color(muted)
                        .child(icons::ring(done as f32 / total as f32).size(px(13.)).text_color(muted))
                        .child(format!("{done}/{total}")),
                )
            })
            .when_some(aside, |d, a| d.child(div().flex_none().w(px(64.)).text_right().text_xs().text_color(muted).child(a)))
            .child(
                div()
                    .flex()
                    .flex_shrink(1.)
                    .min_w_0()
                    .items_center()
                    .gap_1p5()
                    .max_w(px(150.))
                    .overflow_hidden()
                    .text_xs()
                    .text_color(muted)
                    .child(div().size(px(7.)).rounded_full().bg(icons::tag_color(&project.name)))
                    .child(div().truncate().child(project.title.clone())),
            )
            .when(!people.is_empty(), |d| d.child(crate::board::avatar_group(&people, 20., t.background, 3, muted)))
            .on_click(cx.listener(move |this, _, w, cx| this.open_detail(path.clone(), true, w, cx)))
            .into_any_element()
    }

    fn home_project(&self, p: usize, cx: &mut Context<Self>) -> AnyElement {
        let t = cx.theme();
        let (muted, fg, accent) = (t.muted_foreground, t.foreground, t.accent);
        let light = crate::settings::is_light(cx);
        let proj = &self.board.projects[p];
        let ids: Vec<usize> = (0..self.board.tickets.len()).filter(|&i| self.board.tickets[i].project == p).collect();
        let count = |c: Category| ids.iter().filter(|&&i| self.board.tickets[i].category == c).count();
        let closed = ids.iter().filter(|&&i| self.board.tickets[i].category.is_closed()).count();
        let total = ids.len().max(1);
        let frac = closed as f32 / total as f32;
        // The first milestone that still has open work.
        let milestone = self
            .board
            .facets
            .iter()
            .find(|f| model::is_milestone(&f.key))
            .and_then(|f| {
                f.values.iter().find_map(|v| {
                    let (n, c) = ids
                        .iter()
                        .filter(|&&i| self.board.tickets[i].field_values(&f.key).iter().any(|x| x == v))
                        .fold((0, 0), |(n, c), &i| (n + 1, c + self.board.tickets[i].category.is_closed() as usize));
                    (n > 0 && c < n).then(|| (v.clone(), c, n))
                })
            });
        let last = ids.iter().map(|&i| self.updated_at(i)).max().unwrap_or(0);
        let name = proj.name.clone();
        let chip = |c: Category, n: usize| {
            div()
                .flex()
                .items_center()
                .gap_1()
                .text_xs()
                .text_color(muted)
                .child(icons::status_icon(c).size(px(12.)).text_color(icons::status_color(c, light)))
                .child(n.to_string())
        };
        div()
            .id(("home-proj", p))
            .flex()
            .items_center()
            .gap_4()
            .px_3()
            .h(px(52.))
            .cursor_pointer()
            .hover(|d| d.bg(muted.opacity(0.07)))
            .child(icons::ring(frac).size(px(18.)).text_color(icons::tag_color(&proj.name)))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .child(div().truncate().text_sm().font_weight(FontWeight::MEDIUM).text_color(fg).child(proj.title.clone()))
                    .child(
                        div()
                            .flex()
                            .gap_3()
                            .child(chip(Category::Todo, count(Category::Todo)))
                            .child(chip(Category::InProgress, count(Category::InProgress)))
                            .child(chip(Category::InReview, count(Category::InReview)))
                            .child(chip(Category::Done, count(Category::Done))),
                    ),
            )
            .when_some(milestone, |d, (v, c, n)| {
                d.child(
                    div()
                        .flex()
                        .flex_none()
                        .items_center()
                        .gap_1p5()
                        .text_xs()
                        .text_color(muted)
                        .child(icons::diamond(c as f32 / n as f32).size(px(13.)).text_color(accent))
                        .child(format!("{v} · {c}/{n}")),
                )
            })
            .child(
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .gap_2()
                    .w(px(160.))
                    .child(
                        div()
                            .flex_1()
                            .h(px(5.))
                            .rounded_full()
                            .bg(muted.opacity(0.18))
                            .child(div().h_full().rounded_full().bg(accent).w(relative(frac))),
                    )
                    .child(div().text_xs().text_color(muted).child(format!("{:.0}%", frac * 100.))),
            )
            .child(div().w(px(70.)).flex_none().text_xs().text_color(muted).child(ago_label(now_unix() - last)))
            .on_click(cx.listener(move |this, _, _, cx| {
                this.view.project = Some(name.clone());
                this.view.page = crate::store::Page::Board;
                this.save_view();
                cx.notify();
            }))
            .into_any_element()
    }
}
