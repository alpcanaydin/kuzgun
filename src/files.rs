//! What an agent run changed, read-only: the file tree at that moment,
//! the changed files with their line counts, each file's diff and its full
//! text. Git answers when it can; the transcript's own edits answer when
//! it cannot.
//!
//! - A worktree that still exists: its working tree against the commit it
//!   branched from. It is live while the agent works.
//! - Commits the run made (read from `git commit` output in the
//!   transcript): the tree at its last commit against the parent of its
//!   first.
//! - Neither: only the edits in the transcript.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::LazyLock;

use regex::Regex;

use crate::transcript::{Edit, Entry, Kind};

#[derive(Clone, Debug, PartialEq)]
pub enum Source {
    /// A working folder against a base commit.
    Tree { dir: PathBuf, base: String },
    /// The run's commits: `base..head`.
    Commits {
        repo: PathBuf,
        base: String,
        head: String,
    },
    /// The commits the work landed in on the main line, found by time and
    /// by the files the run edited: `base..head`, only those files.
    Landed {
        repo: PathBuf,
        base: String,
        head: String,
        paths: Vec<String>,
    },
    /// Only the transcript's edits.
    Edits { repo: PathBuf },
}

impl Source {
    /// Where the files come from, for the header.
    pub fn label(&self) -> String {
        match self {
            Source::Tree { dir, .. } => format!(
                "Working tree · {}",
                crate::app::repo_relative(dir).trim_end_matches('/')
            ),
            Source::Commits { base, head, .. } => {
                format!("Commits {}..{}", short(base), short(head))
            }
            Source::Landed { base, head, .. } => {
                format!("Landed in {}..{}", short(base), short(head))
            }
            Source::Edits { .. } => "Edits in the conversation".into(),
        }
    }
}

fn short(sha: &str) -> &str {
    &sha[..sha.len().min(7)]
}

#[derive(Clone, Debug, PartialEq)]
pub struct Change {
    pub path: String,
    /// `A`dded, `M`odified or `D`eleted.
    pub status: char,
    pub added: usize,
    pub removed: usize,
}

#[derive(Clone, Debug)]
pub struct Snapshot {
    pub source: Source,
    /// Every file of the tree, repo-relative, sorted.
    pub files: Vec<String>,
    pub changes: Vec<Change>,
    /// Transcript edits by repo-relative path.
    pub edits: BTreeMap<String, Vec<Edit>>,
}

impl Snapshot {
    pub fn change(&self, path: &str) -> Option<&Change> {
        self.changes.iter().find(|c| c.path == path)
    }
}

/// What the snapshot is read from, gathered on the main thread.
#[derive(Clone)]
pub struct Inputs {
    pub repo: PathBuf,
    pub worktree: Option<PathBuf>,
    pub running: bool,
    /// Unix seconds the run started and last acted.
    pub window: (i64, i64),
    pub commits: Vec<String>,
    pub edits: Vec<Edit>,
}

/// `[branch 6531e90] message`, as `git commit` prints it.
static COMMIT: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?m)^\[[\w./+-]+(?: \([\w -]+\))? ([0-9a-f]{7,40})\]").unwrap());

pub fn inputs(
    repo: &Path,
    worktree: Option<PathBuf>,
    running: bool,
    window: (i64, i64),
    entries: &[Entry],
) -> Inputs {
    let mut commits = Vec::new();
    for e in entries.iter().filter(|e| e.kind == Kind::Tool) {
        if let Some(out) = &e.output {
            for c in COMMIT.captures_iter(out) {
                if !commits.contains(&c[1].to_string()) {
                    commits.push(c[1].to_string());
                }
            }
        }
    }
    Inputs {
        repo: repo.to_path_buf(),
        worktree,
        running,
        window,
        commits,
        edits: entries
            .iter()
            .flat_map(|e| e.edits.iter().cloned())
            .collect(),
    }
}

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

/// Git output whatever the exit code (`git diff --no-index` exits 1).
fn git_any(dir: &Path, args: &[&str]) -> String {
    Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
        .unwrap_or_default()
}

