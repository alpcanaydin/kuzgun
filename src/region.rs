//! Regions of the board window as cached views. GPUI redraws the view
//! that scrolls or animates and every view above it. With one view for
//! the whole window, a scroll or a spinner redrew everything. Each region
//! here is its own view that draws a part of `KuzgunApp`, so a scroll in
//! the main area or the live pill leaves the other regions cached. A region
//! still redraws whenever the app changes.

use gpui_kit::*;

use crate::app::KuzgunApp;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RegionKind {
    Sidebar,
    Main,
    Detail,
    /// The floating live status of the session page.
    Pill,
}

pub struct Region {
    app: WeakEntity<KuzgunApp>,
    kind: RegionKind,
    _observe: Subscription,
}

impl Region {
    pub fn new(app: &Entity<KuzgunApp>, kind: RegionKind, cx: &mut Context<Self>) -> Self {
        let observe = cx.observe(app, |_, _, cx| cx.notify());
        Self {
            app: app.downgrade(),
            kind,
            _observe: observe,
        }
    }
}

impl Render for Region {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let kind = self.kind;
        match self.app.upgrade() {
            Some(app) => app.update(cx, |app, cx| app.render_region(kind, window, cx)),
            None => div().into_any_element(),
        }
    }
}

/// The region views of one window, made on the first board render.
#[derive(Clone)]
pub struct Regions {
    pub sidebar: Entity<Region>,
    pub main: Entity<Region>,
    pub detail: Entity<Region>,
    pub pill: Entity<Region>,
}

impl Regions {
    pub fn new(app: &Entity<KuzgunApp>, cx: &mut App) -> Self {
        let mut make = |kind| cx.new(|cx| Region::new(app, kind, cx));
        Self {
            sidebar: make(RegionKind::Sidebar),
            main: make(RegionKind::Main),
            detail: make(RegionKind::Detail),
            pill: make(RegionKind::Pill),
        }
    }
}
