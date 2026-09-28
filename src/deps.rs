//! Dependency map: the open tickets as a layered graph of their blockers.
//! A ticket sits one layer right of its deepest open blocker, so the left
//! edge is what can start now and every arrow reads "must finish first".

use std::collections::HashMap;

use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::{Icon, Sizable as _};
use gpui_kit::component::theme::ActiveTheme as _;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::app::KuzgunApp;
use crate::icons;
use crate::model::{self, Mode};

const NODE_W0: f32 = 240.;
const NODE_H0: f32 = 52.;
const COL_GAP0: f32 = 90.;
const ROW_GAP0: f32 = 14.;
const PAD: f32 = 32.;
const HEAD: f32 = 36.;
const MINI_W: f32 = 220.;
const MINI_H: f32 = 140.;
const MINI_PAD: f32 = 6.;
const ZOOMS: [f32; 7] = [0.4, 0.55, 0.7, 0.85, 1., 1.2, 1.4];

impl KuzgunApp {
    pub fn render_deps(&mut self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let t = cx.theme().clone();
        let (muted, fg, border) = (t.muted_foreground, t.foreground, t.border);
        let light = crate::settings::is_light(cx);
        let z = self.deps_zoom;
        let (node_w, node_h, col_gap, row_gap) = (NODE_W0 * z, NODE_H0 * z, COL_GAP0 * z, ROW_GAP0 * z);
        let scope = self.project_ix();
        let b = &self.board;

        // Open tickets in scope (closed ones block nothing).
        let nodes: Vec<usize> = (0..b.tickets.len())
            .filter(|&i| scope.is_none_or(|p| b.tickets[i].project == p) && !b.tickets[i].category.is_closed())
            .collect();
        let in_set: HashMap<usize, ()> = nodes.iter().map(|&i| (i, ())).collect();
        let open_blockers = |i: usize| -> Vec<usize> {
            b.tickets[i].blocked_by.iter().copied().filter(|j| in_set.contains_key(j)).collect()
        };
        // Layer = 1 + deepest blocker's layer; a guard stops cycles.
        let mut layer: HashMap<usize, usize> = HashMap::new();
        fn depth(
            i: usize,
            layer: &mut HashMap<usize, usize>,
            blockers: &dyn Fn(usize) -> Vec<usize>,
            stack: &mut Vec<usize>,
        ) -> usize {
            if let Some(&l) = layer.get(&i) {
                return l;
            }
            if stack.contains(&i) {
                return 0;
            }
            stack.push(i);
            let l = blockers(i).into_iter().map(|j| depth(j, layer, blockers, stack) + 1).max().unwrap_or(0);
            stack.pop();
            layer.insert(i, l);
            l
        }
        for &i in &nodes {
            let mut stack = Vec::new();
            depth(i, &mut layer, &open_blockers, &mut stack);
        }
        let layers = layer.values().copied().max().map(|m| m + 1).unwrap_or(0);
        let mut columns: Vec<Vec<usize>> = vec![Vec::new(); layers];
        for &i in &nodes {
            columns[layer[&i]].push(i);
        }
        // Inside a layer: by milestone, then number, so related work sits together.
        let milestone_key = b.facets.iter().find(|f| model::is_milestone(&f.key)).map(|f| f.key.clone());
        for col in &mut columns {
            col.sort_by_key(|&i| {
                let m = milestone_key
                    .as_ref()
                    .and_then(|k| b.tickets[i].field_values(k).into_iter().next())
                    .map(|v| model::natural_key(&v))
                    .unwrap_or_default();
                (m, b.tickets[i].project, b.tickets[i].num.unwrap_or(u32::MAX))
            });
        }
        let mut pos: HashMap<usize, (f32, f32)> = HashMap::new();
        for (c, col) in columns.iter().enumerate() {
            for (r, &i) in col.iter().enumerate() {
                let x = PAD + c as f32 * (node_w + col_gap);
                let y = PAD + HEAD + r as f32 * (node_h + row_gap);
                pos.insert(i, (x, y));
            }
        }
        let width = PAD * 2. + layers as f32 * (node_w + col_gap);
        let height = PAD * 2. + HEAD + columns.iter().map(Vec::len).max().unwrap_or(0) as f32 * (node_h + row_gap);

        // Focus: the chain of the picked ticket (what it waits on, what
        // waits on it); the rest fades.
        // The ticket open in the side panel is the focus.
        let selected = self
            .detail
            .as_ref()
            .filter(|d| !d.full && !d.is_doc)
            .and_then(|d| b.find_path(&d.path));
        let chain: Option<std::collections::HashSet<usize>> = selected.map(|s| {
            let mut set = std::collections::HashSet::from([s]);
            let mut up = vec![s];
            while let Some(i) = up.pop() {
                for j in open_blockers(i) {
                    if set.insert(j) {
                        up.push(j);
                    }
                }
            }
            let mut down = vec![s];
            while let Some(i) = down.pop() {
                for &j in &b.tickets[i].blocks {
                    if in_set.contains_key(&j) && set.insert(j) {
                        down.push(j);
                    }
                }
            }
            set
        });
        let lit = |i: usize| chain.as_ref().is_none_or(|c| c.contains(&i));
        // Edges: blocker's right edge → blocked's left edge.
        let mut edges: Vec<(Point<Pixels>, Point<Pixels>, bool)> = Vec::new();
        for &i in &nodes {
            for j in open_blockers(i) {
                let (Some(&(x1, y1)), Some(&(x2, y2))) = (pos.get(&j), pos.get(&i)) else {
                    continue;
                };
                let hot = chain.as_ref().is_some_and(|c| c.contains(&i) && c.contains(&j));
                edges.push((point(px(x1 + node_w), px(y1 + node_h / 2.)), point(px(x2), px(y2 + node_h / 2.)), hot));
            }
        }
        let edge_color = if chain.is_some() { muted.opacity(0.12) } else { muted.opacity(0.35) };
        let hot_color = t.accent;
        let edges_layer = canvas(
            move |_, _, _| edges,
            move |bounds, edges, window, _| {
                for (from, to, hot) in edges {
                    let o = bounds.origin;
                    let (a, z) = (o + from, o + to);
                    let dx = (z.x - a.x).abs() / 2.;
                    let mut path = PathBuilder::stroke(px(if hot { 2. } else { 1.2 }));
                    path.move_to(a);
                    path.cubic_bezier_to(z, point(a.x + dx, a.y), point(z.x - dx, z.y));
                    if let Ok(p) = path.build() {
                        window.paint_path(p, if hot { hot_color } else { edge_color });
                    }
                }
            },
        )
        .absolute()
        .top_0()
        .left_0()
        .w(px(width))
        .h(px(height));

        let layer_heads = (0..layers).map(|c| {
            let label = if c == 0 { "No open blockers".to_string() } else { format!("After {c} step{}", if c == 1 { "" } else { "s" }) };
            div()
                .absolute()
                .left(px(PAD + c as f32 * (node_w + col_gap)))
                .top(px(PAD))
                .w(px(node_w))
                .truncate()
                .text_size(px((12. * z).max(9.)))
                .font_weight(FontWeight::MEDIUM)
                .text_color(if c == 0 { t.green } else { muted })
                .child(format!("{label} · {}", columns[c].len()))
        });

        let node_els: Vec<AnyElement> = nodes
            .iter()
            .filter_map(|&i| {
                let (x, y) = *pos.get(&i)?;
                let tk = &b.tickets[i];
                let frontier = self.idx.frontier.get(i).copied().unwrap_or(false);
                let running = tk.agent.as_ref().is_some_and(|a| a.state == crate::agents::RunState::Running);
                let hitl = self.idx.modes.get(i).copied().flatten() == Some(Mode::Hitl);
                let path = tk.path.clone();
                let sel = selected == Some(i);
                let fade = !lit(i);
                Some(
                    div()
                        .id(("dep", i))
                        .absolute()
                        .left(px(x))
                        .top(px(y))
                        .w(px(node_w))
                        .h(px(node_h))
                        .px_2p5()
                        .flex()
                        .flex_col()
                        .justify_center()
                        .gap_0p5()
                        .rounded(px(8.))
                        .border_1()
                        .border_color(if sel {
                            t.accent
                        } else if frontier {
                            t.green.opacity(0.7)
                        } else {
                            border
                        })
                        .bg(t.background)
                        .when(fade, |d| d.opacity(0.3))
                        .cursor_pointer()
                        .hover(|d| d.border_color(muted))
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap_1p5()
                                .child(icons::status_icon(tk.category).size(px(12. * z)).text_color(icons::status_color(tk.category, light)))
                                .child(div().text_size(px(11. * z)).font_family(crate::settings::mono_font()).text_color(muted).child(tk.key.clone()))
                                .child(div().flex_1())
                                .when(running, |d| d.child(Icon::new(IconName::Bot).size(px(12. * z)).text_color(t.green)))
                                .when(hitl, |d| d.child(Icon::new(IconName::Hand).size(px(12. * z)).text_color(t.accent))),
                        )
                        .when(z >= 0.55, |d| d.child(div().truncate().text_size(px(13. * z)).text_color(fg).child(tk.title.clone())))
                        .on_click(cx.listener(move |this, e: &ClickEvent, w, cx| {
                            cx.stop_propagation();
                            if e.click_count() >= 2 {
                                this.open_detail(path.clone(), true, w, cx);
                            } else {
                                if this.detail.as_ref().is_some_and(|d| !d.full && d.path == path) {
                                    this.detail = None;
                                    cx.notify();
                                } else {
                                    this.open_detail(path.clone(), false, w, cx);
                                }
                            }
                        }))
                        .into_any_element(),
                )
            })
            .collect();