/// The commit a worktree branched from: its merge base with the main line.
fn fork_point(dir: &Path) -> String {
    let default = git(
        dir,
        &["symbolic-ref", "--short", "refs/remotes/origin/HEAD"],
    )
    .map(|s| s.trim().to_string());
    for r in [
        Some("main".to_string()),
        Some("master".to_string()),
        default,
    ]
    .into_iter()
    .flatten()
    {
        if let Some(b) = git(dir, &["merge-base", "HEAD", &r]) {
            return b.trim().to_string();
        }
    }
    "HEAD".into()
}

/// The parent of a commit, or git's empty tree for a root commit.
fn parent(repo: &Path, sha: &str) -> String {
    git(repo, &["rev-parse", &format!("{sha}^")])
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|| "4b825dc642cb6eb9a060e54bf8d69288fbee4904".into())
}

/// A path the agent wrote, relative to the folder the tree is read from.
pub fn relative(path: &str, roots: &[&Path]) -> String {
    // A worktree, even one that is gone: `…/.claude/worktrees/<id>/<rel>`.
    if let Some(i) = path.find("/.claude/worktrees/") {
        let rest = &path[i + "/.claude/worktrees/".len()..];
        if let Some((_, rel)) = rest.split_once('/') {
            return rel.to_string();
        }
    }
    let p = Path::new(path);
    for r in roots {
        if let Ok(rel) = p.strip_prefix(r) {
            return rel.display().to_string();
        }
    }
    path.trim_start_matches("./").to_string()
}

/// Main-line commits from the run's start to a day after its end that
/// touch a file it edited, oldest first.
fn landed(repo: &Path, window: (i64, i64), paths: &[String]) -> Vec<String> {
    if paths.is_empty() || window.0 <= 0 {
        return Vec::new();
    }
    let since = format!("--since=@{}", window.0 - 60);
    let until = format!("--until=@{}", window.1 + 86_400);
    let mut args = vec![
        "log",
        "--reverse",
        "--format=%H",
        since.as_str(),
        until.as_str(),
        "--",
    ];
    args.extend(paths.iter().map(String::as_str));
    git(repo, &args)
        .unwrap_or_default()
        .lines()
        .map(str::to_string)
        .collect()
}

fn changes_from_git(dir: &Path, range: &[&str]) -> Vec<Change> {
    let mut args = vec!["diff", "--no-renames", "--name-status"];
    args.extend_from_slice(range);
    let status: BTreeMap<String, char> = git(dir, &args)
        .unwrap_or_default()
        .lines()
        .filter_map(|l| {
            let (s, p) = l.split_once('\t')?;
            Some((p.to_string(), s.chars().next().unwrap_or('M')))
        })
        .collect();
    let mut args = vec!["diff", "--no-renames", "--numstat"];
    args.extend_from_slice(range);
    let counts: BTreeMap<String, (usize, usize)> = git(dir, &args)
        .unwrap_or_default()
        .lines()
        .filter_map(|l| {
            let mut it = l.splitn(3, '\t');
            let (a, r, p) = (it.next()?, it.next()?, it.next()?);
            Some((
                p.to_string(),
                (a.parse().unwrap_or(0), r.parse().unwrap_or(0)),
            ))
        })
        .collect();
    status
        .into_iter()
        .map(|(path, s)| {
            let (added, removed) = counts.get(&path).copied().unwrap_or((0, 0));
            Change {
                path,
                status: if s == 'A' || s == 'D' { s } else { 'M' },
                added,
                removed,
            }
        })
        .collect()
}

