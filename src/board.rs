//! The board: project sidebar, toolbar, status columns (optionally split
//! into project swimlanes) and cards. The board only reads: the ticket
//! files belong to the agents and skills that write them.

use std::path::PathBuf;
use std::rc::Rc;

use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::Input;
use gpui_kit::component::menu::{ContextMenuExt as _, DropdownMenu as _, PopupMenu, PopupMenuItem};
use gpui_kit::component::theme::ActiveTheme as _;
use gpui_kit::component::{Icon, Sizable as _};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::actions::*;
use crate::app::{KuzgunApp, KuzgunHandle, Quick, ago_since, now_unix, with_app, zrem};
use crate::icons;
use crate::model::{self, Category, Column, Mode};
use crate::store::{Completed, Layout, Ordering};

/// A round avatar with the person's initial, in a colour from the name;
/// the agent gets a robot.
pub fn avatar(name: &str, size: f32, ring: Hsla) -> AnyElement {
    let is_agent = name == AGENT_NAME;
    let bg = if is_agent { hsla(0.42, 0.55, 0.45, 1.) } else { icons::tag_color(name) };
    let initials: String = name
        .split_whitespace()
        .filter_map(|w| w.chars().next())
        .take(2)
        .collect::<String>()
        .to_uppercase();
    div()
        .size(px(size))
        .flex_none()
        .rounded_full()
        .bg(bg)
        .border_2()
        .border_color(ring)
        .flex()
        .items_center()
        .justify_center()
        .text_size(px(size * 0.42))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(rgb(0x111111))
        .when(is_agent, |d| d.child(Icon::new(IconName::Bot).size(px(size * 0.6)).text_color(rgb(0x111111))))
        .when(!is_agent, |d| d.child(initials))
        .into_any_element()
}

/// Overlapping avatars (at most `max`, then `+N`).
pub fn avatar_group(names: &[String], size: f32, ring: Hsla, max: usize, muted: Hsla) -> AnyElement {
    let extra = names.len().saturating_sub(max);
    div()
        .flex()
        .items_center()
        .children(names.iter().take(max).enumerate().map(|(i, n)| {
            div().when(i > 0, |d| d.ml(px(-size * 0.3))).child(avatar(n, size, ring))
        }))
        .when(extra > 0, |d| {
            d.child(
                div()
                    .ml(px(-size * 0.3))
                    .size(px(size))
                    .rounded_full()
                    .bg(muted.opacity(0.3))
                    .border_2()
                    .border_color(ring)
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_size(px(size * 0.4))
                    .child(format!("+{extra}")),
            )
        })
        .into_any_element()
}

/// A column's icon: the status shape, or a lock for tickets that wait on
/// their blockers.
pub fn column_icon(col: &Column, light: bool) -> Icon {
    if col.status == crate::app::WAITING {
        Icon::new(IconName::Lock).text_color(rgb(0xE5484D))
    } else {
        icons::status_icon(col.category).text_color(icons::status_color(col.category, light))
    }
}

/// The name agents go by in avatars.
pub const AGENT_NAME: &str = "Agent";

/// `ready-for-agent` → `Ready for agent`.
pub fn status_label(s: &str) -> String {
    model::humanize(s)
}

const COL_GAP: f32 = 8.;
const BOARD_PAD: f32 = 12.;
const HIDDEN_W: f32 = 220.;

fn tip(text: String) -> impl Fn(&mut Window, &mut App) -> AnyView {
    move |window, cx| gpui_kit::component::tooltip::Tooltip::new(text.clone()).max_w(px(360.)).build(window, cx)
}

