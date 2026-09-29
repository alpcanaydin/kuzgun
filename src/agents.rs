//! Live agent work, read from Claude Code's own session transcripts, so
//! the board shows what agents are doing even when the ticket files do not
//! say it (`/implement` does not write a status line).
//!
//! Sources, all under `~/.claude/projects/<encoded repo path>/`:
//!
//! - `<session>.jsonl`: a top-level session.
//! - `<session>/subagents/agent-<id>.jsonl` with `agent-<id>.meta.json`
//!   (`worktreePath`, `description`): a background agent, usually in the
//!   worktree `.claude/worktrees/agent-<id>`.
//!
//! A background agent maps to the ticket its first prompt names. A
//! top-level session maps to the last ticket it was asked about since its
//! last `/clear`, for example `/implement 115`. A run is running while its
//! last turn is open, and waits on review while its worktree still exists
//! after the agent stopped.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, Mutex};
use std::time::SystemTime;

use regex::Regex;

pub use crate::transcript::Provider;

/// Stopped agents without a worktree count as landed work this long.
const FINISHED_SECS: i64 = 3 * 86_400;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RunState {
    /// The transcript is still being written.
    Running,
    /// The agent stopped; its worktree (with the work) is still there.
    AwaitingReview,
    /// The agent stopped and its worktree is gone (merged or removed).
    Finished,
}

#[derive(Clone, Debug)]
pub struct AgentRun {
    /// The ticket as the prompt names it: a path relative to the repo
    /// root, a key such as `WS-115`, or a bare number such as `115`.
    pub ticket_rel: String,
    pub state: RunState,
    pub description: String,
    pub worktree: Option<PathBuf>,
    pub transcript: PathBuf,
    pub provider: Provider,
    /// Unix seconds.
    pub started: i64,
    pub last_activity: i64,
}