/// Reads the snapshot. Runs git; call it off the main thread.
pub fn snapshot(i: &Inputs) -> Snapshot {
    let wt = i.worktree.as_ref().filter(|w| w.is_dir()).cloned();
    let valid: Vec<&String> = i
        .commits
        .iter()
        .filter(|c| git(&i.repo, &["cat-file", "-e", &format!("{c}^{{commit}}")]).is_some())
        .collect();
    let roots: Vec<&Path> = [i.worktree.as_deref(), Some(i.repo.as_path())]
        .into_iter()
        .flatten()
        .collect();
    let mut edits: BTreeMap<String, Vec<Edit>> = BTreeMap::new();
    for e in &i.edits {
        let rel = relative(&e.path, &roots);
        let mut e = e.clone();
        e.diff = e.diff.replace(&e.path, &rel);
        edits.entry(rel).or_default().push(e);
    }

    let edited: Vec<String> = edits.keys().cloned().collect();
    let landed_in = if valid.is_empty() && !i.running && wt.is_none() {
        landed(&i.repo, i.window, &edited)
    } else {
        Vec::new()
    };
    let source = if let Some(dir) = wt {
        let base = fork_point(&dir);
        Source::Tree { dir, base }
    } else if let (Some(first), Some(last)) = (valid.first(), valid.last()) {
        let base = parent(&i.repo, first);
        if i.running {
            Source::Tree {
                dir: i.repo.clone(),
                base,
            }
        } else {
            Source::Commits {
                repo: i.repo.clone(),
                base,
                head: last.to_string(),
            }
        }
    } else if let (Some(first), Some(last)) = (landed_in.first(), landed_in.last()) {
        Source::Landed {
            repo: i.repo.clone(),
            base: parent(&i.repo, first),
            head: last.clone(),
            paths: edited.clone(),
        }
    } else if i.running {
        Source::Tree {
            dir: i.repo.clone(),
            base: "HEAD".into(),
        }
    } else {
        Source::Edits {
            repo: i.repo.clone(),
        }
    };

    let (files, changes) = match &source {
        Source::Tree { dir, base } => {
            let mut changes = changes_from_git(dir, &[base.as_str()]);
            let untracked: Vec<String> = git(dir, &["ls-files", "--others", "--exclude-standard"])
                .unwrap_or_default()
                .lines()
                .map(str::to_string)
                .collect();
            for p in &untracked {
                let added = std::fs::read_to_string(dir.join(p))
                    .map(|t| t.lines().count())
                    .unwrap_or(0);
                changes.push(Change {
                    path: p.clone(),
                    status: 'A',
                    added,
                    removed: 0,
                });
            }
            let mut files: Vec<String> = git(dir, &["ls-files"])
                .unwrap_or_default()
                .lines()
                .filter(|p| dir.join(p).exists())
                .map(str::to_string)
                .collect();
            files.extend(untracked);
            (files, changes)
        }
        Source::Commits { repo, base, head } => {
            let files = git(repo, &["ls-tree", "-r", "--name-only", head])
                .unwrap_or_default()
                .lines()
                .map(str::to_string)
                .collect();
            (
                files,
                changes_from_git(repo, &[base.as_str(), head.as_str()]),
            )
        }
        Source::Landed {
            repo,
            base,
            head,
            paths,
        } => {
            let files = git(repo, &["ls-tree", "-r", "--name-only", head])
                .unwrap_or_default()
                .lines()
                .map(str::to_string)
                .collect();
            let changes = changes_from_git(repo, &[base.as_str(), head.as_str()])
                .into_iter()
                .filter(|c| paths.contains(&c.path))
                .collect();
            (files, changes)
        }
        Source::Edits { .. } => {
            let changes: Vec<Change> = edits
                .iter()
                .map(|(path, list)| Change {
                    path: path.clone(),
                    status: if list.iter().any(|e| e.deleted) {
                        'D'
                    } else if list.first().is_some_and(|e| e.created) {
                        'A'
                    } else {
                        'M'
                    },
                    added: list.iter().map(|e| e.added).sum(),
                    removed: list.iter().map(|e| e.removed).sum(),
                })
                .collect();
            (changes.iter().map(|c| c.path.clone()).collect(), changes)
        }
    };
    let mut files = files;
    for c in &changes {
        if !files.contains(&c.path) {
            files.push(c.path.clone());
        }
    }
    files.sort();
    files.dedup();
    Snapshot {
        source,
        files,
        changes,
        edits,
    }
}

const MAX_TEXT: usize = 2 * 1024 * 1024;

fn text_or_note(bytes: Vec<u8>) -> String {
    if bytes.contains(&0) {
        return "Binary file.".into();
    }
    if bytes.len() > MAX_TEXT {
        return format!(
            "{}\n… cut at 2 MB",
            String::from_utf8_lossy(&bytes[..MAX_TEXT])
        );
    }
    String::from_utf8_lossy(&bytes).to_string()
}