impl KuzgunApp {
    pub fn render_board(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let full = self.detail.as_ref().is_some_and(|d| d.full);
        let side = self.detail.is_some() && !full;
        let detail_w = self.view.detail_w.unwrap_or(620.);
        let sidebar = if self.view.sidebar_hidden { None } else { Some(self.render_sidebar(cx).into_any_element()) };
        let main: AnyElement = if full {
            self.render_detail(window, cx).into_any_element()
        } else if self.view.page == crate::store::Page::Home {
            self.render_home(cx).into_any_element()
        } else if self.view.page == crate::store::Page::Dependencies {
            self.render_deps(cx).into_any_element()
        } else if self.view.page == crate::store::Page::Agents {
            self.render_agents_page(cx).into_any_element()
        } else {
            let toolbar = self.render_toolbar(side, cx).into_any_element();
            let banner = self.render_map_banner(cx);
            let columns = self.render_columns(window, cx).into_any_element();
            let footer = self.render_hidden_footer(cx);
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .child(toolbar)
                .children(banner)
                .child(columns)
                .children(footer)
                .into_any_element()
        };
        let border = cx.theme().border;
        let detail = if side { Some(self.render_detail(window, cx).into_any_element()) } else { None };
        div()
            .id("board")
            .key_context(BOARD)
            .track_focus(&self.board_focus)
            .flex_1()
            .min_h_0()
            .flex()
            .flex_row()
            .on_action(cx.listener(|this, _: &MoveDown, w, cx| this.move_selection(0, 1, w, cx)))
            .on_action(cx.listener(|this, _: &MoveUp, w, cx| this.move_selection(0, -1, w, cx)))
            .on_action(cx.listener(|this, _: &MoveLeft, w, cx| this.move_selection(-1, 0, w, cx)))
            .on_action(cx.listener(|this, _: &MoveRight, w, cx| this.move_selection(1, 0, w, cx)))
            .on_action(cx.listener(|this, _: &NextTicket, w, cx| this.move_selection(0, 1, w, cx)))
            .on_action(cx.listener(|this, _: &PrevTicket, w, cx| this.move_selection(0, -1, w, cx)))
            .on_action(cx.listener(|this, _: &Peek, w, cx| {
                if this.detail.as_ref().is_some_and(|d| !d.is_doc && Some(&d.path) == this.selected.as_ref()) {
                    this.detail = None;
                    cx.notify();
                } else if let Some(p) = this.selected.clone() {
                    this.open_detail(p, false, w, cx);
                } else {
                    this.select_first_visible(cx);
                }
            }))
            .on_action(cx.listener(|this, _: &OpenFull, w, cx| {
                if let Some(p) = this.selected.clone() {
                    this.open_detail(p, true, w, cx);
                }
            }))
            .on_action(cx.listener(|this, _: &CloseDetail, _, cx| {
                this.detail = None;
                cx.notify();
            }))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, window, cx| {
                    // A click on the board (not in a text field) gives it the keys.
                    if !this.search.read(cx).focus_handle(cx).is_focused(window) {
                        this.board_focus.focus(window, cx);
                    }
                }),
            )
            .children(sidebar)
            .child(main)
            .when_some(detail, |d, detail| {
                d.child(
                    div()
                        .id("detail-resize")
                        .w(px(5.))
                        .h_full()
                        .flex_none()
                        .cursor_col_resize()
                        .border_l_1()
                        .border_color(border)
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(move |this, e: &MouseDownEvent, _, cx| {
                                cx.stop_propagation();
                                this.detail_drag = Some((f32::from(e.position.x), detail_w));
                            }),
                        ),
                )
                .child(div().w(px(detail_w)).flex_none().h_full().child(detail))
            })
    }

    // ---------- sidebar ----------

    fn render_sidebar(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let t = cx.theme();
        let (muted, fg, border, accent) = (t.muted_foreground, t.foreground, t.border, t.accent);
        let light = crate::settings::is_light(cx);
        let row = |id: ElementId, active: bool| {
            div()
                .id(id)
                .flex()
                .items_center()
                .gap_2()
                .h(px(26.))
                .px_2()
                .mx_1()
                .rounded(px(5.))
                .cursor_pointer()
                .text_sm()
                .when(active, |d| d.bg(muted.opacity(0.14)).text_color(fg))
                .when(!active, |d| d.text_color(muted))
                .hover(|d| d.bg(muted.opacity(0.08)))
        };
        // Section headers fold their section (Linear's sidebar groups).
        let folded = self.view.folded.clone();
        let is_open = |title: &str| !folded.iter().any(|k| k == &format!("side:{title}"));
        let header = |title: &str| {
            let key = format!("side:{title}");
            let open = !folded.contains(&key);
            div()
                .id(SharedString::from(key.clone()))
                .flex()
                .items_center()
                .gap_1()
                .px_3()
                .pt_4()
                .pb_1()
                .cursor_pointer()
                .text_size(zrem(11.))
                .font_weight(FontWeight::MEDIUM)
                .text_color(muted.opacity(0.8))
                .hover(|d| d.text_color(fg))
                .child(title.to_uppercase())
                .child(
                    Icon::new(if open { IconName::ChevronDown } else { IconName::ChevronRight })
                        .size(px(11.))
                        .text_color(muted.opacity(0.6)),
                )
                .on_click(move |_, _, cx| {
                    let key = key.clone();
                    with_app(cx, |this, cx| {
                        this.toggle_fold(&key);
                        cx.notify();
                    })
                })
        };
        let count = |n: usize| div().ml_auto().flex_none().text_xs().text_color(muted.opacity(0.7)).child(n.to_string());

        let mut col = div()
            .id("sidebar")
            .w(px(236.))
            .flex_none()
            .h_full()
            .border_r_1()
            .border_color(border)
            .overflow_y_scroll()
            .pb_4()
            .child(
                row("home".into(), self.view.page == crate::store::Page::Home)
                    .mt_2()
                    .child(Icon::new(IconName::House).size(px(14.)))
                    .child("Home")
                    .on_click(cx.listener(|this, _, w, cx| {
                        this.view.page = crate::store::Page::Home;
                        this.detail = None;
                        this.save_view();
                        this.board_focus.focus(w, cx);
                        cx.notify();
                    })),
            )
            .child(
                row("deps".into(), self.view.page == crate::store::Page::Dependencies)
                    .child(Icon::new(IconName::Network).size(px(14.)))
                    .child("Dependencies")
                    .on_click(cx.listener(|this, _, w, cx| {
                        this.view.page = crate::store::Page::Dependencies;
                        this.detail = None;
                        this.save_view();
                        this.board_focus.focus(w, cx);
                        cx.notify();
                    })),
            )
            .child(
                row("agents".into(), self.view.page == crate::store::Page::Agents)
                    .child(Icon::new(IconName::Bot).size(px(14.)))
                    .child("Agents")
                    .child(count(self.agent_runs.iter().filter(|r| r.state == crate::agents::RunState::Running).count()))
                    .on_click(cx.listener(|this, _, w, cx| {
                        this.view.page = crate::store::Page::Agents;
                        this.detail = None;
                        this.save_view();
                        this.board_focus.focus(w, cx);
                        cx.notify();
                    })),
            )
            .child(header("Projects"));
        let projects_open = is_open("Projects");
        col = col.when(projects_open, |col| col.child(
                row("proj-all".into(), self.view.project.is_none())
                    .child(Icon::new(IconName::Layers).size(px(14.)))
                    .child("All projects")
                    .child(count(self.board.tickets.len()))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.view.project = None;
                        this.show_board();
                        this.save_view();
                        cx.notify();
                    })),
            ));
        let show_finished = self.view.show_finished_projects;
        let mut finished_hidden = 0;
        let order = if projects_open { self.project_order() } else { Vec::new() };
        for pi in order {
            let p = &self.board.projects[pi];
            let active_project = self.view.project.as_deref() == Some(p.name.as_str());
            if !show_finished && !active_project && self.project_finished(pi) {
                finished_hidden += 1;
                continue;
            }
            let (total, done) = self
                .board
                .tickets
                .iter()
                .filter(|t| t.project == pi)
                .fold((0, 0), |(n, d), t| (n + 1, d + t.category.is_closed() as usize));
            let frac = if total == 0 { 0. } else { done as f32 / total as f32 };
            let active = self.view.project.as_deref() == Some(p.name.as_str());
            let name = p.name.clone();
            let kind = if p.map.is_some() { "wayfinder map" } else { "spec" };
            col = col.child(
                row(("proj", pi).into(), active)
                    .child(icons::ring(frac).size(px(14.)).text_color(icons::tag_color(&p.name)))
                    .child(div().flex_1().min_w_0().truncate().child(p.title.clone()))
                    .children(p.docs.iter().enumerate().map(|(di, d)| {
                        let path = d.path.clone();
                        let open = self.detail.as_ref().is_some_and(|x| x.is_doc && x.path == d.path);
                        let icon = if d.name.eq_ignore_ascii_case("map.md") { IconName::Map } else { IconName::BookOpen };
                        div()
                            .id(("doc", pi * 100 + di))
                            .flex_none()
                            .p(px(3.))
                            .rounded(px(4.))
                            .text_color(if open { fg } else { muted.opacity(0.7) })
                            .when(open, |d| d.bg(muted.opacity(0.18)))
                            .hover(|d| d.bg(muted.opacity(0.18)).text_color(fg))
                            .child(Icon::new(icon).size(px(13.)))
                            .tooltip(tip(format!("Open {}", d.name)))
                            .on_click(cx.listener(move |this, _, w, cx| {
                                cx.stop_propagation();
                                this.open_doc(path.clone(), w, cx);
                            }))
                    }))
                    .child(
                        div()
                            .flex_none()
                            .text_xs()
                            .font_family(crate::settings::mono_font())
                            .text_color(muted.opacity(0.6))
                            .child(p.key.clone()),
                    )
                    .child(count(total))
                    .tooltip(tip(format!(
                        "{} · {kind} · {done} of {total} closed ({:.0}%)",
                        crate::store::tilde(&p.dir),
                        frac * 100.
                    )))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.view.project = if this.view.project.as_ref() == Some(&name) { None } else { Some(name.clone()) };
                        this.show_board();
                        this.save_view();
                        cx.notify();
                    })),
            );
        }

        if projects_open && (finished_hidden > 0 || show_finished) {
            let label = if show_finished { "Hide finished projects".to_string() } else { format!("Show {finished_hidden} finished") };
            col = col.child(
                div()
                    .id("proj-finished")
                    .flex()
                    .items_center()
                    .gap_2()
                    .mx_1()
                    .px_2()
                    .h(px(28.))
                    .rounded(px(6.))
                    .text_xs()
                    .text_color(muted)
                    .cursor_pointer()
                    .hover(|d| d.bg(muted.opacity(0.1)).text_color(fg))
                    .child(Icon::new(if show_finished { IconName::EyeOff } else { IconName::Eye }).size(px(13.)))
                    .child(label)
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.view.show_finished_projects = !this.view.show_finished_projects;
                        this.save_view();
                        cx.notify();
                    })),
            );
        }
        col = col.child(header("Views"));
        for (qi, q) in Quick::ALL.into_iter().enumerate().filter(|_| is_open("Views")) {
            let icon = match q {
                Quick::All => IconName::LayoutGrid,
                Quick::Frontier => IconName::Compass,
                Quick::Blocked => IconName::Lock,
                Quick::Agent => IconName::Bot,
                Quick::Human => IconName::Hand,
                Quick::Recent => IconName::ClockArrowUp,
                Quick::Attention => IconName::TriangleAlert,
            };
            let help = match q {
                Quick::All => "Every ticket",
                Quick::Frontier => "Open, unclaimed and every blocker closed: what can start now",
                Quick::Blocked => "Open with at least one open blocker",
                Quick::Agent => "An agent can take it alone (ready-for-agent, research)",
                Quick::Human => "Needs a person (ready-for-human, grilling, prototype, Needs a human)",
                Quick::Recent => "Changed in the last 24 hours (git), newest first",
                Quick::Attention => "Stale status, blockers not on the board, blocker cycles, stalled work, resolved without an answer",
            };
            col = col.child(
                row(("quick", qi).into(), self.quick == q)
                    .child(Icon::new(icon).size(px(14.)))
                    .child(div().flex_1().min_w_0().truncate().child(q.label()))
                    .child(count(q.count_in(self)))
                    .tooltip(tip(help.to_string()))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.quick = q;
                        if let Some((k, _)) = this.facet_view.take() {
                            this.filters.remove(&k);
                        }
                        this.show_board();
                        cx.notify();
                    })),
            );
        }

        // Facets: short fields that repeat across tickets. One row per
        // value, with how many tickets carry it and how many are closed.
        let project = self.view.project.as_ref().and_then(|n| self.board.projects.iter().position(|p| &p.name == n));
        for (fi, f) in self.board.facets.iter().enumerate() {
            let current = self.filters.get(&f.key).cloned();
            let mut rows: Vec<(String, usize, usize)> = Vec::new();
            for v in &f.values {
                let (n, closed) = self
                    .board
                    .tickets
                    .iter()
                    .filter(|t| project.is_none_or(|p| t.project == p))
                    .filter(|t| t.field_values(&f.key).iter().any(|x| x == v))
                    .fold((0, 0), |(n, c), t| (n + 1, c + t.category.is_closed() as usize));
                if n > 0 {
                    rows.push((v.clone(), n, closed));
                }
            }
            if rows.is_empty() {
                continue;
            }
            let label = model::field_label(&f.key);
            col = col.child(header(&label));
            if !is_open(&label) {
                continue;
            }
            for (i, (v, n, closed)) in rows.into_iter().enumerate() {
                let on = current.as_deref() == Some(v.as_str());
                let frac = closed as f32 / n as f32;
                let key = f.key.clone();
                let val = v.clone();
                let ring_color = if closed == n { icons::status_color(Category::Done, light) } else { accent };
                let mark = if model::is_milestone(&f.key) { icons::diamond(frac) } else { icons::ring(frac) };
                col = col.child(
                    row(("facet", fi * 1000 + i).into(), on)
                        .child(mark.size(px(14.)).text_color(ring_color))
                        .child(div().flex_1().min_w_0().truncate().child(v.clone()))
                        .child(
                            div()
                                .flex_none()
                                .text_xs()
                                .text_color(muted.opacity(0.7))
                                .child(format!("{closed}/{n}")),
                        )
                        .tooltip(tip(format!("{}: {v} · {n} tickets, {closed} closed ({:.0}%)", model::field_label(&f.key), frac * 100.)))
                        .on_click(move |_, _, cx| {
                            let (key, val) = (key.clone(), val.clone());
                            with_app(cx, |this, cx| {
                                this.show_board();
                                if on {
                                    this.filters.remove(&key);
                                    this.facet_view = None;
                                } else {
                                    this.quick = Quick::All;
                                    this.filters.insert(key.clone(), val.clone());
                                    this.facet_view = Some((key, val));
                                }
                                cx.notify();
                            });
                        }),
                );
            }
        }

        col
    }

    // ---------- toolbar ----------

    fn render_toolbar(&self, narrow: bool, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let t = cx.theme();
        let (muted, fg, border, accent) = (t.muted_foreground, t.foreground, t.border, t.accent);
        let prefs = crate::settings::get();
        let mut chips: Vec<(String, Rc<dyn Fn(&mut KuzgunApp)>)> = Vec::new();
        if self.quick != Quick::All {
            chips.push((format!("View: {}", self.quick.label()), Rc::new(|a: &mut KuzgunApp| a.quick = Quick::All)));
        }
        for (k, v) in &self.filters {
            let key = k.clone();
            chips.push((format!("{}: {v}", model::field_label(k)), Rc::new(move |a: &mut KuzgunApp| {
                a.filters.remove(&key);
            })));
        }
        let filter_groups = Rc::new(self.filter_groups());
        let active: Rc<std::collections::BTreeMap<String, String>> = Rc::new(self.filters.clone());
        let filter = Button::new("filter")
            .ghost()
            .small()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_1()
                    .text_sm()
                    .text_color(if self.filters.is_empty() { muted } else { fg })
                    .child(Icon::new(IconName::ListFilter).size(px(13.)))
                    .when(!narrow, |d| d.child("Filter")),
            )
            .dropdown_menu(move |mut menu: PopupMenu, window, cx| {
                for (key, values) in filter_groups.iter() {
                    let label = model::field_label(key);
                    let (key, values, active) = (key.clone(), values.clone(), active.clone());
                    let sub = PopupMenu::build(window, cx, move |mut m, _, _| {
                        for (label, value, n) in &values {
                            let on = active.get(&key) == Some(value);
                            let (k, v) = (key.clone(), value.clone());
                            m = m.item(PopupMenuItem::new(format!("{label}  {n}")).checked(on).on_click(move |_, _, cx| {
                                let (k, v) = (k.clone(), v.clone());
                                with_app(cx, |this, cx| {
                                    if this.filters.get(&k) == Some(&v) {
                                        this.filters.remove(&k);
                                    } else {
                                        this.filters.insert(k, v);
                                    }
                                    this.facet_view = None;
                                    cx.notify();
                                });
                            }));
                        }
                        m
                    });
                    menu = menu.item(PopupMenuItem::submenu(label, sub));
                }
                menu
            });
        let swim = self.view.swimlanes;
        let layout = self.view.layout;
        let ordering = self.view.ordering;
        let completed = self.view.completed;
        let empty = prefs.show_empty_columns;
        let split = prefs.split_blocked;
        let compact = prefs.compact();
        let display = Button::new("display")
            .ghost()
            .small()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_1()
                    .text_sm()
                    .text_color(muted)
                    .child(Icon::new(IconName::SlidersHorizontal).size(px(13.)))
                    .when(!narrow, |d| d.child("Display")),
            )
            .dropdown_menu(move |menu: PopupMenu, _, _| {
                let toggle = |label: &str, on: bool, f: fn(&mut KuzgunApp)| {
                    PopupMenuItem::new(label.to_string()).checked(on).on_click(move |_, _, cx| {
                        with_app(cx, |this, cx| {
                            f(this);
                            this.save_view();
                            cx.notify();
                        })
                    })
                };
                let pref = |label: &str, on: bool, f: fn(&mut crate::settings::Prefs)| {
                    PopupMenuItem::new(label.to_string()).checked(on).on_click(move |_, _, cx| crate::settings::update(cx, f))
                };
                let p = crate::settings::get();
                let set_view = |label: &str, on: bool, f: Rc<dyn Fn(&mut KuzgunApp)>| {
                    PopupMenuItem::new(label.to_string()).checked(on).on_click(move |_, _, cx| {
                        let f = f.clone();
                        with_app(cx, |this, cx| {
                            f(this);
                            this.save_view();
                            cx.notify();
                        })
                    })
                };
                let mut menu = menu
                    .label("Layout")
                    .item(set_view("Board", layout == Layout::Board, Rc::new(|a| a.view.layout = Layout::Board)))
                    .item(set_view("List", layout == Layout::List, Rc::new(|a| a.view.layout = Layout::List)))
                    .separator()
                    .label("Ordering");
                for o in Ordering::ALL {
                    menu = menu.item(set_view(o.label(), ordering == o, Rc::new(move |a| a.view.ordering = o)));
                }
                menu = menu.separator().label("Completed issues");
                for c in Completed::ALL {
                    menu = menu.item(set_view(c.label(), completed == c, Rc::new(move |a| a.view.completed = c)));
                }
                menu.separator()
                    .label("Board options")
                    .item(toggle("Swimlanes by project", swim, |a| a.view.swimlanes = !a.view.swimlanes))
                    .item(pref("Show empty columns", empty, |p| p.show_empty_columns = !p.show_empty_columns))
                    .item(pref("Separate blocked tickets", split, |p| p.split_blocked = !p.split_blocked))
                    .item(pref("Compact cards", compact, |p| {
                        p.density = if p.compact() { "Comfortable".into() } else { "Compact".into() }
                    }))
                    .separator()
                    .label("Card properties")
                    .item(pref("ID", p.card_id, |p| p.card_id = !p.card_id))
                    .item(pref("Project", p.card_project, |p| p.card_project = !p.card_project))
                    .item(pref("Fields", p.card_fields, |p| p.card_fields = !p.card_fields))
                    .item(pref("AFK / HITL", p.card_mode, |p| p.card_mode = !p.card_mode))
                    .item(pref("Sub-task progress", p.card_checklist, |p| p.card_checklist = !p.card_checklist))
                    .item(pref("Relations", p.card_relations, |p| p.card_relations = !p.card_relations))
                    .item(pref("Days in column", p.card_age, |p| p.card_age = !p.card_age))
                    .item(pref("Updated", p.card_updated, |p| p.card_updated = !p.card_updated))
            });
        div()
            .flex()
            .flex_none()
            .items_center()
            .gap_2()
            .h(px(46.))
            .px_3()
            .border_b_1()
            .border_color(border)
            .child(
                div().w(px(if narrow { 220. } else { 300. })).flex_none().child(
                    Input::new(&self.search)
                        .small()
                        .h(px(30.))
                        .prefix(Icon::new(IconName::Search).size(px(13.)).text_color(muted))
                        .cleanable(true),
                ),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .items_center()
                    .gap_1()
                    .overflow_hidden()
                    .children(chips.into_iter().enumerate().map(|(i, (label, clear))| {
                        div()
                            .id(("chip", i))
                            .flex()
                            .flex_none()
                            .items_center()
                            .gap_1()
                            .h(px(24.))
                            .pl_2()
                            .pr_1()
                            .rounded(px(12.))
                            .bg(accent.opacity(0.14))
                            .text_xs()
                            .text_color(fg)
                            .child(label)
                            .child(
                                div()
                                    .id(("chip-x", i))
                                    .cursor_pointer()
                                    .rounded_full()
                                    .p(px(2.))
                                    .hover(|d| d.bg(muted.opacity(0.2)))
                                    .child(Icon::new(IconName::X).size(px(11.)))
                                    .on_click(move |_, _, cx| {
                                        let clear = clear.clone();
                                        with_app(cx, |this, cx| {
                                            clear(this);
                                            cx.notify();
                                        })
                                    }),
                            )
                    })),
            )
            .child(filter)
            .child(display)
    }

    /// Filter menu entries: each key with its values, labels and counts in
    /// the current project.
    fn filter_groups(&self) -> Vec<(String, Vec<(String, String, usize)>)> {
        let scope = self.project_ix();
        let ids: Vec<usize> = (0..self.board.tickets.len())
            .filter(|&i| scope.is_none_or(|p| self.board.tickets[i].project == p))
            .collect();
        let count = |f: &dyn Fn(usize) -> bool| ids.iter().filter(|&&i| f(i)).count();
        let mut groups = Vec::new();

        let mut statuses: Vec<(String, usize)> = Vec::new();
        for &i in &ids {
            let k = &self.board.tickets[i].status_key;
            match statuses.iter_mut().find(|(s, _)| s == k) {
                Some((_, n)) => *n += 1,
                None => statuses.push((k.clone(), 1)),
            }
        }
        groups.push((
            crate::app::FILTER_STATUS.to_string(),
            statuses.into_iter().map(|(k, n)| (status_label(&k), k, n)).collect(),
        ));

        let rel = |label: &str, n: usize| (label.to_string(), label.to_string(), n);
        groups.push((
            crate::app::FILTER_RELATIONS.to_string(),
            vec![
                rel("Blocked", count(&|i| self.idx.blocked.get(i).copied().unwrap_or(false))),
                rel("Blocking others", count(&|i| self.idx.blocking_open.get(i).copied().unwrap_or(0) > 0)),
                rel("No relations", count(&|i| self.board.tickets[i].blocked_by.is_empty() && self.board.tickets[i].blocks.is_empty())),
            ],
        ));

        let modes: Vec<(String, String, usize)> = [model::Mode::Afk, model::Mode::Hitl]
            .into_iter()
            .map(|m| (m.label().to_string(), m.label().to_string(), count(&|i| self.idx.modes.get(i).copied().flatten() == Some(m))))
            .filter(|(_, _, n)| *n > 0)
            .collect();
        if !modes.is_empty() {
            groups.push((crate::app::FILTER_MODE.to_string(), modes));
        }

        for f in &self.board.facets {
            let values: Vec<(String, String, usize)> = f
                .values
                .iter()
                .map(|v| (v.clone(), v.clone(), count(&|i| self.board.tickets[i].field_values(&f.key).iter().any(|x| x == v))))
                .filter(|(_, _, n)| *n > 0)
                .collect();
            if !values.is_empty() {
                groups.push((f.key.clone(), values));
            }
        }
        groups
    }

    /// Wayfinder project: the map's destination above the board.
    fn render_map_banner(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let p = self
            .view
            .project
            .as_ref()
            .and_then(|n| self.board.projects.iter().find(|p| &p.name == n))?;
        let map = p.map.as_ref()?;
        let t = cx.theme();
        let (muted, fg, border) = (t.muted_foreground, t.foreground, t.border);
        let path = map.path.clone();
        Some(
            div()
                .id("map-banner")
                .flex()
                .flex_none()
                .items_start()
                .gap_3()
                .px_4()
                .py_2()
                .border_b_1()
                .border_color(border)
                .cursor_pointer()
                .hover(|d| d.bg(muted.opacity(0.05)))
                .child(Icon::new(IconName::Flag).size(px(14.)).text_color(muted).mt(px(2.)))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .gap_0p5()
                        .child(div().text_xs().text_color(muted).child("Destination"))
                        .child(div().text_sm().text_color(fg).line_clamp(2).child(map.destination.clone())),
                )
                .child(
                    div()
                        .flex_none()
                        .text_xs()
                        .text_color(muted)
                        .child(format!("{} decisions · {} in fog · {} out of scope", map.decisions, map.fog.len(), map.out_of_scope.len())),
                )
                .on_click(cx.listener(move |this, _, w, cx| this.open_doc(path.clone(), w, cx)))
                .into_any_element(),
        )
    }

    // ---------- columns ----------

    fn fog_items(&self) -> Option<(Vec<String>, Vec<String>)> {
        let p = self
            .view
            .project
            .as_ref()
            .and_then(|n| self.board.projects.iter().find(|p| &p.name == n))?;
        let m = p.map.as_ref()?;
        Some((m.fog.clone(), m.out_of_scope.clone()))
    }

    fn render_columns(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let groups = self.grouped(cx);
        if self.board.tickets.is_empty() {
            return self.render_empty(cx).into_any_element();
        }
        self.sync_layout_sig();
        if self.quick != Quick::All || self.active_facet().is_some() {
            return self.render_view_results(cx).into_any_element();
        }
        if self.view.layout == Layout::List {
            return self.render_list(groups, cx).into_any_element();
        }
        if self.view.swimlanes {
            return self.render_lanes(groups, window, cx).into_any_element();
        }
        let width = crate::settings::get().column_width;
        let fog = self.fog_items();
        let mut out_of_scope: Vec<String> = Vec::new();
        let mut total = BOARD_PAD * 2.;
        let mut cols: Vec<AnyElement> = Vec::new();
        if let Some((fog, out)) = fog {
            if !fog.is_empty() {
                cols.push(self.render_note_column("fog", "Not yet specified", IconName::CloudFog, fog, cx));
                total += width + COL_GAP;
            }
            out_of_scope = out;
        }
        // Hidden columns leave the board and gather in one strip at its end.
        let mut hidden: Vec<(Column, usize)> = Vec::new();
        for (ci, (col, items)) in groups.into_iter().enumerate() {
            if self.view.collapsed.contains(&col.status) {
                hidden.push((col, items.len()));
                continue;
            }
            total += width + COL_GAP;
            cols.push(self.render_column(ci, col, items, cx));
        }
        // Ruled-out items close the board: they are settled, not work.
        if !out_of_scope.is_empty() {
            cols.push(self.render_note_column("out", "Out of scope", IconName::Ban, out_of_scope, cx));
            total += width + COL_GAP;
        }
        if !hidden.is_empty() {
            cols.push(self.render_hidden_columns(hidden, cx));
            total += HIDDEN_W + COL_GAP;
        }
        div()
            .id("columns")
            .flex_1()
            .min_h_0()
            .flex()
            .overflow_x_scroll()
            .restrict_scroll_to_axis()
            .track_scroll(&self.board_scroll)
            .child(
                div()
                    .flex_none()
                    .w(px(total))
                    .h_full()
                    .flex()
                    .flex_row()
                    .gap(px(COL_GAP))
                    .p(px(BOARD_PAD))
                    .children(cols),
            )
            .into_any_element()
    }

    /// "N tickets hidden by filters", with a way out. Views and sidebar
    /// facets show their own results list instead.
    fn render_hidden_footer(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        if self.quick != Quick::All || self.active_facet().is_some() || !self.filtering(cx) {
            return None;
        }
        let hidden = self.in_scope().saturating_sub(self.visible(cx).len());
        if hidden == 0 {
            return None;
        }
        let t = cx.theme();
        let (muted, border, accent) = (t.muted_foreground, t.border, t.accent);
        Some(
            div()
                .flex()
                .flex_none()
                .items_center()
                .justify_center()
                .gap_2()
                .h(px(34.))
                .border_t_1()
                .border_color(border)
                .text_sm()
                .text_color(muted)
                .child(format!("{hidden} ticket{} hidden by filters", if hidden == 1 { "" } else { "s" }))
                .child("·")
                .child(
                    div()
                        .id("clear-filters")
                        .text_color(accent)
                        .cursor_pointer()
                        .hover(|d| d.underline())
                        .child("Clear filters")
                        .on_click(cx.listener(|this, _, w, cx| this.clear_filters(w, cx))),
                )
                .into_any_element(),
        )
    }

    fn render_hidden_columns(&self, hidden: Vec<(Column, usize)>, cx: &mut Context<Self>) -> AnyElement {
        let t = cx.theme();
        let (muted, fg) = (t.muted_foreground, t.foreground);
        let light = crate::settings::is_light(cx);
        div()
            .flex_none()
            .w(px(HIDDEN_W))
            .flex()
            .flex_col()
            .gap_0p5()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_1p5()
                    .px_2()
                    .h(px(34.))
                    .text_sm()
                    .text_color(muted)
                    .child(Icon::new(IconName::EyeOff).size(px(14.)))
                    .child("Hidden columns"),
            )
            .children(hidden.into_iter().enumerate().map(|(i, (col, n))| {
                let status = col.status.clone();
                div()
                    .id(("hidden-col", i))
                    .flex()
                    .items_center()
                    .gap_2()
                    .px_2()
                    .h(px(32.))
                    .rounded(px(6.))
                    .cursor_pointer()
                    .hover(|d| d.bg(muted.opacity(0.1)))
                    .child(column_icon(&col, light).size(px(14.)))
                    .child(div().flex_1().min_w_0().truncate().text_sm().text_color(fg).child(status_label(&col.status)))
                    .child(div().text_xs().text_color(muted).child(n.to_string()))
                    .tooltip(tip("Show this column".to_string()))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.view.collapsed.retain(|s| s != &status);
                        this.save_view();
                        cx.notify();
                    }))
            }))
            .into_any_element()
    }

    /// Card heights depend on these; when they change, remeasure the lists.
    fn sync_layout_sig(&self) {
        use std::hash::{Hash, Hasher};
        let p = crate::settings::get();
        let mut h = std::collections::hash_map::DefaultHasher::new();
        (
            p.column_width as u32,
            p.ui_font_size as u32,
            p.compact(),
            p.card_id,
            p.card_project,
            p.card_fields,
            p.card_mode,
            p.card_checklist,
            p.card_relations,
            p.card_age,
            p.card_updated,
            self.view.project.clone(),
        )
            .hash(&mut h);
        let sig = h.finish();
        if self.layout_sig.get() != sig {
            self.layout_sig.set(sig);
            for (state, _) in self.lists.borrow().values() {
                state.remeasure();
            }
        }
    }

    fn render_empty(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let muted = cx.theme().muted_foreground;
        div()
            .flex_1()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap_2()
            .text_color(muted)
            .child(Icon::new(IconName::SquareKanban).size(px(40.)).text_color(muted.opacity(0.5)))
            .child(if self.loading { "Reading tickets…" } else { "No tickets in this folder" })
            .when(!self.loading, |d| {
                d.child(
                    div()
                        .text_xs()
                        .max_w(px(420.))
                        .text_center()
                        .child("Kuzgun reads tickets from the issues/ folder of each feature, as mattpocock/skills writes them: .scratch/<feature>/issues/NN-slug.md."),
                )
            })
    }

    fn column_header(&self, ci: usize, col: &Column, n: usize, collapsible: bool, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let group = SharedString::from(format!("colhead-{ci}"));
        let t = cx.theme();
        let (muted, fg) = (t.muted_foreground, t.foreground);
        let light = crate::settings::is_light(cx);
        let status = col.status.clone();
        let meaning = self
            .board
            .roles
            .get(&col.status)
            .map(|(role, m)| if role == &col.status { m.clone() } else { format!("{role}: {m}") })
            .or_else(|| model::role_meaning(&col.status).map(str::to_string));
        let help = format!(
            "{} · {}{}",
            col.status,
            col.category.label(),
            meaning.map(|m| format!(" · {m}")).unwrap_or_default()
        );
        div()
            .id(("col-head", ci))
            .group(group.clone())
            .flex()
            .flex_none()
            .items_center()
            .gap_2()
            .h(px(34.))
            .px_2()
            .tooltip(tip(help))
            .child(
                column_icon(col, light).size(px(14.)),
            )
            .child(div().min_w_0().text_sm().text_color(fg).truncate().child(status_label(&col.status)))
            .child(div().flex_none().text_sm().text_color(muted.opacity(0.7)).child(n.to_string()))
            .child(div().flex_1())
            .when(collapsible, |d| {
                d.child(
                    div().invisible().group_hover(group.clone(), |d| d.visible()).child(
                    Button::new(("col-hide", ci))
                        .ghost()
                        .xsmall()
                        .icon(Icon::new(IconName::ChevronsLeft).text_color(muted))
                        .tooltip("Collapse column")
                        .on_click(cx.listener(move |this, _, _, cx| {
                            if !this.view.collapsed.contains(&status) {
                                this.view.collapsed.push(status.clone());
                            }
                            this.save_view();
                            cx.notify();
                        }))),
                )
            })
    }

    /// A read-only column of map bullets (fog, out of scope).
    fn render_note_column(&self, id: &'static str, title: &str, icon: IconName, items: Vec<String>, cx: &mut Context<Self>) -> AnyElement {
        let t = cx.theme();
        let (muted, fg, border) = (t.muted_foreground, t.foreground, t.border);
        let width = crate::settings::get().column_width;
        let style = crate::detail::md_style(cx);
        div()
            .id(id)
            .flex_none()
            .w(px(width))
            .h_full()
            .flex()
            .flex_col()
            .rounded(px(8.))
            .bg(muted.opacity(0.03))
            .border_1()
            .border_dashed()
            .border_color(muted.opacity(0.18))
            .child(
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .gap_2()
                    .h(px(34.))
                    .px_2()
                    .child(Icon::new(icon).size(px(14.)).text_color(muted))
                    .child(div().text_sm().text_color(fg).child(title.to_string()))
                    .child(div().text_sm().text_color(muted.opacity(0.7)).child(items.len().to_string())),
            )
            .child(
                div()
                    .id((id, 1usize))
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .restrict_scroll_to_axis()
                    .flex()
                    .flex_col()
                    .gap(px(6.))
                    .px(px(6.))
                    .pb(px(6.))
                    .children(items.into_iter().enumerate().map(|(i, text)| {
                        div()
                            .id((id, 100 + i))
                            .px(px(10.))
                            .py(px(8.))
                            .rounded(px(8.))
                            .border_1()
                            .border_dashed()
                            .border_color(border)
                            .text_sm()
                            .text_color(muted)
                            .when(id == "out", |d| d.max_h(px(44.)).overflow_hidden())
                            .child(gpui_kit::component::text::TextView::markdown((id, 1000 + i), text).style(style.clone()))
                    })),
            )
            .into_any_element()
    }

    fn render_column(&self, ci: usize, col: Column, items: Vec<usize>, cx: &mut Context<Self>) -> AnyElement {
        let t = cx.theme();
        let muted = t.muted_foreground;
        let light = crate::settings::is_light(cx);
        let width = crate::settings::get().column_width;
        let base = div()
            .id(("col", ci))
            .flex_none()
            .flex()
            .flex_col()
            .h_full()
            .rounded(px(8.))
            .bg(muted.opacity(if light { 0.06 } else { 0.045 }));

        let n = items.len();
        let header = self.column_header(ci, &col, n, true, cx);
        let body: AnyElement = if items.is_empty() {
            div()
                .mx(px(6.))
                .h(px(56.))
                .rounded(px(8.))
                .border_1()
                .border_dashed()
                .border_color(muted.opacity(0.2))
                .flex()
                .items_center()
                .justify_center()
                .text_xs()
                .text_color(muted.opacity(0.5))
                .child("Empty")
                .into_any_element()
        } else {
            let items = Rc::new(items);
            let state = {
                let mut lists = self.lists.borrow_mut();
                let e = lists
                    .entry(col.status.clone())
                    .or_insert_with(|| (ListState::new(0, ListAlignment::Top, px(600.)), Rc::new(Vec::new())));
                if *e.1 != *items {
                    e.0.reset(items.len());
                    e.1 = items.clone();
                }
                e.0.clone()
            };
            let view = cx.entity().downgrade();
            list(state, move |i, window, cx| {
                let Some(&ix) = items.get(i) else {
                    return div().into_any_element();
                };
                view.update(cx, |this, cx| {
                    div().px(px(6.)).pb(px(6.)).child(this.render_card(ix, false, window, cx)).into_any_element()
                })
                .unwrap_or_else(|_| div().into_any_element())
            })
            .flex_1()
            .min_h_0()
            .into_any_element()
        };
        base.w(px(width)).child(header).child(body).into_any_element()
    }

    fn render_lanes(&mut self, groups: Vec<(Column, Vec<usize>)>, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let t = cx.theme();
        let (muted, fg, border) = (t.muted_foreground, t.foreground, t.border);
        let light = crate::settings::is_light(cx);
        let width = crate::settings::get().column_width;
        let cols: Vec<Column> = groups.iter().map(|(c, _)| c.clone()).collect();
        let ncols = cols.len();
        let total = BOARD_PAD * 2. + ncols as f32 * (width + COL_GAP);
        let header = div()
            .flex()
            .flex_none()
            .flex_row()
            .gap(px(COL_GAP))
            .px(px(BOARD_PAD))
            .pt(px(BOARD_PAD))
            .children(cols.iter().enumerate().map(|(ci, c)| {
                let n = groups[ci].1.len();
                div().w(px(width)).flex_none().child(self.column_header(ci, c, n, false, cx))
            }));
        let mut lanes = div().flex().flex_col().gap_1().px(px(BOARD_PAD)).pb(px(BOARD_PAD));
        for (pi, p) in self.board.projects.iter().enumerate() {
            let per: Vec<Vec<usize>> = groups
                .iter()
                .map(|(_, v)| v.iter().copied().filter(|&i| self.board.tickets[i].project == pi).collect())
                .collect();
            let lane_total: usize = per.iter().map(Vec::len).sum();
            if lane_total == 0 {
                continue;
            }
            let collapsed = self.collapsed_lanes.contains(&pi);
            lanes = lanes.child(
                div()
                    .id(("lane-head", pi))
                    .flex()
                    .items_center()
                    .gap_2()
                    .h(px(30.))
                    .mt_2()
                    .cursor_pointer()
                    .border_b_1()
                    .border_color(border)
                    .child(
                        Icon::new(if collapsed { IconName::ChevronRight } else { IconName::ChevronDown })
                            .size(px(13.))
                            .text_color(muted),
                    )
                    .child(div().size(px(8.)).rounded_full().bg(icons::tag_color(&p.name)))
                    .child(div().text_sm().text_color(fg).child(p.title.clone()))
                    .child(div().text_xs().font_family(crate::settings::mono_font()).text_color(muted).child(p.key.clone()))
                    .child(div().text_xs().text_color(muted.opacity(0.7)).child(lane_total.to_string()))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if let Some(i) = this.collapsed_lanes.iter().position(|&x| x == pi) {
                            this.collapsed_lanes.remove(i);
                        } else {
                            this.collapsed_lanes.push(pi);
                        }
                        cx.notify();
                    })),
            );
            if collapsed {
                continue;
            }
            let row = div().flex().flex_row().gap(px(COL_GAP)).children((0..ncols).map(|ci| {
                div()
                    .id(("lane-col", pi * 1000 + ci))
                    .w(px(width))
                    .flex_none()
                    .flex()
                    .flex_col()
                    .gap(px(6.))
                    .p(px(6.))
                    .min_h(px(48.))
                    .rounded(px(8.))
                    .bg(muted.opacity(if light { 0.06 } else { 0.045 }))
                    .children(per[ci].iter().map(|&ix| self.render_card(ix, true, window, cx)))
                    .into_any_element()
            }));
            lanes = lanes.child(row);
        }
        div()
            .id("lanes-x")
            .flex_1()
            .min_h_0()
            .flex()
            .overflow_x_scroll()
            .restrict_scroll_to_axis()
            .child(
                div()
                    .flex_none()
                    .w(px(total))
                    .h_full()
                    .flex()
                    .flex_col()
                    .child(header)
                    .child(
                        div()
                            .id("lanes-y")
                            .flex_1()
                            .min_h_0()
                            .overflow_y_scroll()
                            .restrict_scroll_to_axis()
                            .child(lanes),
                    ),
            )
    }

    // ---------- view results ----------

    /// A sidebar view is a query across statuses: its tickets as one flat
    /// list, like search results, with what the view means on top.
    /// The sidebar facet shown as a flat list, while its filter is still on.
    fn active_facet(&self) -> Option<(String, String)> {
        self.facet_view.clone().filter(|(k, v)| self.quick == Quick::All && self.filters.get(k) == Some(v))
    }

    fn render_view_results(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let t = cx.theme();
        let (muted, fg, border) = (t.muted_foreground, t.foreground, t.border);
        let q = self.quick;
        let mut items = self.visible(cx);
        if q == Quick::Blocked {
            items.sort_by_key(|&i| self.idx.open_blockers.get(i).map(Vec::len).unwrap_or(0));
        }
        let help = match q {
            Quick::All => "",
            Quick::Frontier => "Open, unclaimed and every blocker closed: these can start now.",
            Quick::Blocked => "Open with at least one open blocker. Closest to ready first.",
            Quick::Agent => "An agent can take these alone (ready-for-agent, research).",
            Quick::Human => "These need a person (ready-for-human, grilling, prototype, Needs a human).",
            Quick::Recent => "Changed in the last 24 hours, newest first.",
            Quick::Attention => "Something is off: a stale status, a blocker not on the board, a cycle, stalled work.",
        };
        let facet = self.active_facet();
        let closed = items.iter().filter(|&&i| self.board.tickets[i].category.is_closed()).count();
        let (title, help, list_key) = match &facet {
            Some((k, v)) => (
                format!("{}: {v}", model::field_label(k)),
                format!("{closed} of {} closed.", items.len()),
                format!("__facet_{k}_{v}__"),
            ),
            None => (q.label().to_string(), help.to_string(), format!("__view_{}__", q.label())),
        };
        let items = Rc::new(items);
        let n = items.len();
        let state = {
            let mut lists = self.lists.borrow_mut();
            let e = lists
                .entry(list_key)
                .or_insert_with(|| (ListState::new(0, ListAlignment::Top, px(800.)), Rc::new(Vec::new())));
            if *e.1 != *items {
                e.0.reset(items.len());
                e.1 = items.clone();
            }
            e.0.clone()
        };
        let view = cx.entity().downgrade();
        div()
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .child(
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .gap_3()
                    .px_4()
                    .py_3()
                    .border_b_1()
                    .border_color(border)
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .gap_0p5()
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap_2()
                                    .child(div().text_base().font_weight(FontWeight::SEMIBOLD).text_color(fg).child(title))
                                    .child(div().text_sm().text_color(muted).child(n.to_string())),
                            )
                            .child(div().text_sm().text_color(muted).child(help)),
                    )
                    .child(
                        Button::new("view-close")
                            .ghost()
                            .small()
                            .icon(Icon::new(IconName::X).text_color(muted))
                            .label("Back to board")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.quick = Quick::All;
                                if let Some((k, _)) = this.facet_view.take() {
                                    this.filters.remove(&k);
                                }
                                cx.notify();
                            })),
                    ),
            )
            .when(n == 0, |d| d.child(div().p_6().text_sm().text_color(muted).child("Nothing here right now.")))
            .child(
                list(state, move |i, window, cx| {
                    let Some(&ix) = items.get(i) else {
                        return div().into_any_element();
                    };
                    view.update(cx, |this, cx| {
                        let row = this.list_row(ix, window, cx);
                        let reasons = if this.quick == Quick::Attention {
                            this.idx.attention.get(ix).cloned().unwrap_or_default()
                        } else {
                            Vec::new()
                        };
                        if reasons.is_empty() {
                            return row;
                        }
                        let muted = cx.theme().muted_foreground;
                        div()
                            .w_full()
                            .flex()
                            .flex_col()
                            .child(row)
                            .child(
                                div()
                                    .w_full()
                                    .flex()
                                    .flex_col()
                                    .gap_0p5()
                                    .pl(px(96.))
                                    .pr_4()
                                    .pb_2()
                                    .children(reasons.into_iter().map(|r| {
                                        div()
                                            .flex()
                                            .items_center()
                                            .gap_1p5()
                                            .text_xs()
                                            .text_color(muted)
                                            .child(Icon::new(IconName::TriangleAlert).size(px(11.)))
                                            .child(r)
                                    })),
                            )
                            .into_any_element()
                    })
                    .unwrap_or_else(|_| div().into_any_element())
                })
                .flex_1()
                .min_h_0(),
            )
    }

    // ---------- list ----------

    /// Linear's list layout: one virtualized list, a header row per status
    /// (folds with the column's collapse state) and one row per ticket.
    fn render_list(&self, groups: Vec<(Column, Vec<usize>)>, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        // Rows: a header is `HEADER + group`, a ticket its index.
        const HEADER: usize = usize::MAX / 2;
        let mut rows: Vec<usize> = Vec::new();
        for (gi, (col, items)) in groups.iter().enumerate() {
            if items.is_empty() {
                continue;
            }
            rows.push(HEADER + gi);
            if !self.view.collapsed.contains(&col.status) {
                rows.extend(items.iter().copied());
            }
        }
        let rows = Rc::new(rows);
        let groups = Rc::new(groups);
        let state = {
            let mut lists = self.lists.borrow_mut();
            let e = lists
                .entry("__list__".into())
                .or_insert_with(|| (ListState::new(0, ListAlignment::Top, px(800.)), Rc::new(Vec::new())));
            if *e.1 != *rows {
                e.0.reset(rows.len());
                e.1 = rows.clone();
            }
            e.0.clone()
        };
        let view = cx.entity().downgrade();
        div().flex_1().min_h_0().flex().flex_col().child(
            list(state, move |i, window, cx| {
                let Some(&r) = rows.get(i) else {
                    return div().into_any_element();
                };
                let groups = groups.clone();
                view.update(cx, |this, cx| {
                    if r >= HEADER {
                        let (col, items) = &groups[r - HEADER];
                        this.list_header(col, items.len(), cx)
                    } else {
                        this.list_row(r, window, cx)
                    }
                })
                .unwrap_or_else(|_| div().into_any_element())
            })
            .flex_1()
            .min_h_0(),
        )
    }

    fn list_header(&self, col: &Column, n: usize, cx: &mut Context<Self>) -> AnyElement {
        let t = cx.theme();
        let (muted, fg, border) = (t.muted_foreground, t.foreground, t.border);
        let light = crate::settings::is_light(cx);
        let status = col.status.clone();
        let open = !self.view.collapsed.contains(&col.status);
        div()
            .id(SharedString::from(format!("lh-{}", col.status)))
            .w_full()
            .flex()
            .items_center()
            .gap_2()
            .h(px(36.))
            .px_4()
            .bg(muted.opacity(0.06))
            .border_b_1()
            .border_color(border)
            .cursor_pointer()
            .child(
                Icon::new(if open { IconName::ChevronDown } else { IconName::ChevronRight })
                    .size(px(12.))
                    .text_color(muted),
            )
            .child(column_icon(col, light).size(px(14.)))
            .child(div().text_sm().font_weight(FontWeight::MEDIUM).text_color(fg).child(status_label(&col.status)))
            .child(div().text_sm().text_color(muted).child(n.to_string()))
            .on_click(cx.listener(move |this, _, _, cx| {
                if let Some(i) = this.view.collapsed.iter().position(|s| s == &status) {
                    this.view.collapsed.remove(i);
                } else {
                    this.view.collapsed.push(status.clone());
                }
                this.save_view();
                cx.notify();
            }))
            .into_any_element()
    }

    fn list_row(&self, ix: usize, _window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let t = &self.board.tickets[ix];
        let theme = cx.theme();
        let (muted, fg, border, accent) = (theme.muted_foreground, theme.foreground, theme.border, theme.accent);
        let light = crate::settings::is_light(cx);
        let selected = self.selected.as_ref() == Some(&t.path);
        let blocked = self.idx.open_blockers.get(ix).map(Vec::len).unwrap_or(0);
        let (done, total) = t.checklist_counts();
        let project = &self.board.projects[t.project];
        let milestone = self
            .board
            .facets
            .iter()
            .find(|f| model::is_milestone(&f.key))
            .and_then(|f| t.field_values(&f.key).into_iter().next());
        let people = self.assignees(ix).unwrap_or_default();
        let flash = self.flash.contains_key(&t.path);
        let path = t.path.clone();
        div()
            .id(("lr", ix))
            .w_full()
            .flex()
            .items_center()
            .gap_3()
            .h(px(40.))
            .px_4()
            .border_b_1()
            .border_color(border)
            .cursor_pointer()
            .when(selected, |d| d.bg(accent.opacity(0.12)))
            .when(flash, |d| d.bg(accent.opacity(0.18)))
            .hover(|d| d.bg(muted.opacity(0.07)))
            .child(
                div()
                    .w(px(68.))
                    .flex_none()
                    .text_xs()
                    .font_family(crate::settings::mono_font())
                    .text_color(muted)
                    .child(t.key.clone()),
            )
            .child(icons::status_icon(t.category).size(px(15.)).text_color(icons::status_color(t.category, light)))
            .child(div().flex_1().min_w_0().truncate().text_sm().text_color(fg).child(t.title.clone()))
            .when(t.agent.as_ref().is_some_and(|a| a.state == crate::agents::RunState::Running), |d| {
                d.child(
                    div()
                        .flex()
                        .flex_none()
                        .items_center()
                        .gap_1()
                        .px_1p5()
                        .rounded(px(5.))
                        .bg(theme.green.opacity(0.14))
                        .text_xs()
                        .text_color(theme.green)
                        .child(Icon::new(IconName::Bot).size(px(12.)))
                        .child("Agent working"),
                )
            })
            .when(blocked > 0, |d| {
                d.child(
                    div()
                        .flex()
                        .flex_none()
                        .items_center()
                        .gap_0p5()
                        .text_xs()
                        .text_color(theme.red)
                        .child(Icon::new(IconName::Lock).size(px(12.)))
                        .child(blocked.to_string()),
                )
            })
            .when_some(milestone, |d, m| {
                d.child(
                    div()
                        .flex()
                        .flex_none()
                        .items_center()
                        .gap_1()
                        .px_1p5()
                        .h(px(20.))
                        .rounded(px(5.))
                        .border_1()
                        .border_color(border)
                        .text_xs()
                        .text_color(muted)
                        .child(icons::diamond(0.5).size(px(11.)).text_color(accent))
                        .child(m),
                )
            })
            .when(total > 0, |d| {
                d.child(
                    div()
                        .flex()
                        .flex_none()
                        .items_center()
                        .gap_1()
                        .w(px(52.))
                        .text_xs()
                        .text_color(muted)
                        .child(icons::ring(done as f32 / total as f32).size(px(12.)).text_color(muted))
                        .child(format!("{done}/{total}")),
                )
            })
            .when(self.view.project.is_none(), |d| {
                d.child(
                    div()
                        .flex()
                        .flex_none()
                        .items_center()
                        .gap_1p5()
                        .w(px(150.))
                        .text_xs()
                        .text_color(muted)
                        .child(div().size(px(7.)).rounded_full().bg(icons::tag_color(&project.name)))
                        .child(div().truncate().child(project.title.clone())),
                )
            })
            .child(
                div()
                    .w(px(64.))
                    .flex_none()
                    .flex()
                    .justify_end()
                    .when(!people.is_empty(), |d| d.child(avatar_group(&people, 20., theme.background, 3, muted))),
            )
            .child(
                div()
                    .w(px(56.))
                    .flex_none()
                    .text_xs()
                    .text_color(muted)
                    .child(crate::app::ago_label(now_unix() - self.updated_at(ix))),
            )
            .on_click(cx.listener(move |this, _, w, cx| {
                this.open_detail(path.clone(), true, w, cx);
                this.board_focus.focus(w, cx);
            }))
            .into_any_element()
    }

    // ---------- cards ----------

    fn render_card(&self, ix: usize, in_lane: bool, _window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let t = &self.board.tickets[ix];
        let theme = cx.theme();
        let (muted, fg, border, accent, red, yellow) =
            (theme.muted_foreground, theme.foreground, theme.border, theme.accent, theme.red, theme.yellow);
        let light = crate::settings::is_light(cx);
        let prefs = crate::settings::get();
        let selected = self.selected.as_ref() == Some(&t.path);
        let open = self.detail.as_ref().is_some_and(|d| !d.is_doc && d.path == t.path);
        let flash = self
            .flash
            .get(&t.path)
            .map(|at| 1. - (at.elapsed().as_secs_f32() / 2.6).min(1.))
            .unwrap_or(0.);
        let blocked = self.idx.blocked.get(ix).copied().unwrap_or(false);
        let frontier = self.idx.frontier.get(ix).copied().unwrap_or(false);
        let project = &self.board.projects[t.project];
        let (done, total) = t.checklist_counts();
        let card_bg: Hsla = if light { theme.background } else { rgb(0x0E0E10).into() };
        let chip = |text: String, color: Option<Hsla>| {
            div()
                .flex()
                .flex_none()
                .items_center()
                .gap_1()
                .h(px(20.))
                .px(px(6.))
                .rounded(px(5.))
                .border_1()
                .border_color(border)
                .text_xs()
                .text_color(muted)
                .when_some(color, |d, c| d.child(div().size(px(6.)).rounded_full().bg(c)))
                .child(text)
        };

        let mut chips: Vec<AnyElement> = Vec::new();
        if prefs.card_project && self.view.project.is_none() && !in_lane {
            chips.push(chip(project.title.clone(), Some(icons::tag_color(&project.name))).into_any_element());
        }
        if prefs.card_mode
            && let Some(m) = self.idx.modes.get(ix).copied().flatten()
        {
            let (icon, c) = match m {
                Mode::Afk => (IconName::Bot, theme.blue),
                Mode::Hitl => (IconName::User, accent),
            };
            chips.push(
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .gap_1()
                    .h(px(20.))
                    .px(px(6.))
                    .rounded(px(5.))
                    .bg(c.opacity(0.14))
                    .text_xs()
                    .text_color(c)
                    .child(Icon::new(icon).size(px(11.)))
                    .child(m.label())
                    .into_any_element(),
            );
        }
        if prefs.card_fields {
            for f in &self.board.facets {
                for v in t.field_values(&f.key) {
                    if model::is_milestone(&f.key) {
                        chips.push(
                            div()
                                .flex()
                                .flex_none()
                                .items_center()
                                .gap_1()
                                .h(px(20.))
                                .px(px(6.))
                                .rounded(px(5.))
                                .border_1()
                                .border_color(border)
                                .text_xs()
                                .text_color(muted)
                                .child(icons::diamond(0.5).size(px(11.)).text_color(accent))
                                .child(v.clone())
                                .into_any_element(),
                        );
                        continue;
                    }
                    let is_kind = f.key.eq_ignore_ascii_case("type");
                    chips.push(chip(v.clone(), (!is_kind).then(|| icons::tag_color(&v))).into_any_element());
                }
            }
        }
        if prefs.card_checklist && total > 0 {
            let frac = done as f32 / total as f32;
            let c = if done == total { icons::status_color(Category::Done, light) } else { muted };
            chips.push(
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .gap_1()
                    .h(px(20.))
                    .px(px(6.))
                    .rounded(px(5.))
                    .border_1()
                    .border_color(border)
                    .text_xs()
                    .text_color(muted)
                    .child(icons::ring(frac).size(px(12.)).text_color(c))
                    .child(format!("{done}/{total}"))
                    .into_any_element(),
            );
        }
        if t.comments > 0 {
            chips.push(
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .gap_1()
                    .text_xs()
                    .text_color(muted)
                    .child(Icon::new(IconName::MessageSquare).size(px(12.)))
                    .child(t.comments.to_string())
                    .into_any_element(),
            );
        }
        if prefs.card_age
            && !t.category.is_closed()
            && let Some(since) = self.ages.get(&t.path).map(|g| g.since)
        {
            let days = ((now_unix() - since).max(0) / 86_400) as u32;
            if days >= 1 {
                chips.push(
                    div()
                        .id(("age", ix))
                        .flex()
                        .flex_none()
                        .items_center()
                        .gap_1()
                        .h(px(20.))
                        .text_xs()
                        .text_color(muted)
                        .child(Icon::new(IconName::Clock).size(px(11.)))
                        .child(format!("{days}d"))
                        .tooltip(tip(format!("{} for {days} days", status_label(&t.status_key))))
                        .into_any_element(),
                );
            }
        }
        if prefs.card_updated
            && let Some(m) = t.modified
        {
            chips.push(div().flex_none().text_xs().text_color(muted.opacity(0.7)).child(ago_since(Some(m))).into_any_element());
        }

        let mut marks: Vec<AnyElement> = Vec::new();
        if t.agent.is_none()
            && let Some(why) = &t.inferred
        {
            marks.push(
                div()
                    .id(("inferred", ix))
                    .flex()
                    .items_center()
                    .text_xs()
                    .text_color(muted)
                    .child(Icon::new(IconName::Sparkles).size(px(11.)))
                    .tooltip(tip(format!("{why}. The file still says {}.", status_label(&t.status))))
                    .into_any_element(),
            );
        }
        if let Some(run) = &t.agent {
            let (label, c) = match run.state {
                crate::agents::RunState::Running => ("Agent working", theme.green),
                crate::agents::RunState::AwaitingReview => ("Awaiting review", yellow),
                crate::agents::RunState::Finished => ("Agent finished", muted),
            };
            marks.push(
                div()
                    .id(("agent", ix))
                    .flex()
                    .items_center()
                    .gap_1()
                    .px_1p5()
                    .rounded(px(5.))
                    .bg(c.opacity(0.14))
                    .text_xs()
                    .text_color(c)
                    .child(Icon::new(IconName::Bot).size(px(12.)))
                    .child(label)
                    .tooltip(tip(format!(
                        "{}\nLast activity {} · the file says {}",
                        run.description,
                        crate::app::ago_label(now_unix() - run.last_activity),
                        status_label(&t.status)
                    )))
                    .into_any_element(),
            );
        }
        if prefs.card_relations {
            if frontier {
                marks.push(
                    div()
                        .id(("frontier", ix))
                        .child(Icon::new(IconName::Compass).size(px(12.)).text_color(theme.green))
                        .tooltip(tip("On the frontier: open and every blocker closed".into()))
                        .into_any_element(),
                );
            }
            if blocked {
                let open_blockers = self.idx.open_blockers.get(ix).cloned().unwrap_or_default();
                let projects = self.blocking_projects(ix);
                let names = open_blockers
                    .iter()
                    .map(|&b| format!("{} {}", self.board.tickets[b].key, self.board.tickets[b].title))
                    .chain(projects.iter().map(|&p| format!("The whole {} project", self.board.projects[p].title)))
                    .collect::<Vec<_>>()
                    .join("\n");
                let count = open_blockers.len() + projects.len();
                marks.push(
                    div()
                        .id(("blocked", ix))
                        .flex()
                        .items_center()
                        .gap_0p5()
                        .text_xs()
                        .text_color(red)
                        .child(Icon::new(IconName::Lock).size(px(12.)))
                        .child(count.to_string())
                        .tooltip(tip(format!("Blocked by\n{names}")))
                        .into_any_element(),
                );
            }
            if let Some(h) = &t.needs_human {
                marks.push(
                    div()
                        .id(("human", ix))
                        .child(Icon::new(IconName::Hand).size(px(12.)).text_color(accent))
                        .tooltip(tip(format!("Needs a human: {h}")))
                        .into_any_element(),
                );
            }
        }

        let people = self.assignees(ix).filter(|p| !p.is_empty());
        let next_skill = self.next_command(ix).map(|(s, _, _)| s);
        let path = t.path.clone();
        let menu_path = t.path.clone();
        let is_map = project.map.is_some();
        let compact = prefs.compact();
        div()
            .id(("card", ix))
            .flex()
            .flex_col()
            .gap(px(if compact { 4. } else { 6. }))
            .px(px(10.))
            .py(px(if compact { 6. } else { 9. }))
            .rounded(px(8.))
            .border_1()
            .border_color(if selected || open { accent } else { border })
            .bg(card_bg)
            .when(flash > 0., |d| d.bg(accent.opacity(0.18 * flash)))
            .when(blocked, |d| d.opacity(0.78))
            .cursor_pointer()
            .hover(|d| d.border_color(muted.opacity(0.45)))
            .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                // A card opens as its own page; space still peeks at the side.
                this.open_detail(path.clone(), true, window, cx);
                this.board_focus.focus(window, cx);
            }))
            .context_menu(move |menu: PopupMenu, window, cx| {
                let editors_menu = {
                    let p = menu_path.clone();
                    PopupMenu::build(window, cx, move |mut m, _, _| {
                        for e in crate::editors::installed() {
                            let (name, p) = (e.name.clone(), p.clone());
                            m = m.item(PopupMenuItem::new(e.name.clone()).on_click(move |_, _, cx| {
                                let (name, p) = (name.clone(), p.clone());
                                with_app(cx, |a, cx| a.open_with(name, &p, cx));
                            }));
                        }
                        m
                    })
                };
                let go = |f: fn(&mut KuzgunApp, PathBuf, &mut Window, &mut Context<KuzgunApp>)| {
                    let p = menu_path.clone();
                    move |_: &ClickEvent, window: &mut Window, cx: &mut App| {
                        let p = p.clone();
                        let view = cx.global::<KuzgunHandle>().0.clone();
                        view.update(cx, |this, cx| f(this, p, window, cx));
                    }
                };
                let menu = menu
                    .item(PopupMenuItem::new("Open").on_click(go(|a, p, w, cx| a.open_detail(p, true, w, cx))))
                    .item(PopupMenuItem::new("Peek").on_click(go(|a, p, w, cx| a.open_detail(p, false, w, cx))))
                    .separator();
                let menu = match &next_skill {
                    Some(skill) => menu
                        .item(PopupMenuItem::new(format!("Start agent (/{skill})")).on_click(go(|a, p, _, cx| a.start_agent(&p, cx))))
                        .item(
                            PopupMenuItem::new(format!("Copy /{skill} command (suggested)"))
                                .on_click(go(|a, p, _, cx| a.copy_next(&p, cx))),
                        ),
                    None => menu,
                }
                    .item(PopupMenuItem::new("Copy /implement Command").on_click(go(|a, p, _, cx| a.copy_command("implement", &p, cx))))
                    .item(PopupMenuItem::new("Copy /triage Command").on_click(go(|a, p, _, cx| a.copy_command("triage", &p, cx))));
                let menu = if is_map {
                    menu.item(PopupMenuItem::new("Copy /wayfinder Command").on_click(go(|a, p, _, cx| a.copy_command("wayfinder", &p, cx))))
                } else {
                    menu
                };
                menu.separator()
                    .item(PopupMenuItem::new("Open in Editor").on_click(go(|a, p, _, cx| a.open_in_editor(&p, cx))))
                    .item(PopupMenuItem::submenu("Open in", editors_menu))
                    .item(PopupMenuItem::new("Reveal in Finder").on_click(go(|_, p, _, cx| cx.reveal_path(&p))))
                    .separator()
                    .item(PopupMenuItem::new("Copy ID").on_click(go(|a, p, _, cx| {
                        a.selected = Some(p);
                        a.copy_current("ID", cx);
                    })))
                    .item(PopupMenuItem::new("Copy Title").on_click(go(|a, p, _, cx| {
                        a.selected = Some(p);
                        a.copy_current("title", cx);
                    })))
                    .item(PopupMenuItem::new("Copy Path").on_click(go(|a, p, _, cx| a.copy("path", p.display().to_string(), cx))))
            })
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_1p5()
                    .when(prefs.card_id, |d| {
                        d.child(
                            div()
                                .text_xs()
                                .font_family(crate::settings::mono_font())
                                .text_color(muted.opacity(0.8))
                                .child(t.key.clone()),
                        )
                    })
                    .child(div().flex_1())
                    .children(marks)
                    .children(people.map(|p| {
                        div()
                            .id(("people", ix))
                            .child(avatar_group(&p, 20., card_bg, 3, muted))
                            .tooltip(tip(format!("Assignee: {}", p.join(", "))))
                    })),
            )
            .child(
                div()
                    .flex()
                    .items_start()
                    .gap_2()
                    .child(
                        div().flex_none().pt(px(2.)).child(
                            icons::status_icon(t.category)
                                .size(px(14.))
                                .text_color(icons::status_color(t.category, light)),
                        ),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_sm()
                            .line_height(rems(1.3))
                            .text_color(fg)
                            .when(compact, |d| d.truncate())
                            .when(!compact, |d| d.line_clamp(3))
                            .child(t.title.clone()),
                    ),
            )
            .when(!chips.is_empty() && !compact, |d| {
                d.child(div().flex().flex_wrap().items_center().gap_1().children(chips))
            })
            .into_any_element()
    }
}
