//! Editors installed on this Mac, for "Open in Editor". Found by app
//! bundle name in the usual Applications folders; opened with `open -a`.

use std::path::{Path, PathBuf};
use std::sync::LazyLock;

#[derive(Clone, Debug, PartialEq)]
pub struct Editor {
    /// Display name, also the saved setting.
    pub name: String,
    pub app: PathBuf,
}

/// Known markdown-capable editors, in the order the picker lists them.
const KNOWN: [&str; 22] = [
    "Zed",
    "Visual Studio Code",
    "Cursor",
    "Windsurf",
    "Sublime Text",
    "Nova",
    "BBEdit",
    "CotEditor",
    "WebStorm",
    "IntelliJ IDEA",
    "RustRover",
    "PyCharm",
    "Fleet",
    "Xcode",
    "Obsidian",
    "iA Writer",
    "Typora",
    "MarkText",
    "MacVim",
    "Emacs",
    "Neovide",
    "TextEdit",
];

static INSTALLED: LazyLock<Vec<Editor>> = LazyLock::new(scan);

fn scan() -> Vec<Editor> {
    let mut dirs: Vec<PathBuf> = vec![PathBuf::from("/Applications"), PathBuf::from("/System/Applications")];
    if let Some(home) = dirs::home_dir() {
        dirs.push(home.join("Applications"));
    }
    let apps: Vec<PathBuf> = dirs
        .iter()
        .filter_map(|d| std::fs::read_dir(d).ok())
        .flat_map(|rd| rd.flatten().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|e| e == "app"))
        .collect();
    KNOWN
        .iter()
        .filter_map(|name| {
            // "WebStorm 2026.3 EAP.app" still counts as WebStorm; the plain
            // name wins when both exist.
            let stem = |p: &Path| p.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
            let app = apps
                .iter()
                .find(|p| stem(p) == *name)
                .or_else(|| apps.iter().find(|p| stem(p).starts_with(&format!("{name} "))))?;
            Some(Editor { name: name.to_string(), app: app.clone() })
        })
        .collect()
}

pub fn installed() -> &'static [Editor] {
    &INSTALLED
}

/// Opens `file` in the editor saved in the settings (the custom command
/// first, then the picked app, else the system's default for `.md`).
pub fn open(file: &Path) -> Result<(), String> {
    let prefs = crate::settings::get();
    let cmd = prefs.editor_command.trim();
    if !cmd.is_empty() {
        let mut parts = cmd.split_whitespace();
        let bin = parts.next().unwrap_or_default().to_string();
        let args: Vec<String> = parts.map(str::to_string).collect();
        // GUI apps get a login-less PATH: look in Homebrew and /usr/local too.
        let path_env = format!("{}:/opt/homebrew/bin:/usr/local/bin", std::env::var("PATH").unwrap_or_default());
        return std::process::Command::new(&bin)
            .args(&args)
            .arg(file)
            .env("PATH", path_env)
            .spawn()
            .map(|_| ())
            .map_err(|e| format!("{bin}: {e}"));
    }
    let app = installed().iter().find(|e| e.name == prefs.editor_app).map(|e| e.app.clone());
    let mut c = std::process::Command::new("open");
    if let Some(app) = app {
        c.arg("-a").arg(app);
    }
    c.arg(file).spawn().map(|_| ()).map_err(|e| format!("open: {e}"))
}
