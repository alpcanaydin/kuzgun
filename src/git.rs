//! A ticket's history from git: every commit that touched the file, with
//! the status moves and checklist ticks each one made. Read off the UI
//! thread; a folder outside git has no history.

use std::path::Path;
use std::process::Command;

use regex::Regex;
use std::sync::LazyLock;

#[derive(Clone, Debug, Default)]
pub struct Commit {
    pub hash: String,
    pub author: String,
    /// Unix seconds.
    pub time: i64,
    pub subject: String,
    /// `(from, to)` when the commit changed the status line.
    pub status_change: Option<(String, String)>,
    pub checked: usize,
    pub unchecked: usize,
    pub added: usize,
    pub removed: usize,
    pub created: bool,
}

#[derive(Clone, Debug, Default)]
pub struct History {
    pub in_repo: bool,
    pub commits: Vec<Commit>,
    /// `git status --porcelain` code for the file (`??`, ` M`, …).
    pub worktree: Option<String>,
    pub branch: Option<String>,
    /// Unix seconds the ticket entered its current status (last status move,
    /// else creation).
    pub status_since: Option<i64>,
}

static STATUS_LINE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^(?:\*\*Status:?\*\*:?|Status:|status:)\s*(.+?)\s*$").unwrap());
static CHECK_LINE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^\s*[-*+] \[( |x|X)\] (.*)$").unwrap());

fn git(dir: &Path, args: &[&str]) -> Option<String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).to_string())
}

pub fn history(path: &Path) -> History {
    let Some(dir) = path.parent() else {
        return History::default();
    };
    if git(dir, &["rev-parse", "--is-inside-work-tree"]).is_none() {
        return History::default();
    }
    let file = path.to_string_lossy().to_string();
    let branch = git(dir, &["rev-parse", "--abbrev-ref", "HEAD"]).map(|s| s.trim().to_string());
    let worktree = git(dir, &["status", "--porcelain", "--", &file]).and_then(|s| {
        s.lines()
            .next()
            .map(|l| l.chars().take(2).collect::<String>())
    });
    let log = git(
        dir,
        &[
            "log",
            "--follow",
            "-p",
            "-U0",
            "--no-color",
            "--format=\u{1e}%H\u{1f}%an\u{1f}%at\u{1f}%s",
            "--",
            &file,
        ],
    )
    .unwrap_or_default();
    let mut commits = Vec::new();
    for chunk in log.split('\u{1e}').filter(|c| !c.trim().is_empty()) {
        let mut lines = chunk.lines();
        let head: Vec<&str> = lines.next().unwrap_or_default().split('\u{1f}').collect();
        if head.len() < 4 {
            continue;
        }
        let mut c = Commit {
            hash: head[0].to_string(),
            author: head[1].to_string(),
            time: head[2].parse().unwrap_or(0),
            subject: head[3].to_string(),
            ..Default::default()
        };
        let mut old_status = None;
        let mut new_status = None;
        let mut removed_checks: Vec<(bool, String)> = Vec::new();
        let mut added_checks: Vec<(bool, String)> = Vec::new();
        for l in lines {
            if l.starts_with("new file mode") {
                c.created = true;
            }
            if l.starts_with("+++") || l.starts_with("---") {
                continue;
            }
            if let Some(rest) = l.strip_prefix('-') {
                c.removed += 1;
                if let Some(m) = STATUS_LINE.captures(rest) {
                    old_status = Some(m[1].to_string());
                }
                if let Some(m) = CHECK_LINE.captures(rest) {
                    removed_checks.push((&m[1] != " ", m[2].to_string()));
                }
            } else if let Some(rest) = l.strip_prefix('+') {
                c.added += 1;
                if let Some(m) = STATUS_LINE.captures(rest) {
                    new_status = Some(m[1].to_string());
                }
                if let Some(m) = CHECK_LINE.captures(rest) {
                    added_checks.push((&m[1] != " ", m[2].to_string()));
                }
            }
        }
        if let Some(to) = new_status {
            let from = old_status.unwrap_or_default();
            if from != to {
                c.status_change = Some((from, to));
            }
        }
        for (done, text) in &added_checks {
            if let Some((was, _)) = removed_checks.iter().find(|(_, t)| t == text) {
                if *done && !*was {
                    c.checked += 1;
                } else if !*done && *was {
                    c.unchecked += 1;
                }
            }
        }
        commits.push(c);
    }
    let status_since = commits
        .iter()
        .find(|c| c.status_change.is_some())
        .or_else(|| commits.last())
        .map(|c| c.time);
    History {
        in_repo: true,
        commits,
        worktree,
        branch,
        status_since,
    }
}