/// The unified diff of one file. Runs git; call it off the main thread.
pub fn diff(snap: &Snapshot, path: &str) -> String {
    let text = match &snap.source {
        Source::Tree { dir, base } => {
            let tracked = git(dir, &["ls-files", "--error-unmatch", "--", path]).is_some();
            if tracked || snap.change(path).is_some_and(|c| c.status == 'D') {
                git_any(dir, &["diff", "--no-renames", base, "--", path])
            } else {
                git_any(dir, &["diff", "--no-index", "--", "/dev/null", path])
            }
        }
        Source::Commits { repo, base, head }
        | Source::Landed {
            repo, base, head, ..
        } => git_any(repo, &["diff", "--no-renames", base, head, "--", path]),
        Source::Edits { .. } => String::new(),
    };
    if !text.trim().is_empty() {
        return text;
    }
    // Git has nothing (edits only, or git failed): the transcript's edits.
    snap.edits
        .get(path)
        .map(|list| {
            list.iter()
                .map(|e| e.diff.as_str())
                .collect::<Vec<_>>()
                .join("\n")
        })
        .unwrap_or_default()
}

/// The full text of one file at the snapshot. Runs git; call it off the
/// main thread.
pub fn content(snap: &Snapshot, path: &str) -> String {
    let bytes = match &snap.source {
        Source::Tree { dir, base } => std::fs::read(dir.join(path)).ok().or_else(|| {
            Command::new("git")
                .arg("-C")
                .arg(dir)
                .args(["show", &format!("{base}:{path}")])
                .output()
                .ok()
                .map(|o| o.stdout)
        }),
        Source::Commits { repo, base, head }
        | Source::Landed {
            repo, base, head, ..
        } => [head, base].iter().find_map(|r| {
            Command::new("git")
                .arg("-C")
                .arg(repo)
                .args(["show", &format!("{r}:{path}")])
                .output()
                .ok()
                .filter(|o| o.status.success())
                .map(|o| o.stdout)
        }),
        Source::Edits { repo } => std::fs::read(repo.join(path)).ok(),
    };
    bytes
        .map(text_or_note)
        .unwrap_or_else(|| "The file is not in this snapshot.".into())
}

/// The highlighter language of a file, from its name.
pub fn language(path: &str) -> String {
    let name = Path::new(path)
        .file_name()
        .map(|n| n.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    if name == "makefile" {
        return "make".into();
    }
    if name == "dockerfile" {
        return "bash".into();
    }
    let ext = Path::new(path)
        .extension()
        .map(|e| e.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    match ext.as_str() {
        "mjs" | "cjs" | "jsx" => "javascript".into(),
        "mts" | "cts" => "typescript".into(),
        "jsonc" | "json5" => "json".into(),
        "zsh" | "fish" => "bash".into(),
        "h" => "c".into(),
        "hpp" | "cc" => "cpp".into(),
        "" => "text".into(),
        e => e.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_commits_in_tool_output() {
        let entry = Entry {
            kind: Kind::Tool,
            text: "Commit".into(),
            tool: Some("Bash".into()),
            output: Some(
                "[agent-a1b2 6531e90] feat(repo): name the workspace\n 3 files changed".into(),
            ),
            call: None,
            at: 0,
            edits: Vec::new(),
        };
        let i = inputs(Path::new("/r"), None, false, (0, 0), &[entry]);
        assert_eq!(i.commits, vec!["6531e90".to_string()]);
    }

    #[test]
    fn edits_only_snapshot() {
        let mut e = Edit::between("/r/src/a.rs", "a\n", "b\n");
        e.created = false;
        let i = Inputs {
            repo: PathBuf::from("/r"),
            worktree: None,
            running: false,
            window: (0, 0),
            commits: Vec::new(),
            edits: vec![e],
        };
        let s = snapshot(&i);
        assert_eq!(s.files, vec!["src/a.rs".to_string()]);
        assert_eq!(s.changes[0].status, 'M');
        assert_eq!((s.changes[0].added, s.changes[0].removed), (1, 1));
        assert!(diff(&s, "src/a.rs").contains("+b"));
    }

    #[test]
    fn picks_languages() {
        assert_eq!(language("src/app.rs"), "rs");
        assert_eq!(language("web/x.mjs"), "javascript");
        assert_eq!(language("Makefile"), "make");
    }
}
