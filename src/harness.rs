//! The agent harnesses beyond Claude Code and Codex: where each keeps its
//! sessions, and how a session reads as a conversation. Read-only.
//!
//! Each harness gives two things: `discover` finds its sessions and their
//! working folders, and a parser turns a session into transcript entries.
//! The rest (the ticket a session works on, whether it runs, the session
//! page) is shared.

use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::transcript::{Kind, Provider, Transcript, flatten, summarize, time_of};

/// A session found on disk.
#[derive(Clone, Debug)]
pub struct Found {
    pub provider: Provider,
    /// The session file, or `<db>#<session id>` for a database.
    pub key: PathBuf,
    /// The folder the session worked in.
    pub cwd: PathBuf,
    /// Unix seconds of its last change.
    pub modified: i64,
}

fn home() -> PathBuf {
    dirs::home_dir().unwrap_or_default()
}

fn mtime(p: &Path) -> i64 {
    std::fs::metadata(p)
        .and_then(|m| m.modified())
        .map(crate::agents::unix)
        .unwrap_or(0)
}

/// Every file under `dir` (to `depth` levels) whose name passes `keep`.
fn files(dir: &Path, depth: usize, keep: &dyn Fn(&Path) -> bool, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            if depth > 0 {
                files(&p, depth - 1, keep, out);
            }
        } else if keep(&p) {
            out.push(p);
        }
    }
}

/// Sessions of every harness here, changed within `max_age` seconds, whose
/// folder is inside `repo`.
pub fn discover(repo: &Path, max_age: i64) -> Vec<Found> {
    let now = crate::agents::unix(std::time::SystemTime::now());
    let repo = std::fs::canonicalize(repo).unwrap_or_else(|_| repo.to_path_buf());
    let mut out = Vec::new();
    cursor(&repo, &mut out);
    out.retain(|f| now - f.modified <= max_age && inside(&f.cwd, &repo));
    out
}

fn inside(cwd: &Path, repo: &Path) -> bool {
    let cwd = std::fs::canonicalize(cwd).unwrap_or_else(|_| cwd.to_path_buf());
    cwd.starts_with(repo)
}

/// A line of a JSONL session.
pub fn line(t: &mut Transcript, provider: Provider, v: &Value) {
    if provider == Provider::Cursor {
        cursor_line(t, v)
    }
}

/// A session stored as one JSON document: read it whole when it changed.
pub fn update_document(t: &mut Transcript, path: &Path, provider: Provider) -> bool {
    let Ok(md) = std::fs::metadata(path) else {
        return false;
    };
    let stamp = (md.len(), mtime(path));
    if stamp == t.stamp {
        return false;
    }
    let Ok(text) = std::fs::read_to_string(path) else {
        return false;
    };
    let Ok(v) = serde_json::from_str::<Value>(&text) else {
        return false;
    };
    *t = Transcript::default();
    t.stamp = stamp;
    let _ = (provider, v);
    true
}

/// A session stored in SQLite: read it again when the database changed.
pub fn update_sqlite(t: &mut Transcript, key: &Path, provider: Provider) -> bool {
    let key = key.to_string_lossy();
    let Some((db, _id)) = key.rsplit_once('#') else {
        return false;
    };
    let db = Path::new(db);
    // WAL databases change their -wal file first.
    let wal = db.with_extension(format!(
        "{}-wal",
        db.extension()
            .map(|e| e.to_string_lossy())
            .unwrap_or_default()
    ));
    let stamp = (
        std::fs::metadata(db).map(|m| m.len()).unwrap_or(0),
        mtime(db).max(mtime(&wal)),
    );
    if stamp == t.stamp {
        return false;
    }
    *t = Transcript::default();
    t.stamp = stamp;
    let _ = provider;
    true
}

/// The command that resumes a session in its harness.
pub fn resume_command(provider: Provider, key: &Path) -> Option<String> {
    let id = key.file_stem()?.to_string_lossy().to_string();
    Some(match provider {
        Provider::Cursor => format!("cursor-agent --resume {id}"),
        _ => return None,
    })
}

// ---------- Cursor CLI ----------
//
// `~/.cursor/projects/<folder with / as ->/agent-transcripts/<id>/<id>.jsonl`:
// `{"role": "user" | "assistant", "message": {"content": [parts]}}`. A user
// part wraps the prompt in `<user_query>` and says the time in
// `<timestamp>`. Lines carry no time of their own.

fn cursor(_repo: &Path, out: &mut Vec<Found>) {
    let root = home().join(".cursor/projects");
    let Ok(rd) = std::fs::read_dir(&root) else {
        return;
    };
    for e in rd.flatten() {
        let dir = e.path();
        let name = e.file_name().to_string_lossy().to_string();
        let Some(cwd) = decode_dashes(&name) else {
            continue;
        };
        let mut found = Vec::new();
        files(
            &dir.join("agent-transcripts"),
            2,
            &|p| p.extension().is_some_and(|x| x == "jsonl"),
            &mut found,
        );
        for key in found {
            out.push(Found {
                provider: Provider::Cursor,
                modified: mtime(&key),
                key,
                cwd: cwd.clone(),
            });
        }
    }
}

