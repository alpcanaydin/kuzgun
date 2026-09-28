//! Status icons in the Linear style (shape from the category, color from
//! the status) and the progress ring. SVGs render as a mask in the text
//! color, so one shape serves every theme.

use gpui_kit::component::Icon;
use gpui_kit::*;

use crate::model::Category;

const BACKLOG: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 16 16"><circle cx="8" cy="8" r="6" fill="none" stroke="black" stroke-width="1.6" stroke-dasharray="2.2 1.9"/></svg>"#;
const TODO: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 16 16"><circle cx="8" cy="8" r="6" fill="none" stroke="black" stroke-width="1.6"/></svg>"#;
const IN_PROGRESS: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 16 16"><circle cx="8" cy="8" r="6" fill="none" stroke="black" stroke-width="1.6"/><path d="M8 4 A4 4 0 0 1 8 12 Z" fill="black"/></svg>"#;
const IN_REVIEW: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 16 16"><circle cx="8" cy="8" r="6" fill="none" stroke="black" stroke-width="1.6"/><path d="M8 4 A4 4 0 1 1 4 8 L8 8 Z" fill="black"/></svg>"#;
const DONE: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 16 16"><path fill-rule="evenodd" d="M8 1 A7 7 0 1 1 7.99 1 Z M11.6 5.6 L10.5 4.5 L6.9 8.9 L5.3 7.4 L4.3 8.5 L7 11.1 Z" fill="black"/></svg>"#;
const CANCELED: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 16 16"><path fill-rule="evenodd" d="M8 1 A7 7 0 1 1 7.99 1 Z M5.4 4.3 L4.3 5.4 L6.9 8 L4.3 10.6 L5.4 11.7 L8 9.1 L10.6 11.7 L11.7 10.6 L9.1 8 L11.7 5.4 L10.6 4.3 L8 6.9 Z" fill="black"/></svg>"#;

pub const LOGO: &[u8] = include_bytes!("../assets/icon/kuzgun-256.png");

static CLAUDE_CODE: std::sync::LazyLock<std::sync::Arc<Image>> = std::sync::LazyLock::new(|| {
    std::sync::Arc::new(Image::from_bytes(ImageFormat::Svg, include_bytes!("../assets/icon/harness/claudecode-color.svg").to_vec()))
});
static CODEX: std::sync::LazyLock<std::sync::Arc<Image>> = std::sync::LazyLock::new(|| {
    std::sync::Arc::new(Image::from_bytes(ImageFormat::Svg, include_bytes!("../assets/icon/harness/codex-color.svg").to_vec()))
});

/// The logo of the agent harness that ran a session, in its own colors.
pub fn harness_logo(p: crate::transcript::Provider, size: f32) -> Img {
    let image = match p {
        crate::transcript::Provider::Claude => CLAUDE_CODE.clone(),
        crate::transcript::Provider::Codex => CODEX.clone(),
    };
    img(image).size(px(size)).flex_none()
}

pub fn status_icon(c: Category) -> Icon {
    let svg = match c {
        Category::Backlog => BACKLOG,
        Category::Todo => TODO,
        Category::InProgress => IN_PROGRESS,
        Category::InReview => IN_REVIEW,
        Category::Done => DONE,
        Category::Canceled => CANCELED,
    };
    Icon::default().data(svg.as_bytes())
}

/// Linear's default state colors; one per category, tuned per mode.
pub fn status_color(c: Category, light: bool) -> Hsla {
    let hex = match (c, light) {
        (Category::Backlog, false) => 0xBEC2C8,
        (Category::Backlog, true) => 0x8A8F98,
        (Category::Todo, false) => 0xE2E2E2,
        (Category::Todo, true) => 0x6B6F76,
        (Category::InProgress, _) => 0xF2C94C,
        (Category::InReview, _) => 0x4CB782,
        (Category::Done, false) => 0x7C83F0,
        (Category::Done, true) => 0x5E6AD2,
        (Category::Canceled, _) => 0x95A2B3,
    };
    rgb(hex).into()
}

/// A ring filled to `frac` (0–1), clockwise from the top.
pub fn ring(frac: f32) -> Icon {
    let f = frac.clamp(0., 1.);
    let inner = if f >= 0.999 {
        r#"<circle cx="8" cy="8" r="4.2" fill="black"/>"#.to_string()
    } else if f <= 0.001 {
        String::new()
    } else {
        let a = f * std::f32::consts::TAU;
        let (x, y) = (8. + 4.2 * a.sin(), 8. - 4.2 * a.cos());
        let large = if f > 0.5 { 1 } else { 0 };
        format!(r#"<path d="M8 8 L8 3.8 A4.2 4.2 0 {large} 1 {x:.2} {y:.2} Z" fill="black"/>"#)
    };
    let svg = format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 16 16"><circle cx="8" cy="8" r="6.3" fill="none" stroke="black" stroke-width="1.4"/>{inner}</svg>"#
    );
    Icon::default().data(svg.as_bytes())
}

/// A deterministic hue per text (project and label chips).
pub fn tag_color(text: &str) -> Hsla {
    let mut h: u32 = 2166136261;
    for b in text.bytes() {
        h ^= b as u32;
        h = h.wrapping_mul(16777619);
    }
    hsla((h % 360) as f32 / 360., 0.55, 0.62, 1.)
}

/// Linear's milestone mark: a diamond that fills bottom-up with `frac` (0–1).
pub fn diamond(frac: f32) -> Icon {
    let f = frac.clamp(0., 1.);
    let top = 14.5 - 13. * f;
    let svg = format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 16 16"><defs><clipPath id="d"><path d="M8 1.5 L14.5 8 L8 14.5 L1.5 8 Z"/></clipPath></defs><path d="M8 1.5 L14.5 8 L8 14.5 L1.5 8 Z" fill="none" stroke="black" stroke-width="1.4" stroke-linejoin="round"/><rect x="0" y="{top:.2}" width="16" height="16" fill="black" clip-path="url(#d)"/></svg>"#
    );
    Icon::default().data(svg.as_bytes())
}