/// What git knows about one ticket file.
#[derive(Clone, Debug, Default)]
pub struct FileGit {
    /// Unix seconds of the last status change, else of the first commit.
    pub since: i64,
    /// Unix seconds of the first commit.
    pub created: i64,
    /// Unix seconds of the last commit.
    pub updated: i64,
    /// Who committed the file first (the reporter).
    pub reporter: String,
    /// Who moved its status on or checked its sub-tasks, most active first.
    pub doers: Vec<String>,
    /// Everyone who committed to it, with their commit count, most first.
    pub contributors: Vec<(String, usize)>,
}

/// Per ticket file under `root`, from one `git log` for the whole board,
/// run in the background.
pub fn board_ages(root: &Path) -> std::collections::HashMap<std::path::PathBuf, FileGit> {
    use std::collections::HashMap;
    let mut out: HashMap<std::path::PathBuf, FileGit> = HashMap::new();
    let Some(top) = git(root, &["rev-parse", "--show-toplevel"]) else {
        return out;
    };
    let top = std::path::PathBuf::from(top.trim());
    let Some(log) = git(
        root,
        &[
            "log",
            "-p",
            "-U0",
            "--no-color",
            "--no-renames",
            "--format=\u{1e}%at\u{1f}%an",
            "--",
            ".",
        ],
    ) else {
        return out;
    };
    // Newest first: the first status line added per file is the latest
    // move, and the last commit seen per file is its creation.
    let mut status_seen: HashMap<std::path::PathBuf, i64> = HashMap::new();
    let mut first: HashMap<std::path::PathBuf, (i64, String)> = HashMap::new();
    let mut last: HashMap<std::path::PathBuf, i64> = HashMap::new();
    let mut doers: HashMap<std::path::PathBuf, Vec<(String, usize)>> = HashMap::new();
    let mut all: HashMap<std::path::PathBuf, Vec<(String, usize)>> = HashMap::new();
    let bump = |v: &mut Vec<(String, usize)>, who: &str| match v.iter_mut().find(|(n, _)| n == who)
    {
        Some(e) => e.1 += 1,
        None => v.push((who.to_string(), 1)),
    };
    for chunk in log.split('\u{1e}').filter(|c| !c.trim().is_empty()) {
        let mut lines = chunk.lines();
        let head = lines.next().unwrap_or_default();
        let (time, author) = head.split_once('\u{1f}').unwrap_or((head, ""));
        let time: i64 = time.trim().parse().unwrap_or(0);
        let author = author.trim().to_string();
        let mut file: Option<std::path::PathBuf> = None;
        let mut created = false;
        let mut did: Vec<std::path::PathBuf> = Vec::new();
        for l in lines {
            if l.starts_with("--- /dev/null") {
                created = true;
                continue;
            }
            if let Some(p) = l.strip_prefix("+++ b/") {
                let f = top.join(p);
                first.insert(f.clone(), (time, author.clone()));
                last.entry(f.clone()).or_insert(time);
                bump(all.entry(f.clone()).or_default(), &author);
                file = Some(f);
                continue;
            }
            if l.starts_with("+++ ") || l.starts_with("diff --git") {
                file = None;
                created = false;
                continue;
            }
            let Some(f) = &file else {
                continue;
            };
            if let Some(rest) = l.strip_prefix('+') {
                if STATUS_LINE.is_match(rest) {
                    status_seen.entry(f.clone()).or_insert(time);
                }
                let progressed = STATUS_LINE.is_match(rest)
                    || CHECK_LINE.captures(rest).is_some_and(|c| &c[1] != " ");
                if progressed && !created && !did.contains(f) {
                    did.push(f.clone());
                }
            }
        }
        for f in did {
            bump(doers.entry(f).or_default(), &author);
        }
    }
    for (f, (created, reporter)) in first {
        let updated = last.get(&f).copied().unwrap_or(created);
        let since = status_seen.get(&f).copied().unwrap_or(created);
        let mut d = doers.remove(&f).unwrap_or_default();
        d.sort_by_key(|(_, n)| std::cmp::Reverse(*n));
        let mut c = all.remove(&f).unwrap_or_default();
        c.sort_by_key(|(_, n)| std::cmp::Reverse(*n));
        out.insert(
            f,
            FileGit {
                since,
                created,
                updated,
                reporter,
                doers: d.into_iter().map(|(n, _)| n).collect(),
                contributors: c,
            },
        );
    }
    out
}