/// A folder written with every `/` as `-` (`Users-a-Projects-they`): the
/// dashes are ambiguous, so try each split against the disk.
fn decode_dashes(name: &str) -> Option<PathBuf> {
    let parts: Vec<&str> = name.split('-').filter(|p| !p.is_empty()).collect();
    fn walk(base: PathBuf, rest: &[&str]) -> Option<PathBuf> {
        if rest.is_empty() {
            return Some(base);
        }
        // Longest joined name first: `reyz-api` before `reyz` + `api`.
        for n in (1..=rest.len()).rev() {
            for sep in ["-", ".", "_", " "] {
                let seg = rest[..n].join(sep);
                for seg in [seg.clone(), format!(".{seg}")] {
                    let next = base.join(&seg);
                    if next.is_dir()
                        && let Some(found) = walk(next, &rest[n..])
                    {
                        return Some(found);
                    }
                }
                if n == 1 {
                    break;
                }
            }
        }
        None
    }
    walk(PathBuf::from("/"), &parts)
}

fn cursor_line(t: &mut Transcript, v: &Value) {
    let parts = v["message"]["content"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    match v["role"].as_str() {
        Some("user") => {
            for p in &parts {
                let Some(text) = p["text"].as_str() else {
                    continue;
                };
                let at = tag(text, "timestamp")
                    .and_then(|s| cursor_time(&s))
                    .unwrap_or(0);
                let prompt = tag(text, "user_query").unwrap_or_else(|| text.to_string());
                if !prompt.trim_start().starts_with('<') {
                    t.turn_start = at;
                    t.turn_tokens = 0;
                    t.push(Kind::User, prompt, at);
                }
            }
        }
        Some("assistant") => {
            for p in &parts {
                match p["type"].as_str() {
                    Some("text") => {
                        let text = p["text"]
                            .as_str()
                            .unwrap_or_default()
                            .replace("[REDACTED]", "");
                        t.push(Kind::Assistant, text, 0);
                    }
                    Some("thinking" | "reasoning") => {
                        let text = p["text"]
                            .as_str()
                            .or(p["thinking"].as_str())
                            .unwrap_or_default();
                        t.push(Kind::Thinking, text.to_string(), 0);
                    }
                    Some("tool_use" | "tool-call" | "tool_call") => {
                        let name = p["name"]
                            .as_str()
                            .or(p["toolName"].as_str())
                            .unwrap_or("tool");
                        let input = if p["input"].is_null() {
                            &p["args"]
                        } else {
                            &p["input"]
                        };
                        let id = p["id"]
                            .as_str()
                            .or(p["toolCallId"].as_str())
                            .map(str::to_string);
                        t.push_tool(name, summarize(input), id, 0);
                    }
                    _ => {}
                }
            }
        }
        Some("tool") => {
            for p in &parts {
                if let Some(id) = p["tool_use_id"].as_str().or(p["toolCallId"].as_str()) {
                    let out = if p["content"].is_null() {
                        flatten(&p["result"])
                    } else {
                        flatten(&p["content"])
                    };
                    t.attach_output(id, out);
                }
            }
        }
        _ => {}
    }
    let _ = time_of;
}

/// `Sunday, Sep 20, 2026, 12:31 AM (UTC+3)`.
fn cursor_time(s: &str) -> Option<i64> {
    let (body, zone) = s.rsplit_once(" (UTC")?;
    let zone = zone.trim_end_matches(')');
    let offset: i32 = if zone.is_empty() {
        0
    } else {
        zone.split(':').next()?.parse().ok()?
    };
    let body = body.split_once(", ").map_or(body, |(_, rest)| rest);
    let naive = chrono::NaiveDateTime::parse_from_str(body, "%b %d, %Y, %I:%M %p").ok()?;
    Some(naive.and_utc().timestamp() - i64::from(offset) * 3600)
}

/// The text inside `<name>…</name>`.
fn tag(s: &str, name: &str) -> Option<String> {
    let open = format!("<{name}>");
    let a = s.find(&open)? + open.len();
    let b = s[a..].find(&format!("</{name}>"))? + a;
    Some(s[a..b].trim().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_cursor_times_and_prompts() {
        assert_eq!(
            cursor_time("Sunday, Sep 20, 2026, 12:31 AM (UTC+3)"),
            Some(1_789_853_460)
        );
        let mut t = Transcript::default();
        cursor_line(
            &mut t,
            &serde_json::json!({"role":"user","message":{"content":[{"type":"text","text":"<timestamp>Sunday, Sep 20, 2026, 12:31 AM (UTC+3)</timestamp>\n<user_query>\nImplement WS-5\n</user_query>"}]}}),
        );
        cursor_line(
            &mut t,
            &serde_json::json!({"role":"assistant","message":{"content":[{"type":"text","text":"Done.\n\n[REDACTED]"}]}}),
        );
        assert_eq!(t.entries.len(), 2);
        assert_eq!(t.entries[0].text, "Implement WS-5");
        assert_eq!(t.entries[1].text, "Done.");
    }

    #[test]
    fn decodes_dashed_folders() {
        let home = home();
        let name = home.display().to_string().replace('/', "-");
        assert_eq!(decode_dashes(&name), Some(home));
    }
}