        let empty = nodes.is_empty();
        let mini = (!empty).then(|| {
            let scale = (MINI_W / width).min(MINI_H / height);
            let (mw, mh) = (width * scale, height * scale);
            let rects: Vec<(f32, f32, Hsla)> = nodes
                .iter()
                .filter_map(|&i| {
                    let &(x, y) = pos.get(&i)?;
                    let c = if selected == Some(i) {
                        t.accent
                    } else if !lit(i) {
                        muted.opacity(0.15)
                    } else if self.idx.frontier.get(i).copied().unwrap_or(false) {
                        t.green.opacity(0.8)
                    } else {
                        muted.opacity(0.55)
                    };
                    Some((x * scale, y * scale, c))
                })
                .collect();
            let (nw, nh) = ((node_w * scale).max(2.), (node_h * scale).max(1.5));
            let handle = self.deps_scroll.clone();
            let accent = t.accent;
            let jump = move |this: &mut KuzgunApp, p: Point<Pixels>| {
                let b = this.deps_scroll.bounds();
                let origin = point(
                    b.origin.x + b.size.width - px(mw + MINI_PAD * 2. + 12.) + px(MINI_PAD),
                    b.origin.y + b.size.height - px(mh + MINI_PAD * 2. + 12.) + px(MINI_PAD),
                );
                let c = (p - origin) * (1. / scale);
                this.deps_scroll.set_offset(point(b.size.width / 2. - c.x, b.size.height / 2. - c.y));
            };
            div()
                .id("deps-mini")
                .absolute()
                .right(px(12.))
                .bottom(px(12.))
                .p(px(MINI_PAD))
                .rounded(px(8.))
                .border_1()
                .border_color(border)
                .bg(t.background.opacity(0.92))
                .shadow_md()
                .cursor(CursorStyle::Crosshair)
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, e: &MouseDownEvent, _, cx| {
                        cx.stop_propagation();
                        this.deps_mini_drag = true;
                        jump(this, e.position);
                        cx.notify();
                    }),
                )
                .on_mouse_move(cx.listener(move |this, e: &MouseMoveEvent, _, cx| {
                    if this.deps_mini_drag {
                        if e.pressed_button != Some(MouseButton::Left) {
                            this.deps_mini_drag = false;
                        } else {
                            jump(this, e.position);
                        }
                        cx.notify();
                    }
                }))
                .on_mouse_up(
                    MouseButton::Left,
                    cx.listener(|this, _: &MouseUpEvent, _, cx| {
                        this.deps_mini_drag = false;
                        cx.notify();
                    }),
                )
                .child(
                    canvas(
                        move |_, _, _| {
                            // Read the scroll at paint time: it moves without a re-render.
                            let (view, off) = (handle.bounds(), handle.offset());
                            let frame = (
                                f32::from(-off.x) * scale,
                                f32::from(-off.y) * scale,
                                f32::from(view.size.width) * scale,
                                f32::from(view.size.height) * scale,
                            );
                            (rects, frame)
                        },
                        move |bounds, (rects, (fx, fy, fw, fh)), window, _| {
                            let o = bounds.origin;
                            for (x, y, c) in rects {
                                window.paint_quad(fill(Bounds::new(o + point(px(x), px(y)), size(px(nw), px(nh))), c));
                            }
                            let r = Bounds::new(o + point(px(fx), px(fy)), size(px(fw), px(fh)))
                                .intersect(&bounds);
                            window.paint_quad(
                                quad(r, px(2.), accent.opacity(0.08), px(1.), accent, BorderStyle::default()),
                            );
                        },
                    )
                    .w(px(mw))
                    .h(px(mh)),
                )
        });
        let step = |dir: i32| {
            move |this: &mut KuzgunApp, _: &ClickEvent, _: &mut Window, cx: &mut Context<KuzgunApp>| {
                let z = this.deps_zoom;
                this.deps_zoom = if dir > 0 {
                    ZOOMS.iter().copied().find(|&v| v > z + 0.01).unwrap_or(ZOOMS[ZOOMS.len() - 1])
                } else {
                    ZOOMS.iter().rev().copied().find(|&v| v < z - 0.01).unwrap_or(ZOOMS[0])
                };
                cx.notify();
            }
        };
        let fit_w = width / z;
        let zoom_bar = div()
            .flex()
            .flex_none()
            .items_center()
            .gap_1()
            .child(Button::new("zoom-out").ghost().xsmall().icon(Icon::new(IconName::Minus).text_color(muted)).tooltip("Zoom out").on_click(cx.listener(step(-1))))
            .child(
                Button::new("zoom-reset")
                    .ghost()
                    .xsmall()
                    .label(format!("{:.0}%", z * 100.))
                    .tooltip("Reset to 100%")
                    .on_click(cx.listener(|this, _, _, cx| {
                        let z0 = this.deps_zoom;
                        // Keep the centre of the view in place.
                        let b = this.deps_scroll.bounds();
                        let mid = point(b.size.width / 2., b.size.height / 2.);
                        let content = mid - this.deps_scroll.offset();
                        this.deps_zoom = 1.;
                        this.deps_scroll.set_offset(mid - content * (1. / z0));
                        cx.notify();
                    })),
            )
            .child(Button::new("zoom-in").ghost().xsmall().icon(Icon::new(IconName::Plus).text_color(muted)).tooltip("Zoom in").on_click(cx.listener(step(1))))
            .child(
                Button::new("zoom-fit")
                    .ghost()
                    .xsmall()
                    .label("Fit")
                    .tooltip("Fit the whole map in the window")
                    .on_click(cx.listener(move |this, _, window, cx| {
                        let avail = f32::from(window.viewport_size().width) - 300.;
                        let want = (avail / fit_w).clamp(ZOOMS[0], 1.);
                        this.deps_zoom = *ZOOMS.iter().rev().find(|&&v| v <= want).unwrap_or(&ZOOMS[0]);
                        this.deps_scroll.set_offset(point(px(0.), px(0.)));
                        cx.notify();
                    })),
            );
        div()
            .id("deps")
            .flex_1()
            .min_w_0()
            .min_h_0()
            .flex()
            .flex_col()
            .child(
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .gap_3()
                    .h(px(46.))
                    .px_4()
                    .border_b_1()
                    .border_color(border)
                    .child(Icon::new(IconName::Network).size(px(16.)).text_color(muted))
                    .child(div().text_sm().font_weight(FontWeight::SEMIBOLD).text_color(fg).child("Dependencies"))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_xs()
                            .text_color(muted)
                            .child(format!("{} open tickets", nodes.len())),
                    )
                    .child(zoom_bar),
            )
            .child(
                div().flex_1().min_h_0().relative().child(
                div()
                    .id("deps-scroll")
                    .size_full()
                    .overflow_scroll()
                    .track_scroll(&self.deps_scroll)
                    .on_pinch(cx.listener(|this, e: &PinchEvent, _, cx| {
                        let z0 = this.deps_zoom;
                        let z1 = (z0 * (1. + e.delta)).clamp(ZOOMS[0], ZOOMS[ZOOMS.len() - 1]);
                        if (z1 - z0).abs() < f32::EPSILON {
                            return;
                        }
                        // Keep the point under the fingers in place.
                        let local = e.position - this.deps_scroll.bounds().origin;
                        let content = local - this.deps_scroll.offset();
                        this.deps_zoom = z1;
                        this.deps_scroll.set_offset(local - content * (z1 / z0));
                        cx.notify();
                    }))
                    .cursor(if self.deps_drag.is_some() { CursorStyle::ClosedHand } else { CursorStyle::OpenHand })
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, e: &MouseDownEvent, _, cx| {
                            this.deps_drag = Some((e.position, this.deps_scroll.offset()));
                            cx.notify();
                        }),
                    )
                    .on_mouse_move(cx.listener(|this, e: &MouseMoveEvent, _, cx| {
                        if let Some((start, offset)) = this.deps_drag {
                            if e.pressed_button != Some(MouseButton::Left) {
                                this.deps_drag = None;
                                cx.notify();
                                return;
                            }
                            this.deps_scroll.set_offset(offset + (e.position - start));
                            cx.notify();
                        }
                    }))
                    .on_mouse_up(
                        MouseButton::Left,
                        cx.listener(|this, e: &MouseUpEvent, _, cx| {
                            // A click on the empty map (no drag) clears the focus.
                            if let Some((start, _)) = this.deps_drag.take() {
                                let d = e.position - start;
                                if f32::from(d.x).abs() < 3. && f32::from(d.y).abs() < 3. {
                                    this.detail = None;
                                }
                            }
                            cx.notify();
                        }),
                    )
                    .when(empty, |d| {
                        d.child(div().p_8().text_sm().text_color(muted).child("No open tickets here."))
                    })
                    .when(!empty, |d| {
                        d.child(
                            div()
                                .relative()
                                .w(px(width))
                                .h(px(height))
                                .child(edges_layer)
                                .children(layer_heads)
                                .children(node_els),
                        )
                    }),
                )
                .children(mini),
            )
    }
}