static TICKET_PATH: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"((?:\.scratch|[A-Za-z0-9_.-]+)/[A-Za-z0-9_./-]*?issues/[A-Za-z0-9_.-]+\.md)")
        .unwrap()
});
/// A skill run on a ticket: `<command-name>/implement</command-name>
/// <command-args>115</command-args>`, or the same typed as plain text.
static SKILL_CALL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?s)<command-name>/?(?:[\w-]+:)?([\w-]+)</command-name>.*?<command-args>(.*?)</command-args>|(?m)^[/$](?:[\w-]+:)?([\w-]+)[ \t]+(\S.*)$").unwrap()
});
static TICKET_KEY: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\b([A-Z][A-Z0-9]{0,5}-\d{1,5})\b").unwrap());
static TICKET_NUM: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?:^|[\s#])(\d{1,5})\b").unwrap());
/// Skills that work on one ticket.
const TICKET_SKILLS: [&str; 9] = [
    "implement",
    "implement-spec",
    "wayfinder",
    "triage",
    "tdd",
    "diagnosing-bugs",
    "prototype",
    "research",
    "code-review",
];
static NAMED_TICKET: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)(?:implement|ticket|work on|resolve|claim)\W{0,20}`?(\.scratch/[A-Za-z0-9_./-]+?\.md)").unwrap()
});

/// `~/.claude/projects/` name of a directory: every non-alphanumeric
/// character becomes `-`.
pub fn project_dir(repo: &Path) -> Option<PathBuf> {
    let key: String = repo
        .to_string_lossy()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    let dir = dirs::home_dir()?.join(".claude/projects").join(key);
    dir.is_dir().then_some(dir)
}

/// The git top level above `dir` (a `.git` file or folder), if any.
pub fn repo_root(dir: &Path) -> Option<PathBuf> {
    let mut d = Some(dir);
    while let Some(p) = d {
        if p.join(".git").exists() {
            return Some(p.to_path_buf());
        }
        d = p.parent();
    }
    None
}

/// Every transcript of the repo and of its worktrees' own sessions.
pub fn transcript_dirs(repo: &Path) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = project_dir(repo).into_iter().collect();
    // Sessions started inside a worktree live under the worktree's key.
    let wt = repo.join(".claude/worktrees");
    if let Ok(rd) = std::fs::read_dir(&wt) {
        for e in rd.flatten() {
            if let Some(d) = project_dir(&e.path()) {
                out.push(d);
            }
        }
    }
    out
}

pub fn unix(t: SystemTime) -> i64 {
    t.duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// The first user prompt of a transcript (its task), from the file head.
fn first_prompt(path: &Path) -> Option<String> {
    use std::io::Read;
    let mut f = std::fs::File::open(path).ok()?;
    let mut buf = vec![0u8; 256 * 1024];
    let n = f.read(&mut buf).ok()?;
    let head = String::from_utf8_lossy(&buf[..n]);
    for line in head.lines() {
        let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        if v.get("type").and_then(|t| t.as_str()) != Some("user") {
            continue;
        }
        let content = &v["message"]["content"];
        let text = match content {
            serde_json::Value::String(s) => s.clone(),
            serde_json::Value::Array(parts) => parts
                .iter()
                .filter_map(|p| p.get("text").and_then(|t| t.as_str()))
                .collect::<Vec<_>>()
                .join(" "),
            _ => continue,
        };
        if !text.trim().is_empty() {
            return Some(text);
        }
    }
    None
}

/// The state of the last turn, read from the transcript's own entries,
/// not from the file time: Claude Code keeps writing bookkeeping entries
/// (`turn_duration`, `away_summary`) after a turn ends.
///
/// Returns whether the turn is still open and the Unix time of its last
/// message. A turn is open after a prompt or a tool result, and while an
/// assistant message has not ended with `end_turn` or `stop_sequence`.
fn turn_state(path: &Path) -> (bool, i64) {
    use std::io::{Read, Seek, SeekFrom};
    let Ok(mut f) = std::fs::File::open(path) else {
        return (false, 0);
    };
    let len = f.metadata().map(|m| m.len()).unwrap_or(0);
    let _ = f.seek(SeekFrom::Start(len.saturating_sub(256 * 1024)));
    let mut bytes = Vec::new();
    if f.read_to_end(&mut bytes).is_err() {
        return (false, 0);
    }
    let buf = String::from_utf8_lossy(&bytes);
    let at = |v: &serde_json::Value| {
        v["timestamp"]
            .as_str()
            .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
            .map_or(0, |d| d.timestamp())
    };
    for line in buf.lines().rev() {
        let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        match (
            v.get("type").and_then(|t| t.as_str()),
            v.get("subtype").and_then(|t| t.as_str()),
        ) {
            (Some("system"), Some("turn_duration")) => return (false, at(&v)),
            (Some("assistant"), _) => {
                let open = !matches!(
                    v["message"]["stop_reason"].as_str(),
                    Some("end_turn" | "stop_sequence")
                );
                return (open, at(&v));
            }
            (Some("user"), _) => {
                let interrupted = line.contains("[Request interrupted by user");
                return (!interrupted, at(&v));
            }
            _ => continue,
        }
    }
    (false, 0)
}

/// The ticket a prompt is about: the path after "implement ticket …",
/// else the first ticket path it names.
fn ticket_of(prompt: &str) -> Option<String> {
    if let Some(c) = SKILL_CALL.captures(prompt) {
        let name = c.get(1).or(c.get(3)).map_or("", |m| m.as_str());
        let args = c.get(2).or(c.get(4)).map_or("", |m| m.as_str());
        if TICKET_SKILLS.contains(&name) {
            let found = TICKET_PATH
                .captures(args)
                .map(|c| c[1].to_string())
                .or_else(|| TICKET_KEY.captures(args).map(|c| c[1].to_string()))
                .or_else(|| TICKET_NUM.captures(args).map(|c| c[1].to_string()));
            if found.is_some() {
                return found;
            }
        }
    }
    NAMED_TICKET
        .captures(prompt)
        .map(|c| c[1].to_string())
        .or_else(|| TICKET_PATH.captures(prompt).map(|c| c[1].to_string()))
}

/// What the app last read of a top-level transcript: the byte offset and
/// the latest ticket asked about, with the Unix time of that prompt.
type Seen = (u64, Option<(String, i64)>);
static SEEN: LazyLock<Mutex<HashMap<PathBuf, Seen>>> = LazyLock::new(|| Mutex::new(HashMap::new()));

fn user_text(v: &serde_json::Value) -> Option<String> {
    if v.get("type").and_then(|t| t.as_str()) != Some("user")
        || v.get("isMeta").and_then(|m| m.as_bool()) == Some(true)
    {
        return None;
    }
    match &v["message"]["content"] {
        serde_json::Value::String(s) => Some(s.clone()),
        serde_json::Value::Array(parts) => {
            let text: Vec<&str> = parts
                .iter()
                .filter(|p| p.get("type").and_then(|t| t.as_str()) == Some("text"))
                .filter_map(|p| p.get("text").and_then(|t| t.as_str()))
                .collect();
            (!text.is_empty()).then(|| text.join(" "))
        }
        _ => None,
    }
}

/// The last ticket a top-level session was asked about since its last
/// `/clear`. Reads only the bytes added since the previous scan.
fn latest_ticket(path: &Path) -> Option<(String, i64)> {
    latest_ticket_with(path, user_text)
}

fn latest_ticket_with(
    path: &Path,
    prompt_of: fn(&serde_json::Value) -> Option<String>,
) -> Option<(String, i64)> {
    use std::io::{Read, Seek, SeekFrom};
    let len = std::fs::metadata(path).ok()?.len();
    let mut seen = SEEN.lock().ok()?;
    let entry = seen.entry(path.to_path_buf()).or_insert((0, None));
    if entry.0 > len {
        *entry = (0, None);
    }
    if entry.0 < len {
        let mut f = std::fs::File::open(path).ok()?;
        f.seek(SeekFrom::Start(entry.0)).ok()?;
        let mut buf = Vec::with_capacity((len - entry.0) as usize);
        f.take(len - entry.0).read_to_end(&mut buf).ok()?;
        // Keep a line that is still being written for the next scan.
        let end = buf.iter().rposition(|&b| b == b'\n').map_or(0, |i| i + 1);
        for line in String::from_utf8_lossy(&buf[..end]).lines() {
            if !line.contains("\"user\"") {
                continue;
            }
            let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else {
                continue;
            };
            let Some(text) = prompt_of(&v) else {
                continue;
            };
            if text.contains("<command-name>/clear</command-name>") {
                entry.1 = None;
            } else if !text.starts_with("Another Claude session sent a message")
                && let Some(t) = ticket_of(&text)
            {
                let at = v["timestamp"]
                    .as_str()
                    .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
                    .map_or(0, |d| d.timestamp());
                entry.1 = Some((t, at));
            }
        }
        entry.0 += end as u64;
    }
    entry.1.clone()
}

fn run_of(
    transcript: &Path,
    meta: Option<serde_json::Value>,
    now: i64,
    max_age: i64,
) -> Option<AgentRun> {
    let md = std::fs::metadata(transcript).ok()?;
    let last = md.modified().map(unix).unwrap_or(0);
    let started = md.created().map(unix).unwrap_or(last);
    let (prompt, ticket_rel, started) = if meta.is_some() {
        let prompt = first_prompt(transcript)?;
        let t = ticket_of(&prompt)?;
        (prompt, t, started)
    } else {
        let (t, at) = latest_ticket(transcript)?;
        (
            format!("Session on {t}"),
            t,
            if at > 0 { at } else { started },
        )
    };
    let worktree = meta
        .as_ref()
        .and_then(|m| m.get("worktreePath"))
        .and_then(|w| w.as_str())
        .map(PathBuf::from)
        .filter(|p| p.is_dir());
    let description = meta
        .as_ref()
        .and_then(|m| m.get("description"))
        .and_then(|d| d.as_str())
        .map(str::to_string)
        .unwrap_or_else(|| prompt.chars().take(80).collect());
    let (open, said) = turn_state(transcript);
    let last = if said > 0 { said } else { last };
    // An open turn with no message for two hours is a crashed session.
    let state = if open && now - last <= 2 * 3600 {
        RunState::Running
    } else if worktree.is_some() {
        RunState::AwaitingReview
    } else if now - last <= max_age {
        RunState::Finished
    } else {
        return None;
    };
    Some(AgentRun {
        ticket_rel,
        state,
        description,
        worktree,
        transcript: transcript.to_path_buf(),
        provider: Provider::Claude,
        started,
        last_activity: last,
    })
}

/// The prompt text of a Codex user message. Context the app injects comes
/// as tagged user messages and is not a prompt.
fn codex_user_text(v: &serde_json::Value) -> Option<String> {
    let p = &v["payload"];
    if v["type"].as_str() != Some("response_item")
        || p["type"].as_str() != Some("message")
        || p["role"].as_str() != Some("user")
    {
        return None;
    }
    let text = p["content"]
        .as_array()?
        .iter()
        .filter_map(|c| c["text"].as_str())
        .collect::<Vec<_>>()
        .join("\n");
    (!text.trim_start().starts_with('<')).then_some(text)
}

/// Whether the last Codex turn is still open, and the time of its last
/// event.
fn codex_turn_state(path: &Path) -> (bool, i64) {
    use std::io::{Read, Seek, SeekFrom};
    let Ok(mut f) = std::fs::File::open(path) else {
        return (false, 0);
    };
    let len = f.metadata().map(|m| m.len()).unwrap_or(0);
    let _ = f.seek(SeekFrom::Start(len.saturating_sub(256 * 1024)));
    let mut bytes = Vec::new();
    if f.read_to_end(&mut bytes).is_err() {
        return (false, 0);
    }
    let buf = String::from_utf8_lossy(&bytes);
    let mut last = 0;
    for line in buf.lines().rev() {
        let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        let at = v["timestamp"]
            .as_str()
            .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
            .map_or(0, |d| d.timestamp());
        if last == 0 {
            last = at;
        }
        if v["type"].as_str() == Some("event_msg") {
            match v["payload"]["type"].as_str() {
                Some("task_started") => return (true, last),
                Some("task_complete" | "turn_aborted") => return (false, last),
                _ => {}
            }
        }
    }
    (false, last)
}

/// Codex sessions of the last days whose working folder is in the repo.
fn codex_runs(repo: &Path, now: i64, max_age: i64) -> Vec<AgentRun> {
    let Some(root) = dirs::home_dir().map(|h| h.join(".codex/sessions")) else {
        return Vec::new();
    };
    // Sessions sit in day folders: `YYYY/MM/DD`. Recent scans read only the
    // last days; a history scan reads them all.
    let days: Vec<PathBuf> = if max_age <= FINISHED_SECS {
        (0..=(max_age / 86_400))
            .map(|back| {
                root.join(
                    (chrono::Local::now() - chrono::Duration::days(back))
                        .format("%Y/%m/%d")
                        .to_string(),
                )
            })
            .collect()
    } else {
        let sub = |p: &Path| -> Vec<PathBuf> {
            std::fs::read_dir(p)
                .map(|rd| {
                    rd.flatten()
                        .map(|e| e.path())
                        .filter(|p| p.is_dir())
                        .collect()
                })
                .unwrap_or_default()
        };
        sub(&root)
            .iter()
            .flat_map(|y| sub(y))
            .flat_map(|m| sub(&m))
            .collect()
    };
    let mut out = Vec::new();
    for dir in days {
        let Ok(rd) = std::fs::read_dir(&dir) else {
            continue;
        };
        for e in rd.flatten() {
            let path = e.path();
            if path.extension().is_none_or(|x| x != "jsonl") {
                continue;
            }
            let Some(cwd) = codex_cwd(&path) else {
                continue;
            };
            if !cwd.starts_with(repo) {
                continue;
            }
            let Some((ticket_rel, asked)) = latest_ticket_with(&path, codex_user_text) else {
                continue;
            };
            let md = std::fs::metadata(&path).ok();
            let started = md
                .as_ref()
                .and_then(|m| m.created().ok())
                .map(unix)
                .unwrap_or(asked);
            let (open, said) = codex_turn_state(&path);
            let last = if said > 0 {
                said
            } else {
                md.and_then(|m| m.modified().ok()).map(unix).unwrap_or(0)
            };
            let state = if open && now - last <= 2 * 3600 {
                RunState::Running
            } else if now - last <= max_age {
                RunState::Finished
            } else {
                continue;
            };
            out.push(AgentRun {
                description: format!("Codex session on {ticket_rel}"),
                ticket_rel,
                state,
                worktree: None,
                transcript: path,
                provider: Provider::Codex,
                started: if asked > 0 { asked } else { started },
                last_activity: last,
            });
        }
    }
    out
}

/// The working folder of a Codex session, from its first line.
fn codex_cwd(path: &Path) -> Option<PathBuf> {
    use std::io::BufRead;
    let f = std::fs::File::open(path).ok()?;
    let mut first = String::new();
    std::io::BufReader::new(f).read_line(&mut first).ok()?;
    let v: serde_json::Value = serde_json::from_str(&first).ok()?;
    v["payload"]["cwd"].as_str().map(PathBuf::from)
}

/// Agent runs that touch a ticket of this repo, newest activity first.
/// Only transcripts written in the last three days are opened.
pub fn scan(repo: &Path) -> Vec<AgentRun> {
    scan_within(repo, FINISHED_SECS)
}

/// Every agent run on this repo's tickets, of any age. Heavier than
/// [`scan`]: read once per board, for the sessions of closed tickets.
pub fn history(repo: &Path) -> Vec<AgentRun> {
    scan_within(repo, i64::MAX)
}

fn scan_within(repo: &Path, max_age: i64) -> Vec<AgentRun> {
    let now = unix(SystemTime::now());
    let fresh = |p: &Path| {
        std::fs::metadata(p)
            .and_then(|m| m.modified())
            .map(|t| now - unix(t) <= max_age)
            .unwrap_or(false)
    };
    let mut runs = Vec::new();
    for dir in transcript_dirs(repo) {
        let Ok(rd) = std::fs::read_dir(&dir) else {
            continue;
        };
        for e in rd.flatten() {
            let p = e.path();
            if p.extension().is_some_and(|x| x == "jsonl") {
                if fresh(&p)
                    && let Some(r) = run_of(&p, None, now, max_age)
                {
                    runs.push(r);
                }
            } else if p.is_dir() {
                let sub = p.join("subagents");
                let Ok(rd) = std::fs::read_dir(&sub) else {
                    continue;
                };
                for e in rd.flatten() {
                    let t = e.path();
                    if t.extension().is_none_or(|x| x != "jsonl") || !fresh(&t) {
                        continue;
                    }
                    let meta = std::fs::read_to_string(t.with_extension("meta.json"))
                        .ok()
                        .and_then(|s| serde_json::from_str(&s).ok());
                    if let Some(r) = run_of(&t, meta, now, max_age) {
                        runs.push(r);
                    }
                }
            }
        }
    }
    runs.extend(codex_runs(repo, now, max_age));
    runs.sort_by_key(|r| std::cmp::Reverse(r.last_activity));
    runs
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_the_implemented_ticket() {
        let p = "You work in the They repo. Implement ticket `.scratch/walking-skeleton/issues/111-demo-seed.md` end to end. See `.scratch/product-v2/issues/62-handover.md`.";
        assert_eq!(
            ticket_of(p).as_deref(),
            Some(".scratch/walking-skeleton/issues/111-demo-seed.md")
        );
        assert_eq!(
            ticket_of("/implement .scratch/x/issues/05-a.md").as_deref(),
            Some(".scratch/x/issues/05-a.md")
        );
        assert_eq!(ticket_of("hello"), None);
        assert_eq!(
            ticket_of("<command-message>implement</command-message> <command-name>/implement</command-name> <command-args>115</command-args>").as_deref(),
            Some("115")
        );
        assert_eq!(ticket_of("/implement WS-94 now").as_deref(), Some("WS-94"));
        assert_eq!(ticket_of("/grill-me 12"), None);
    }

    #[test]
    fn reads_the_turn_state() {
        let path = std::env::temp_dir().join(format!("kuzgun-turn-{}.jsonl", std::process::id()));
        let entry = |kind: &str, extra: &str| {
            format!("{{\"type\":\"{kind}\",\"timestamp\":\"2026-09-28T20:00:00Z\"{extra}}}\n")
        };
        let open = entry("user", ",\"message\":{\"content\":\"go\"}")
            + &entry("assistant", ",\"message\":{\"stop_reason\":\"tool_use\"}");
        std::fs::write(&path, &open).unwrap();
        assert!(turn_state(&path).0);

        let ended = open.clone()
            + &entry("assistant", ",\"message\":{\"stop_reason\":\"end_turn\"}")
            + &entry("system", ",\"subtype\":\"turn_duration\"")
            + &entry("system", ",\"subtype\":\"away_summary\"");
        std::fs::write(&path, &ended).unwrap();
        assert!(!turn_state(&path).0);

        let interrupted = open
            + &entry(
                "user",
                ",\"message\":{\"content\":\"[Request interrupted by user]\"}",
            );
        std::fs::write(&path, &interrupted).unwrap();
        assert!(!turn_state(&path).0);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn encodes_project_dirs() {
        let key: String = "/Users/a/they/.claude/worktrees/agent-1"
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
            .collect();
        assert_eq!(key, "-Users-a-they--claude-worktrees-agent-1");
    }
}
