//! Start a Claude Code session on a ticket in the person's terminal. Kuzgun
//! writes nothing to the repo: it runs `claude "<command>"` in the repo
//! folder, through a small launcher script in the temp folder.

use std::path::{Path, PathBuf};
use std::sync::LazyLock;

#[derive(Clone, Debug, PartialEq)]
pub struct Terminal {
    pub name: String,
    pub app: PathBuf,
}

/// Supported terminals, in the order a default is picked.
const KNOWN: [&str; 6] = ["Ghostty", "iTerm", "WezTerm", "kitty", "Warp", "Terminal"];

static INSTALLED: LazyLock<Vec<Terminal>> = LazyLock::new(|| {
    let dirs = [
        PathBuf::from("/Applications"),
        PathBuf::from("/System/Applications/Utilities"),
        dirs::home_dir().map(|h| h.join("Applications")).unwrap_or_default(),
    ];
    KNOWN
        .iter()
        .filter_map(|name| {
            dirs.iter()
                .map(|d| d.join(format!("{name}.app")))
                .find(|p| p.is_dir())
                .map(|app| Terminal { name: name.to_string(), app })
        })
        .collect()
});

pub fn installed() -> &'static [Terminal] {
    &INSTALLED
}

/// The terminal from the settings, else the first one installed.
pub fn chosen() -> Option<Terminal> {
    let want = crate::settings::get().terminal_app.clone();
    installed()
        .iter()
        .find(|t| t.name == want)
        .or_else(|| installed().first())
        .cloned()
}

fn quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}

/// Opens a new terminal window in `repo` running `claude <command>`.
pub fn start_agent(repo: &Path, command: &str) -> Result<String, String> {
    let term = chosen().ok_or("No terminal app found")?;
    let script = std::env::temp_dir().join(format!("kuzgun-agent-{}.command", std::process::id() as u64 * 1000 + rand_suffix()));
    // A login shell finds `claude` on the person's PATH; the shell stays
    // open when the session ends.
    let body = format!(
        "#!/bin/zsh -l\ncd {}\nclaude {}\nexec $SHELL -l\n",
        quote(&repo.display().to_string()),
        quote(command)
    );
    std::fs::write(&script, body).map_err(|e| e.to_string())?;
    let _ = std::process::Command::new("chmod").arg("+x").arg(&script).status();
    let s = script.display().to_string();
    let result = match term.name.as_str() {
        "Ghostty" => std::process::Command::new("open")
            .arg("-na")
            .arg(&term.app)
            .args(["--args", "-e", "/bin/zsh", &s])
            .spawn(),
        "WezTerm" => std::process::Command::new(term.app.join("Contents/MacOS/wezterm"))
            .args(["start", "--", "/bin/zsh", &s])
            .spawn(),
        "kitty" => std::process::Command::new(term.app.join("Contents/MacOS/kitty")).args(["/bin/zsh", &s]).spawn(),
        // Terminal, iTerm and Warp open a `.command` file as a new session.
        _ => std::process::Command::new("open").arg("-a").arg(&term.app).arg(&script).spawn(),
    };
    result.map(|_| term.name.clone()).map_err(|e| format!("{}: {e}", term.name))
}

fn rand_suffix() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.subsec_nanos() as u64 % 1000)
        .unwrap_or(0)
}
