//! The agent harnesses beyond Claude Code and Codex: where each keeps its
//! sessions, and how a session reads as a conversation. Read-only.
//!
//! Each harness gives two things: `discover` finds its sessions and their
//! working folders, and a parser turns a session into transcript entries.
//! The rest (the ticket a session works on, whether it runs, the session
//! page) is shared.
//!
//! Formats, by harness:
//! - Cursor: `~/.cursor/projects/<path as dashes>/agent-transcripts/<id>/<id>.jsonl`.
//! - Gemini CLI: `~/.gemini/tmp/<project>/chats/session-*.jsonl` (or a legacy
//!   `.json`); a message is re-appended with the same `id` as it changes.
//! - OpenCode: `~/.local/share/opencode/opencode.db`, tables `session`,
//!   `message` and `part`.
//! - Kimi: `~/.kimi/sessions/<md5 of the folder>/<id>/wire.jsonl`, and the
//!   newer `~/.kimi-code/sessions/<ws>/<id>/agents/main/wire.jsonl`.
//! - Copilot CLI: `~/.copilot/session-state/<id>/events.jsonl`.
//! - Junie: `~/.junie/sessions/session-*/events.jsonl`.
//! - Hermes: `~/.hermes/state.db`, tables `sessions` and `messages`.
//! - Pi: `~/.pi/agent/sessions/--<folder>--/<ts>_<id>.jsonl`.
//! - Amp: `~/.local/share/amp/threads/T-*.json` (older Amp versions; newer
//!   ones keep threads on the server).
//! - Grok CLI: `~/.grok/sessions/<folder>/<id>/chat_history.jsonl`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::transcript::{Edit, Kind, Provider, Transcript, cut, flatten, patch_edits, summarize};

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

/// Every file under `dir` (to `depth` levels) that passes `keep`.
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

fn subdirs(dir: &Path) -> Vec<PathBuf> {
    std::fs::read_dir(dir)
        .map(|rd| {
            rd.flatten()
                .map(|e| e.path())
                .filter(|p| p.is_dir())
                .collect()
        })
        .unwrap_or_default()
}

fn read_json(p: &Path) -> Option<Value> {
    serde_json::from_str(&std::fs::read_to_string(p).ok()?).ok()
}

/// The first line of a file, parsed.
fn first_line(p: &Path) -> Option<Value> {
    use std::io::BufRead;
    let f = std::fs::File::open(p).ok()?;
    let mut line = String::new();
    std::io::BufReader::new(f).read_line(&mut line).ok()?;
    serde_json::from_str(&line).ok()
}

fn expand_home(s: &str) -> PathBuf {
    match s.strip_prefix("~/") {
        Some(rest) => home().join(rest),
        None => PathBuf::from(s),
    }
}

/// `%2FUsers%2Fa` → `/Users/a`.
fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && i + 2 < bytes.len()
            && let Ok(b) = u8::from_str_radix(&s[i + 1..i + 3], 16)
        {
            out.push(b);
            i += 3;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).to_string()
}

/// ISO 8601 time → Unix seconds.
fn iso(v: &Value) -> i64 {
    v.as_str()
        .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
        .map_or(0, |d| d.timestamp())
}

/// Epoch milliseconds → seconds.
fn ms(v: &Value) -> i64 {
    v.as_i64()
        .or_else(|| v.as_f64().map(|f| f as i64))
        .map_or(0, |m| m / 1000)
}

/// Text of a message content: a string, or the text of its parts.
fn text_of(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Array(parts) => parts
            .iter()
            .filter_map(|p| p["text"].as_str().or_else(|| p.as_str()))
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    }
}

/// Tool arguments that may arrive as a JSON string.
fn args_of(v: &Value) -> Value {
    match v {
        Value::String(s) => serde_json::from_str(s).unwrap_or(Value::String(s.clone())),
        other => other.clone(),
    }
}

/// The text inside `<name>…</name>`.
fn tag(s: &str, name: &str) -> Option<String> {
    let open = format!("<{name}>");
    let a = s.find(&open)? + open.len();
    let b = s[a..].find(&format!("</{name}>"))? + a;
    Some(s[a..b].trim().to_string())
}

/// The file changes of a tool call, whatever the harness calls its fields.
fn edits_of(name: &str, input: &Value) -> Vec<Edit> {
    let lower: String = name
        .to_lowercase()
        .chars()
        .filter(|c| c.is_alphanumeric())
        .collect();
    let writes = [
        "edit",
        "write",
        "create",
        "replace",
        "patch",
        "filechange",
        "delete",
    ];
    if lower.contains("read") || !writes.iter().any(|w| lower.contains(w)) {
        return Vec::new();
    }
    // A patch: the whole input as text, or a patch field.
    if let Some(p) = input.as_str() {
        return if p.contains("*** ") {
            patch_edits(p)
        } else {
            Vec::new()
        };
    }
    for key in ["patchText", "patch", "input"] {
        if let Some(p) = input[key].as_str()
            && p.contains("*** ")
        {
            return patch_edits(p);
        }
    }
    let s = |keys: &[&str]| {
        keys.iter()
            .find_map(|k| input[*k].as_str())
            .map(str::to_string)
    };
    let Some(path) = s(&["path", "file_path", "filePath", "target_file", "filename"]) else {
        return Vec::new();
    };
    // Whole-file writes.
    if let Some(content) = s(&["content", "contents", "file_text", "fileText"])
        && input["old_string"].is_null()
    {
        let mut e = Edit::between(&path, "", &content);
        e.created = true;
        return vec![e];
    }
    // Replacements: one, or a list (Pi `edits`, Kimi `edit`).
    let pairs = |v: &Value| -> Vec<(String, String)> {
        let one = |e: &Value| {
            let o = ["oldText", "old", "old_string", "oldString", "old_str"]
                .iter()
                .find_map(|k| e[*k].as_str())?;
            let n = ["newText", "new", "new_string", "newString", "new_str"]
                .iter()
                .find_map(|k| e[*k].as_str())?;
            Some((o.to_string(), n.to_string()))
        };
        match v {
            Value::Array(a) => a.iter().filter_map(one).collect(),
            Value::Object(_) => one(v).into_iter().collect(),
            _ => Vec::new(),
        }
    };
    let mut list = pairs(&input["edits"]);
    list.extend(pairs(&input["edit"]));
    if list.is_empty() {
        list = pairs(input);
    }
    list.into_iter()
        .map(|(o, n)| Edit::between(&path, &o, &n))
        .collect()
}

/// A unified diff as one edit.
fn diff_edit(path: &str, diff: &str) -> Edit {
    let (mut added, mut removed) = (0, 0);
    for l in diff.lines() {
        if l.starts_with('+') && !l.starts_with("+++") {
            added += 1;
        } else if l.starts_with('-') && !l.starts_with("---") {
            removed += 1;
        }
    }
    Edit {
        path: path.to_string(),
        diff: diff.to_string(),
        added,
        removed,
        created: false,
        deleted: false,
    }
}

impl Transcript {
    /// A tool call with its file changes.
    fn call(&mut self, name: &str, input: &Value, id: Option<String>, at: i64) {
        self.push_tool(name, summarize(input), id, at);
        let edits = edits_of(name, input);
        if let Some(e) = self.entries.last_mut() {
            e.edits = edits;
        }
    }

    /// A user prompt: a new turn begins.
    fn prompt(&mut self, text: String, at: i64) {
        let text = text.trim().to_string();
        if text.is_empty() {
            return;
        }
        self.turn_start = at;
        self.turn_tokens = 0;
        self.push(Kind::User, text, at);
    }

    /// Replaces or appends an entry a harness sends again as it changes
    /// (Junie steps), keyed in `call`.
    fn upsert(
        &mut self,
        kind: Kind,
        key: String,
        text: String,
        tool: Option<&str>,
        at: i64,
    ) -> Option<usize> {
        if let Some(i) = self
            .entries
            .iter()
            .rposition(|e| e.call.as_deref() == Some(key.as_str()))
        {
            let e = &mut self.entries[i];
            if !text.trim().is_empty() {
                e.text = cut(text.trim(), 12_000);
            }
            if at > 0 {
                e.at = at;
            }
            return Some(i);
        }
        if kind == Kind::Tool {
            self.push_tool(tool.unwrap_or("tool"), text, Some(key), at);
        } else {
            let before = self.entries.len();
            self.push(kind, text, at);
            if self.entries.len() == before {
                return None;
            }
            if let Some(e) = self.entries.last_mut() {
                e.call = Some(key);
            }
        }
        Some(self.entries.len() - 1)
    }
}

/// Sessions of every harness here, changed within `max_age` seconds, whose
/// folder is inside `repo`.
pub fn discover(repo: &Path, max_age: i64) -> Vec<Found> {
    let now = crate::agents::unix(std::time::SystemTime::now());
    let repo = std::fs::canonicalize(repo).unwrap_or_else(|_| repo.to_path_buf());
    let mut out = Vec::new();
    cursor(&mut out);
    gemini(&mut out);
    opencode(&mut out);
    kimi(&mut out);
    copilot(&mut out);
    junie(&mut out);
    hermes(&mut out);
    pi(&mut out);
    amp(&mut out);
    grok(&mut out);
    out.retain(|f| now - f.modified <= max_age && inside(&f.cwd, &repo));
    out
}

fn inside(cwd: &Path, repo: &Path) -> bool {
    let cwd = std::fs::canonicalize(cwd).unwrap_or_else(|_| cwd.to_path_buf());
    cwd.starts_with(repo)
}

/// A line of a JSONL session.
pub fn line(t: &mut Transcript, provider: Provider, v: &Value) {
    match provider {
        Provider::Cursor => cursor_line(t, v),
        Provider::Kimi => kimi_line(t, v),
        Provider::Copilot => copilot_line(t, v),
        Provider::Junie => junie_line(t, v),
        Provider::Pi => pi_line(t, v),
        Provider::Grok => grok_line(t, v),
        _ => {}
    }
}

/// A session read whole when it changed: a JSON document, or JSONL whose
/// records overwrite earlier ones (Gemini).
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
    *t = Transcript::default();
    t.stamp = stamp;
    match provider {
        Provider::Gemini => gemini_parse(t, &text, path),
        Provider::Amp => {
            if let Ok(v) = serde_json::from_str::<Value>(&text) {
                amp_parse(t, &v);
            }
        }
        _ => {}
    }
    true
}

/// A session stored in SQLite: read again when its rows changed.
pub fn update_sqlite(t: &mut Transcript, key: &Path, provider: Provider) -> bool {
    let key = key.to_string_lossy();
    let Some((db, id)) = key.rsplit_once('#') else {
        return false;
    };
    let Some(conn) = open_db(Path::new(db)) else {
        return false;
    };
    let sql = match provider {
        Provider::OpenCode => {
            "SELECT COUNT(*), COALESCE(MAX(time_updated), 0) FROM part WHERE session_id = ?1"
        }
        _ => {
            "SELECT COUNT(*), CAST(COALESCE(MAX(timestamp), 0) * 1000 AS INTEGER) FROM messages WHERE session_id = ?1"
        }
    };
    let Ok(stamp) = conn.query_row(sql, [id], |r| {
        Ok((r.get::<_, i64>(0)? as u64, r.get::<_, i64>(1)?))
    }) else {
        return false;
    };
    if stamp == t.stamp {
        return false;
    }
    *t = Transcript::default();
    t.stamp = stamp;
    match provider {
        Provider::OpenCode => opencode_parse(t, &conn, id),
        Provider::Hermes => hermes_parse(t, &conn, id),
        _ => {}
    }
    true
}

/// Opens a harness database read-only. The harness keeps writing to it
/// (WAL), so Kuzgun never takes a write lock, and waits a little when the
/// harness is mid-write.
pub(crate) fn open_db(db: &Path) -> Option<rusqlite::Connection> {
    use rusqlite::OpenFlags;
    if !db.is_file() {
        return None;
    }
    let c = rusqlite::Connection::open_with_flags(
        db,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .ok()?;
    let _ = c.busy_timeout(std::time::Duration::from_millis(500));
    Some(c)
}

/// The command that resumes a session in its harness.
pub fn resume_command(provider: Provider, key: &Path) -> Option<String> {
    let text = key.to_string_lossy();
    let db_id = text.rsplit_once('#').map(|(_, id)| id.to_string());
    let stem = key.file_stem()?.to_string_lossy().to_string();
    let parent = || {
        key.parent()
            .and_then(|p| p.file_name())
            .map(|n| n.to_string_lossy().to_string())
    };
    Some(match provider {
        Provider::Cursor => format!("cursor-agent --resume {stem}"),
        Provider::Gemini => {
            let id = first_line(key).and_then(|v| v["sessionId"].as_str().map(str::to_string))?;
            format!("gemini --resume {id}")
        }
        Provider::OpenCode => format!("opencode -s {}", db_id?),
        Provider::Hermes => format!("hermes --resume {}", db_id?),
        Provider::Copilot => format!("copilot --resume {}", parent()?),
        Provider::Pi => format!("pi --session '{}'", key.display()),
        Provider::Amp => format!("amp threads continue {stem}"),
        _ => return None,
    })
}

// ---------- Cursor ----------
//
// `{"role":"user"|"assistant","message":{"content":[parts]}}` and
// `{"type":"turn_ended","status":…}`. A user part wraps the prompt in
// `<user_query>` and says the time in `<timestamp>`; tool calls have no id
// and no result. Files before mid-2026 never write `turn_ended`.

fn cursor(out: &mut Vec<Found>) {
    let root = home().join(".cursor/projects");
    let Ok(rd) = std::fs::read_dir(&root) else {
        return;
    };
    for e in rd.flatten() {
        let name = e.file_name().to_string_lossy().to_string();
        if name.starts_with("var-folders") || name.chars().all(|c| c.is_ascii_digit()) {
            continue;
        }
        let mut found = Vec::new();
        files(
            &e.path().join("agent-transcripts"),
            3,
            &|p| p.extension().is_some_and(|x| x == "jsonl"),
            &mut found,
        );
        if found.is_empty() {
            continue;
        }
        let Some(cwd) = decoded(&name) else {
            continue;
        };
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

/// `decode_dashes`, remembered: it probes the disk, and scans run often.
fn decoded(name: &str) -> Option<PathBuf> {
    static SEEN: std::sync::LazyLock<std::sync::Mutex<HashMap<String, Option<PathBuf>>>> =
        std::sync::LazyLock::new(Default::default);
    let mut seen = SEEN.lock().ok()?;
    seen.entry(name.to_string())
        .or_insert_with(|| decode_dashes(name))
        .clone()
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
    if v["type"].as_str() == Some("turn_ended") {
        t.open_turn = Some(false);
        t.cursor_turns = true;
        return;
    }
    let parts = match &v["message"]["content"] {
        Value::Array(a) => a.clone(),
        Value::String(s) => vec![serde_json::json!({"type": "text", "text": s})],
        _ => Vec::new(),
    };
    match v["role"].as_str() {
        Some("user") => {
            for p in &parts {
                let Some(text) = p["text"].as_str() else {
                    continue;
                };
                let head = text.trim_start();
                if head.starts_with("<user_info>") || head.starts_with("<agent_transcripts>") {
                    continue;
                }
                let at = tag(text, "timestamp")
                    .and_then(|s| cursor_time(&s))
                    .unwrap_or(0);
                let prompt = tag(text, "user_query").unwrap_or_else(|| text.to_string());
                if !prompt.trim_start().starts_with('<') {
                    t.prompt(prompt, at);
                    // Only files that end their turns say when one is open.
                    if t.cursor_turns {
                        t.open_turn = Some(true);
                    }
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
                        match (text.find("<think>"), text.find("</think>")) {
                            (Some(a), Some(b)) if b > a => {
                                t.push(Kind::Thinking, text[a + 7..b].to_string(), 0);
                                t.push(Kind::Assistant, text[b + 8..].to_string(), 0);
                            }
                            _ => t.push(Kind::Assistant, text, 0),
                        }
                    }
                    Some("thinking" | "reasoning") => {
                        let text = p["thinking"]
                            .as_str()
                            .or(p["text"].as_str())
                            .unwrap_or_default();
                        t.push(Kind::Thinking, text.to_string(), 0);
                    }
                    Some("tool_use" | "tool-use" | "tool_call" | "tool-call") => {
                        let name = p["name"]
                            .as_str()
                            .or(p["tool"].as_str())
                            .or(p["toolName"].as_str())
                            .unwrap_or("tool");
                        let input = if p["input"].is_null() {
                            args_of(&p["args"])
                        } else {
                            p["input"].clone()
                        };
                        let id = p["id"]
                            .as_str()
                            .or(p["tool_use_id"].as_str())
                            .or(p["toolCallId"].as_str())
                            .map(str::to_string);
                        t.call(name, &input, id, 0);
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
}

/// `Sunday, Sep 20, 2026, 12:31 AM (UTC+3)`.
fn cursor_time(s: &str) -> Option<i64> {
    let (body, zone) = s.rsplit_once(" (UTC")?;
    let zone = zone.trim_end_matches(')');
    let (hours, minutes) = match zone.split_once(':') {
        Some((h, m)) => (h.parse::<i64>().ok()?, m.parse::<i64>().ok()?),
        None if zone.is_empty() => (0, 0),
        None => (zone.parse::<i64>().ok()?, 0),
    };
    let body = body.split_once(", ").map_or(body, |(_, rest)| rest);
    let naive = chrono::NaiveDateTime::parse_from_str(body, "%b %d, %Y, %I:%M %p").ok()?;
    Some(naive.and_utc().timestamp() - hours * 3600 - hours.signum() * minutes * 60)
}

// ---------- Gemini CLI ----------
//
// Line 1 is metadata; a message record `{id, timestamp, type, content,
// thoughts, toolCalls, tokens}` is re-appended with the same `id` each time
// it changes (last write wins); `{"$rewindTo": id}` drops it and all after.
// A tool call holds its own result. The legacy `.json` has `messages`.

fn gemini(out: &mut Vec<Found>) {
    let root = home().join(".gemini");
    let projects: HashMap<String, String> = read_json(&root.join("projects.json"))
        .and_then(|v| v["projects"].as_object().cloned())
        .map(|m| {
            m.into_iter()
                .filter_map(|(path, name)| Some((name.as_str()?.to_string(), path)))
                .collect()
        })
        .unwrap_or_default();
    for dir in subdirs(&root.join("tmp")) {
        let name = dir
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        let cwd = std::fs::read_to_string(dir.join(".project_root"))
            .ok()
            .map(|s| PathBuf::from(s.trim()))
            .or_else(|| projects.get(&name).map(PathBuf::from));
        let Some(cwd) = cwd else {
            continue;
        };
        let mut found = Vec::new();
        files(
            &dir.join("chats"),
            1,
            &|p| p.extension().is_some_and(|x| x == "jsonl" || x == "json"),
            &mut found,
        );
        for key in found {
            out.push(Found {
                provider: Provider::Gemini,
                modified: mtime(&key),
                key,
                cwd: cwd.clone(),
            });
        }
    }
}

fn gemini_parse(t: &mut Transcript, text: &str, path: &Path) {
    let mut order: Vec<String> = Vec::new();
    let mut messages: HashMap<String, Value> = HashMap::new();
    let add = |order: &mut Vec<String>, messages: &mut HashMap<String, Value>, m: Value| {
        let Some(id) = m["id"].as_str().map(str::to_string) else {
            return;
        };
        if !messages.contains_key(&id) {
            order.push(id.clone());
        }
        messages.insert(id, m);
    };
    if path.extension().is_some_and(|x| x == "json") {
        if let Ok(v) = serde_json::from_str::<Value>(text) {
            for m in v["messages"].as_array().cloned().unwrap_or_default() {
                add(&mut order, &mut messages, m);
            }
        }
    } else {
        for line in text.lines() {
            let Ok(v) = serde_json::from_str::<Value>(line) else {
                continue;
            };
            if let Some(to) = v["$rewindTo"].as_str() {
                match order.iter().position(|id| id == to) {
                    Some(i) => {
                        for id in order.drain(i..) {
                            messages.remove(&id);
                        }
                    }
                    None => {
                        order.clear();
                        messages.clear();
                    }
                }
            } else if v["id"].is_string() {
                add(&mut order, &mut messages, v);
            }
        }
    }
    let running = ["scheduled", "validating", "awaiting_approval", "executing"];
    let mut open = false;
    for id in &order {
        let m = &messages[id];
        let at = iso(&m["timestamp"]);
        match m["type"].as_str() {
            Some("user") => {
                t.prompt(text_of(&m["content"]), at);
                open = true;
            }
            Some("gemini") => {
                for th in m["thoughts"].as_array().into_iter().flatten() {
                    let s = [th["subject"].as_str(), th["description"].as_str()]
                        .into_iter()
                        .flatten()
                        .collect::<Vec<_>>()
                        .join(": ");
                    t.push(Kind::Thinking, s, iso(&th["timestamp"]).max(at));
                }
                t.push(Kind::Assistant, text_of(&m["content"]), at);
                let mut busy = false;
                for c in m["toolCalls"].as_array().into_iter().flatten() {
                    let name = c["name"].as_str().unwrap_or("tool");
                    let id = c["id"].as_str().map(str::to_string);
                    t.call(name, &c["args"], id.clone(), iso(&c["timestamp"]).max(at));
                    let out: Vec<String> = c["result"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .map(|r| {
                            let resp = &r["functionResponse"]["response"];
                            resp["output"]
                                .as_str()
                                .or(resp["error"].as_str())
                                .map_or_else(|| resp.to_string(), str::to_string)
                        })
                        .collect();
                    if let (Some(id), false) = (&id, out.is_empty()) {
                        t.attach_output(id, out.join("\n"));
                    }
                    // The file diff Gemini shows beats the arguments.
                    let d = &c["resultDisplay"];
                    if let (Some(diff), Some(e)) = (d["fileDiff"].as_str(), t.entries.last_mut()) {
                        let path = d["filePath"]
                            .as_str()
                            .or(d["fileName"].as_str())
                            .unwrap_or_default();
                        let mut edit = diff_edit(path, diff);
                        edit.created = d["isNewFile"].as_bool().unwrap_or(false);
                        e.edits = vec![edit];
                    }
                    busy |= c["status"].as_str().is_some_and(|s| running.contains(&s));
                }
                // Tokens are written when the reply ends.
                open = busy || m["tokens"].is_null();
            }
            _ => {}
        }
    }
    t.open_turn = Some(open);
}

// ---------- OpenCode ----------
//
// `session(id, directory, time_updated…)`, `message(id, session_id, data)`
// and `part(id, message_id, session_id, data)`, times in epoch ms. A `tool`
// part holds the call, its status and its result; an `edit` tool carries
// its unified diff in `state.metadata.diff`.

fn opencode(out: &mut Vec<Found>) {
    let base = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| home().join(".local/share"))
        .join("opencode");
    let mut dbs = Vec::new();
    files(
        &base,
        0,
        &|p| {
            p.file_name().is_some_and(|n| {
                n.to_string_lossy().starts_with("opencode") && n.to_string_lossy().ends_with(".db")
            })
        },
        &mut dbs,
    );
    for db in dbs {
        let Some(conn) = open_db(&db) else {
            continue;
        };
        let sql = "SELECT s.id, COALESCE(NULLIF(s.directory, ''), p.worktree, ''), \
                   MAX(COALESCE(m.time_updated, 0), s.time_updated) \
                   FROM session s LEFT JOIN project p ON p.id = s.project_id \
                   LEFT JOIN (SELECT session_id, MAX(time_updated) AS time_updated FROM message GROUP BY session_id) m \
                   ON m.session_id = s.id \
                   WHERE EXISTS (SELECT 1 FROM part WHERE part.session_id = s.id)";
        let Ok(mut stmt) = conn.prepare(sql) else {
            continue;
        };
        let rows = stmt.query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, i64>(2)?,
            ))
        });
        for (id, dir, updated) in rows.into_iter().flatten().flatten() {
            if dir.is_empty() || dir == "/" {
                continue;
            }
            out.push(Found {
                provider: Provider::OpenCode,
                key: PathBuf::from(format!("{}#{id}", db.display())),
                cwd: PathBuf::from(dir),
                modified: updated / 1000,
            });
        }
    }
}

fn opencode_parse(t: &mut Transcript, conn: &rusqlite::Connection, id: &str) {
    let Ok(mut stmt) = conn.prepare(
        "SELECT m.id, m.data, p.data FROM message m LEFT JOIN part p ON p.message_id = m.id \
         WHERE m.session_id = ?1 ORDER BY m.time_created, m.id, p.id",
    ) else {
        return;
    };
    let rows = stmt.query_map([id], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, Option<String>>(2)?,
        ))
    });
    let mut current = String::new();
    let mut user_text: Vec<String> = Vec::new();
    let mut user_at = 0;
    let mut open = false;
    let flush = |t: &mut Transcript, text: &mut Vec<String>, at: i64| {
        if !text.is_empty() {
            t.prompt(text.join("\n"), at);
            text.clear();
        }
    };
    for (mid, mdata, pdata) in rows.into_iter().flatten().flatten() {
        let m: Value = serde_json::from_str(&mdata).unwrap_or(Value::Null);
        let role = m["role"].as_str().unwrap_or_default();
        let at = ms(&m["time"]["created"]);
        if mid != current {
            flush(t, &mut user_text, user_at);
            current = mid;
            if role == "user" {
                user_at = at;
                open = true;
            } else if role == "assistant" {
                open = m["time"]["completed"].is_null()
                    && m["error"].is_null()
                    && m["finish"].is_null();
            }
        }
        let Some(pdata) = pdata else {
            continue;
        };
        let p: Value = serde_json::from_str(&pdata).unwrap_or(Value::Null);
        match (role, p["type"].as_str()) {
            ("user", Some("text")) if p["synthetic"].as_bool() != Some(true) => {
                user_text.push(p["text"].as_str().unwrap_or_default().to_string());
            }
            ("assistant", Some("text")) => t.push(
                Kind::Assistant,
                p["text"].as_str().unwrap_or_default().to_string(),
                at,
            ),
            ("assistant", Some("reasoning")) => t.push(
                Kind::Thinking,
                p["text"].as_str().unwrap_or_default().to_string(),
                at,
            ),
            ("assistant", Some("tool")) => {
                let state = &p["state"];
                let name = p["tool"].as_str().unwrap_or("tool");
                let input = args_of(&state["input"]);
                let call = p["callID"].as_str().map(str::to_string);
                let started = ms(&state["time"]["start"]).max(at);
                t.call(name, &input, call.clone(), started);
                let meta = &state["metadata"];
                if let Some(e) = t.entries.last_mut() {
                    if let Some(diff) = meta["diff"].as_str() {
                        let path = input["filePath"].as_str().unwrap_or_default();
                        e.edits = vec![diff_edit(path, diff)];
                    } else if let Some(files) = meta["files"].as_array() {
                        let list: Vec<Edit> = files
                            .iter()
                            .filter_map(|f| {
                                let mut e = diff_edit(
                                    f["filePath"].as_str()?,
                                    f["patch"].as_str().unwrap_or_default(),
                                );
                                e.created = f["type"].as_str() == Some("add");
                                e.deleted = f["type"].as_str() == Some("delete");
                                Some(e)
                            })
                            .collect();
                        if !list.is_empty() {
                            e.edits = list;
                        }
                    }
                }
                let out = state["output"]
                    .as_str()
                    .or(state["error"].as_str())
                    .map(str::to_string);
                if let (Some(call), Some(out)) = (call, out) {
                    t.attach_output(&call, out);
                }
                open |= matches!(state["status"].as_str(), Some("pending" | "running"));
            }
            _ => {}
        }
    }
    flush(t, &mut user_text, user_at);
    t.open_turn = Some(open);
}

// ---------- Kimi ----------
//
// Legacy `~/.kimi`: `{timestamp, message: {type, payload}}` with
// `TurnBegin`, `ContentPart`, `ToolCall`, `ToolResult`, `TurnEnd`; the
// session folder is the MD5 of its working folder. Newer `~/.kimi-code`:
// `{type, time}` records `turn.prompt`, `context.append_loop_event`
// (`content.part`, `tool.call`, `tool.result`) and `turn.ended`.

fn kimi(out: &mut Vec<Found>) {
    // Legacy: match each known folder's MD5 to a session bucket.
    let legacy = home().join(".kimi");
    let dirs: Vec<String> = read_json(&legacy.join("kimi.json"))
        .and_then(|v| v["work_dirs"].as_array().cloned())
        .unwrap_or_default()
        .iter()
        .filter_map(|w| w["path"].as_str().map(str::to_string))
        .collect();
    for dir in dirs {
        let bucket = format!("{:x}", md5::compute(dir.as_bytes()));
        for session in subdirs(&legacy.join("sessions").join(&bucket)) {
            let key = session.join("wire.jsonl");
            if key.is_file() {
                out.push(Found {
                    provider: Provider::Kimi,
                    modified: mtime(&key),
                    key,
                    cwd: PathBuf::from(&dir),
                });
            }
        }
    }
    // Kimi Code: each session names its folder in `state.json`.
    for ws in subdirs(&home().join(".kimi-code/sessions")) {
        for session in subdirs(&ws) {
            let state = read_json(&session.join("state.json")).unwrap_or(Value::Null);
            let cwd = ["cwd", "workDir"]
                .iter()
                .find_map(|k| state[*k].as_str())
                .or(state["custom"]["cwd"].as_str());
            let Some(cwd) = cwd else {
                continue;
            };
            let key = session.join("agents/main/wire.jsonl");
            if key.is_file() {
                out.push(Found {
                    provider: Provider::Kimi,
                    modified: mtime(&key),
                    key,
                    cwd: PathBuf::from(cwd),
                });
            }
        }
    }
}

fn kimi_line(t: &mut Transcript, v: &Value) {
    // Legacy wire protocol.
    if let Some(kind) = v["message"]["type"].as_str() {
        let p = &v["message"]["payload"];
        let at = v["timestamp"].as_f64().map_or(0, |f| f as i64);
        match kind {
            "TurnBegin" | "SteerInput" => {
                t.prompt(text_of(&p["user_input"]), at);
                t.open_turn = Some(true);
            }
            "TurnEnd" => t.open_turn = Some(false),
            "ContentPart" => kimi_part(t, p, at),
            "ToolCall" => {
                let name = p["function"]["name"].as_str().unwrap_or("tool");
                t.call(
                    name,
                    &args_of(&p["function"]["arguments"]),
                    p["id"].as_str().map(str::to_string),
                    at,
                );
            }
            "ToolResult" => {
                let rv = &p["return_value"];
                let Some(id) = p["tool_call_id"].as_str() else {
                    return;
                };
                t.attach_output(id, text_of(&rv["output"]));
                let diffs: Vec<Edit> = rv["display"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter(|d| d["type"].as_str() == Some("diff"))
                    .filter_map(|d| {
                        Some(Edit::between(
                            d["path"].as_str()?,
                            d["old_text"].as_str().unwrap_or_default(),
                            d["new_text"].as_str().unwrap_or_default(),
                        ))
                    })
                    .collect();
                if !diffs.is_empty()
                    && let Some(e) = t
                        .entries
                        .iter_mut()
                        .rev()
                        .find(|e| e.call.as_deref() == Some(id))
                {
                    e.edits = diffs;
                }
            }
            _ => {}
        }
        return;
    }
    // Kimi Code.
    let at = ms(&v["time"]);
    match v["type"].as_str() {
        Some("turn.prompt") => {
            t.prompt(text_of(&v["input"]), at);
            t.open_turn = Some(true);
        }
        Some("turn.ended" | "turn.cancel") => t.open_turn = Some(false),
        Some("context.append_message") => {
            let m = &v["message"];
            match m["role"].as_str() {
                // Migrated sessions keep the whole history here.
                Some("assistant") => {
                    t.push(Kind::Assistant, text_of(&m["content"]), at);
                    for c in m["toolCalls"].as_array().into_iter().flatten() {
                        let name = c["function"]["name"]
                            .as_str()
                            .or(c["name"].as_str())
                            .unwrap_or("tool");
                        let args = if c["function"]["arguments"].is_null() {
                            &c["arguments"]
                        } else {
                            &c["function"]["arguments"]
                        };
                        t.call(
                            name,
                            &args_of(args),
                            c["id"].as_str().map(str::to_string),
                            at,
                        );
                    }
                }
                Some("tool") => {
                    if let Some(id) = m["toolCallId"].as_str() {
                        t.attach_output(id, text_of(&m["content"]));
                    }
                }
                _ => {}
            }
        }
        Some("context.append_loop_event") => {
            let e = &v["event"];
            match e["type"].as_str() {
                Some("content.part") => kimi_part(t, &e["part"], at),
                Some("tool.call") => {
                    t.call(
                        e["name"].as_str().unwrap_or("tool"),
                        &e["args"],
                        e["toolCallId"].as_str().map(str::to_string),
                        at,
                    );
                }
                Some("tool.result") => {
                    if let Some(id) = e["toolCallId"].as_str() {
                        t.attach_output(id, text_of(&e["result"]["output"]));
                    }
                }
                _ => {}
            }
        }
        _ => {}
    }
}

/// A streamed text or thinking part; consecutive parts join.
fn kimi_part(t: &mut Transcript, p: &Value, at: i64) {
    let (kind, text) = match p["type"].as_str() {
        Some("text") => (Kind::Assistant, p["text"].as_str().unwrap_or_default()),
        Some("think") => (Kind::Thinking, p["think"].as_str().unwrap_or_default()),
        _ => return,
    };
    match t.entries.last_mut() {
        Some(last) if last.kind == kind && last.tool.is_none() => last.text.push_str(text),
        _ => {
            let before = t.entries.len();
            t.push(kind, text.to_string(), at);
            // Keep the trailing space: the next part joins to it.
            if t.entries.len() > before
                && let Some(last) = t.entries.last_mut()
            {
                last.text = text.trim_start().to_string();
            }
        }
    }
}

// ---------- Copilot CLI ----------
//
// `{type, data, id, timestamp}`: `user.message`, `assistant.message`
// (`content`, `reasoningText`, `toolRequests`), `tool.execution_start`,
// `tool.execution_complete` (paired by `toolCallId`), and
// `assistant.turn_start` / `assistant.turn_end`.

fn copilot(out: &mut Vec<Found>) {
    let root = std::env::var_os("COPILOT_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| home().join(".copilot"));
    for session in subdirs(&root.join("session-state")) {
        let key = session.join("events.jsonl");
        if !key.is_file() {
            continue;
        }
        let yaml = std::fs::read_to_string(session.join("workspace.yaml")).unwrap_or_default();
        let cwd = yaml
            .lines()
            .find_map(|l| {
                l.strip_prefix("cwd:")
                    .map(|v| v.trim().trim_matches('"').to_string())
            })
            .or_else(|| {
                first_line(&key)
                    .and_then(|v| v["data"]["context"]["cwd"].as_str().map(str::to_string))
            });
        if let Some(cwd) = cwd {
            out.push(Found {
                provider: Provider::Copilot,
                modified: mtime(&key),
                key,
                cwd: PathBuf::from(cwd),
            });
        }
    }
}

fn copilot_line(t: &mut Transcript, v: &Value) {
    let d = &v["data"];
    let at = iso(&v["timestamp"]);
    match v["type"].as_str() {
        Some("user.message") => {
            let injected = d["isAutopilotContinuation"].as_bool() == Some(true)
                || d["source"].as_str().is_some_and(|s| s.starts_with("skill"));
            if !injected {
                t.prompt(d["content"].as_str().unwrap_or_default().to_string(), at);
                t.open_turn = Some(true);
            }
        }
        Some("assistant.turn_start") => t.open_turn = Some(true),
        Some("assistant.turn_end" | "abort" | "session.shutdown") => t.open_turn = Some(false),
        Some("assistant.reasoning") => t.push(
            Kind::Thinking,
            d["content"].as_str().unwrap_or_default().to_string(),
            at,
        ),
        Some("assistant.message") => {
            t.push(
                Kind::Thinking,
                d["reasoningText"].as_str().unwrap_or_default().to_string(),
                at,
            );
            t.push(
                Kind::Assistant,
                d["content"].as_str().unwrap_or_default().to_string(),
                at,
            );
            for r in d["toolRequests"].as_array().into_iter().flatten() {
                t.call(
                    r["name"].as_str().unwrap_or("tool"),
                    &args_of(&r["arguments"]),
                    r["toolCallId"].as_str().map(str::to_string),
                    at,
                );
            }
        }
        Some("tool.execution_start") => {
            let id = d["toolCallId"].as_str().map(str::to_string);
            let known = id
                .as_deref()
                .is_some_and(|id| t.entries.iter().any(|e| e.call.as_deref() == Some(id)));
            if !known {
                t.call(
                    d["toolName"].as_str().unwrap_or("tool"),
                    &args_of(&d["arguments"]),
                    id,
                    at,
                );
            }
        }
        Some("tool.execution_complete") => {
            let Some(id) = d["toolCallId"].as_str() else {
                return;
            };
            let r = &d["result"];
            let out = r["detailedContent"]
                .as_str()
                .or(r["content"].as_str())
                .or(r.as_str())
                .or(d["error"]["message"].as_str())
                .unwrap_or_default();
            t.attach_output(id, out.to_string());
        }
        _ => {}
    }
}

// ---------- Junie ----------
//
// `{kind: "UserPromptEvent", prompt}` and `{kind: "SessionA2uxEvent",
// timestampMs, event: {agentEvent: {kind, stepId, …}}}`. A step is sent
// again as it changes (last write wins). File changes carry the whole file
// before and after.

fn junie(out: &mut Vec<Found>) {
    let root = home().join(".junie");
    let mut cwds: HashMap<String, PathBuf> = HashMap::new();
    for p in std::fs::read_dir(root.join("processes"))
        .into_iter()
        .flatten()
        .flatten()
    {
        if let Some(v) = read_json(&p.path())
            && let (Some(id), Some(path)) = (v["sessionId"].as_str(), v["projectPath"].as_str())
        {
            cwds.insert(id.to_string(), PathBuf::from(path));
        }
    }
    if let Ok(index) = std::fs::read_to_string(root.join("sessions/index.jsonl")) {
        for v in index
            .lines()
            .filter_map(|l| serde_json::from_str::<Value>(l).ok())
        {
            if let (Some(id), Some(path)) = (v["sessionId"].as_str(), v["projectDir"].as_str()) {
                cwds.insert(id.to_string(), PathBuf::from(path));
            }
        }
    }
    for session in subdirs(&root.join("sessions")) {
        let key = session.join("events.jsonl");
        let id = session
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        if let (true, Some(cwd)) = (key.is_file(), cwds.get(&id)) {
            out.push(Found {
                provider: Provider::Junie,
                modified: mtime(&key),
                key,
                cwd: cwd.clone(),
            });
        }
    }
}

fn junie_line(t: &mut Transcript, v: &Value) {
    match v["kind"].as_str() {
        Some("UserPromptEvent") => {
            let at = t.entries.last().map_or(0, |e| e.at);
            t.prompt(v["prompt"].as_str().unwrap_or_default().to_string(), at);
            t.open_turn = Some(true);
        }
        Some("SessionA2uxEvent") => {
            let at = ms(&v["timestampMs"]);
            let e = &v["event"]["agentEvent"];
            let step = format!("junie:{}", e["stepId"].as_str().unwrap_or_default());
            let s = |k: &str| e[k].as_str().unwrap_or_default().to_string();
            match e["kind"].as_str() {
                Some("ResultBlockUpdatedEvent") => {
                    t.upsert(Kind::Assistant, step, s("result"), None, at);
                }
                Some("MarkdownBlockUpdatedEvent" | "StreamingAgentMessageCompletedEvent") => {
                    t.upsert(Kind::Assistant, step, s("text"), None, at);
                }
                Some("AgentThoughtBlockUpdatedEvent") => {
                    t.upsert(Kind::Thinking, step, s("text"), None, at);
                }
                Some("TerminalBlockUpdatedEvent") => {
                    if let Some(i) = t.upsert(Kind::Tool, step, s("command"), Some("Terminal"), at)
                    {
                        let out = e["presentableOutput"]
                            .as_str()
                            .or(e["details"].as_str())
                            .unwrap_or_default();
                        t.entries[i].output = Some(cut(out, 4_000));
                    }
                }
                Some("ViewFilesBlockUpdatedEvent") => {
                    let files: Vec<&str> = e["files"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(|f| f.as_str().or(f["path"].as_str()))
                        .collect();
                    t.upsert(Kind::Tool, step, files.join(", "), Some("Read"), at);
                }
                Some("ToolBlockUpdatedEvent" | "McpBlockUpdatedEvent") => {
                    let name = e["toolName"]
                        .as_str()
                        .or(e["toolType"].as_str())
                        .unwrap_or("tool")
                        .to_string();
                    if let Some(i) = t.upsert(Kind::Tool, step, s("text"), Some(&name), at) {
                        t.entries[i].output = e["details"].as_str().map(|d| cut(d, 4_000));
                    }
                }
                Some("FileChangesBlockUpdatedEvent") => {
                    let edits: Vec<Edit> = e["changes"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .map(|c| {
                            let before = c["beforeContent"]["text"].as_str().unwrap_or_default();
                            let after = c["afterContent"]["text"].as_str().unwrap_or_default();
                            let path = c["afterRelativePath"]
                                .as_str()
                                .or(c["beforeRelativePath"].as_str())
                                .unwrap_or_default();
                            let mut edit = Edit::between(path, before, after);
                            edit.created = c["beforeRelativePath"].is_null();
                            edit.deleted = c["afterRelativePath"].is_null();
                            edit
                        })
                        .collect();
                    let label = edits
                        .iter()
                        .map(|e| e.path.clone())
                        .collect::<Vec<_>>()
                        .join(", ");
                    if let Some(i) = t.upsert(Kind::Tool, step, label, Some("FileChanges"), at) {
                        t.entries[i].edits = edits;
                    }
                }
                Some("ChatTurnStateEvent") => match e["phase"].as_str() {
                    Some("STARTED") => t.open_turn = Some(true),
                    Some("FINISHED") => t.open_turn = Some(false),
                    _ => {}
                },
                _ => {}
            }
        }
        _ => {}
    }
}

// ---------- Hermes ----------
//
// `sessions(id, cwd, model_config, last_activity_at…)` and OpenAI-style
// `messages(role, content, reasoning, tool_calls, tool_call_id, timestamp,
// finish_reason, active)`, times in epoch seconds. `tool_calls` is JSON
// text whose `function.arguments` is a JSON string.

fn hermes(out: &mut Vec<Found>) {
    let db = home().join(".hermes/state.db");
    let Some(conn) = open_db(&db) else {
        return;
    };
    // Newer schemas have `cwd` and `last_activity_at`; older ones don't.
    let queries = [
        "SELECT id, COALESCE(NULLIF(cwd, ''), json_extract(model_config, '$.cwd'), ''), \
         CAST(COALESCE(last_activity_at, ended_at, started_at, 0) AS REAL) FROM sessions \
         WHERE EXISTS (SELECT 1 FROM messages m WHERE m.session_id = sessions.id)",
        "SELECT id, COALESCE(json_extract(model_config, '$.cwd'), ''), CAST(COALESCE(started_at, 0) AS REAL) FROM sessions \
         WHERE EXISTS (SELECT 1 FROM messages m WHERE m.session_id = sessions.id)",
    ];
    for sql in queries {
        let Ok(mut stmt) = conn.prepare(sql) else {
            continue;
        };
        let rows = stmt.query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, f64>(2)?,
            ))
        });
        for (id, cwd, at) in rows.into_iter().flatten().flatten() {
            if cwd.is_empty() {
                continue;
            }
            out.push(Found {
                provider: Provider::Hermes,
                key: PathBuf::from(format!("{}#{id}", db.display())),
                cwd: expand_home(&cwd),
                modified: at as i64,
            });
        }
        break;
    }
}

type HermesRow = (
    String,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<f64>,
    Option<String>,
);

fn hermes_parse(t: &mut Transcript, conn: &rusqlite::Connection, id: &str) {
    let queries = [
        "SELECT role, content, tool_call_id, tool_calls, COALESCE(reasoning, reasoning_content), timestamp, finish_reason \
         FROM messages WHERE session_id = ?1 AND COALESCE(active, 1) = 1 ORDER BY timestamp, id",
        "SELECT role, content, tool_call_id, tool_calls, NULL, timestamp, NULL FROM messages WHERE session_id = ?1 ORDER BY timestamp, id",
    ];
    let mut last_role = String::new();
    let mut last_finish = String::new();
    for sql in queries {
        let Ok(mut stmt) = conn.prepare(sql) else {
            continue;
        };
        let rows = stmt.query_map([id], |r| -> rusqlite::Result<HermesRow> {
            Ok((
                r.get(0)?,
                r.get(1)?,
                r.get(2)?,
                r.get(3)?,
                r.get(4)?,
                r.get(5)?,
                r.get(6)?,
            ))
        });
        for (role, content, call_id, calls, reasoning, at, finish) in
            rows.into_iter().flatten().flatten()
        {
            let at = at.unwrap_or(0.) as i64;
            let content = content.unwrap_or_default();
            match role.as_str() {
                "user" => t.prompt(content, at),
                "assistant" => {
                    t.push(Kind::Thinking, reasoning.unwrap_or_default(), at);
                    t.push(Kind::Assistant, content, at);
                    let calls: Value = calls
                        .and_then(|c| serde_json::from_str(&c).ok())
                        .unwrap_or(Value::Null);
                    for c in calls.as_array().into_iter().flatten() {
                        let name = c["function"]["name"].as_str().unwrap_or("tool");
                        t.call(
                            name,
                            &args_of(&c["function"]["arguments"]),
                            c["id"].as_str().map(str::to_string),
                            at,
                        );
                    }
                }
                "tool" => {
                    if let Some(id) = call_id {
                        t.attach_output(&id, content);
                    }
                }
                _ => {}
            }
            last_role = role;
            last_finish = finish.unwrap_or_default();
        }
        break;
    }
    // A live lease says a turn runs; else the last row says it.
    let leased = conn
        .query_row(
            "SELECT COUNT(*) FROM session_turn_leases WHERE conversation_id = ?1 AND expires_at > unixepoch()",
            [id],
            |r| r.get::<_, i64>(0),
        )
        .is_ok_and(|n| n > 0);
    t.open_turn = Some(
        leased
            || last_role == "user"
            || last_role == "tool"
            || (last_role == "assistant" && last_finish == "tool_calls"),
    );
}

// ---------- Pi ----------
//
// Line 1 `{"type":"session","cwd"}`; then `{"type":"message","message":
// {role: user|assistant|toolResult|bashExecution, content, stopReason,
// timestamp (ms)}}`. Assistant blocks: `text`, `thinking`, `toolCall {id,
// name, arguments}`; a `toolResult` answers by `toolCallId`.

fn pi(out: &mut Vec<Found>) {
    let root = std::env::var_os("PI_CODING_AGENT_SESSION_DIR")
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("PI_CODING_AGENT_DIR").map(|d| PathBuf::from(d).join("sessions"))
        })
        .unwrap_or_else(|| home().join(".pi/agent/sessions"));
    let mut found = Vec::new();
    files(
        &root,
        4,
        &|p| p.extension().is_some_and(|x| x == "jsonl"),
        &mut found,
    );
    for key in found {
        let Some(header) = first_line(&key) else {
            continue;
        };
        if header["type"].as_str() != Some("session") {
            continue;
        }
        if let Some(cwd) = header["cwd"].as_str() {
            out.push(Found {
                provider: Provider::Pi,
                modified: mtime(&key),
                cwd: PathBuf::from(cwd),
                key,
            });
        }
    }
}

fn pi_line(t: &mut Transcript, v: &Value) {
    if v["type"].as_str() != Some("message") {
        return;
    }
    let m = &v["message"];
    let at = if m["timestamp"].is_null() {
        iso(&v["timestamp"])
    } else {
        ms(&m["timestamp"])
    };
    match m["role"].as_str() {
        Some("user") => {
            t.prompt(text_of(&m["content"]), at);
            t.open_turn = Some(true);
        }
        Some("assistant") => {
            for b in m["content"].as_array().into_iter().flatten() {
                match b["type"].as_str() {
                    Some("text") => t.push(
                        Kind::Assistant,
                        b["text"].as_str().unwrap_or_default().to_string(),
                        at,
                    ),
                    Some("thinking") => t.push(
                        Kind::Thinking,
                        b["thinking"].as_str().unwrap_or_default().to_string(),
                        at,
                    ),
                    Some("toolCall") => {
                        t.call(
                            b["name"].as_str().unwrap_or("tool"),
                            &args_of(&b["arguments"]),
                            b["id"].as_str().map(str::to_string),
                            at,
                        );
                    }
                    _ => {}
                }
            }
            t.open_turn = Some(m["stopReason"].as_str() == Some("toolUse"));
        }
        Some("toolResult") => {
            let Some(id) = m["toolCallId"].as_str() else {
                return;
            };
            t.attach_output(id, text_of(&m["content"]));
            // An edit's result holds its exact patch.
            if let Some(patch) = m["details"]["patch"].as_str()
                && let Some(e) = t
                    .entries
                    .iter_mut()
                    .rev()
                    .find(|e| e.call.as_deref() == Some(id))
            {
                let path = e.edits.first().map(|x| x.path.clone()).unwrap_or_default();
                e.edits = vec![diff_edit(&path, patch)];
            }
            t.open_turn = Some(true);
        }
        Some("bashExecution") => {
            t.push_tool(
                "bash",
                m["command"].as_str().unwrap_or_default().to_string(),
                None,
                at,
            );
            if let Some(e) = t.entries.last_mut() {
                e.output = m["output"].as_str().map(|o| cut(o, 4_000));
            }
        }
        _ => {}
    }
}

// ---------- Amp ----------
//
// One JSON document per thread: `messages[]` in the Anthropic shape; a
// user message with `meta.sentAt` is a prompt, one without carries tool
// results (`tool_result {toolUseID, run {status, result}}`). The folder is
// `env.initial.trees[0].uri`.

fn amp(out: &mut Vec<Found>) {
    let root = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| home().join(".local/share"))
        .join("amp/threads");
    let mut found = Vec::new();
    files(
        &root,
        0,
        &|p| p.extension().is_some_and(|x| x == "json"),
        &mut found,
    );
    for key in found {
        let Some(v) = read_json(&key) else {
            continue;
        };
        let Some(uri) = v["env"]["initial"]["trees"][0]["uri"].as_str() else {
            continue;
        };
        let cwd = percent_decode(uri.trim_start_matches("file://"));
        out.push(Found {
            provider: Provider::Amp,
            modified: mtime(&key),
            key,
            cwd: PathBuf::from(cwd),
        });
    }
}

fn amp_parse(t: &mut Transcript, v: &Value) {
    let mut open = false;
    let mut at = ms(&v["created"]);
    for m in v["messages"].as_array().into_iter().flatten() {
        let blocks = m["content"].as_array().cloned().unwrap_or_default();
        match m["role"].as_str() {
            Some("user") => {
                if !m["meta"]["sentAt"].is_null() {
                    at = ms(&m["meta"]["sentAt"]);
                    t.prompt(text_of(&m["content"]), at);
                    open = true;
                    continue;
                }
                for b in blocks
                    .iter()
                    .filter(|b| b["type"].as_str() == Some("tool_result"))
                {
                    let Some(id) = b["toolUseID"].as_str().or(b["tool_use_id"].as_str()) else {
                        continue;
                    };
                    let run = &b["run"];
                    let result = &run["result"];
                    let out = result
                        .as_str()
                        .or(result["output"].as_str())
                        .or(result["content"].as_str())
                        .or(result["diff"].as_str())
                        .or(run["error"]["message"].as_str())
                        .map_or_else(|| flatten(&b["content"]), str::to_string);
                    t.attach_output(id, out);
                    if let Some(diff) = result["diff"].as_str()
                        && let Some(e) = t
                            .entries
                            .iter_mut()
                            .rev()
                            .find(|e| e.call.as_deref() == Some(id))
                    {
                        let path = e.edits.first().map(|x| x.path.clone()).unwrap_or_default();
                        e.edits = vec![diff_edit(&path, diff)];
                    }
                }
            }
            Some("assistant") => {
                at = iso(&m["usage"]["timestamp"]).max(at);
                for b in &blocks {
                    match b["type"].as_str() {
                        Some("text") => t.push(
                            Kind::Assistant,
                            b["text"].as_str().unwrap_or_default().to_string(),
                            at,
                        ),
                        Some("thinking") => t.push(
                            Kind::Thinking,
                            b["thinking"].as_str().unwrap_or_default().to_string(),
                            at,
                        ),
                        Some("tool_use") => t.call(
                            b["name"].as_str().unwrap_or("tool"),
                            &b["input"],
                            b["id"].as_str().map(str::to_string),
                            at,
                        ),
                        _ => {}
                    }
                }
                let state = &m["state"];
                open = state["type"].as_str() == Some("streaming")
                    || (state["type"].as_str() == Some("complete")
                        && state["stopReason"].as_str() == Some("tool_use"));
            }
            _ => {}
        }
    }
    t.open_turn = Some(open);
}

// ---------- Grok CLI ----------
//
// `chat_history.jsonl`: `user` (prompt in `<user_query>`, `prompt_index`),
// `reasoning` (`summary[].text`), `assistant` (`content`, `tool_calls`
// with JSON-string `arguments`) and `tool_result` (`tool_call_id`). It has
// no times; the session is live while `updates.jsonl` grows.

fn grok(out: &mut Vec<Found>) {
    let root = std::env::var_os("GROK_HOME")
        .map(|h| PathBuf::from(h).join("sessions"))
        .unwrap_or_else(|| home().join(".grok/sessions"));
    for folder in subdirs(&root) {
        let name = folder
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        for session in subdirs(&folder) {
            let key = session.join("chat_history.jsonl");
            if !key.is_file() {
                continue;
            }
            let cwd = read_json(&session.join("summary.json"))
                .and_then(|v| v["info"]["cwd"].as_str().map(str::to_string))
                .unwrap_or_else(|| percent_decode(&name));
            let modified = mtime(&key).max(mtime(&session.join("updates.jsonl")));
            out.push(Found {
                provider: Provider::Grok,
                modified,
                key,
                cwd: PathBuf::from(cwd),
            });
        }
    }
}

fn grok_line(t: &mut Transcript, v: &Value) {
    match v["type"].as_str() {
        Some("user") => {
            if v["prompt_index"].is_null() || !v["synthetic_reason"].is_null() {
                return;
            }
            let text = text_of(&v["content"]);
            t.prompt(tag(&text, "user_query").unwrap_or(text), 0);
        }
        Some("reasoning") => {
            let text: Vec<&str> = v["summary"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|s| s["text"].as_str())
                .collect();
            t.push(Kind::Thinking, text.join("\n"), 0);
        }
        Some("assistant") => {
            t.push(Kind::Assistant, text_of(&v["content"]), 0);
            for c in v["tool_calls"].as_array().into_iter().flatten() {
                t.call(
                    c["name"].as_str().unwrap_or("tool"),
                    &args_of(&c["arguments"]),
                    c["id"].as_str().map(str::to_string),
                    0,
                );
            }
        }
        Some("tool_result") => {
            if let Some(id) = v["tool_call_id"].as_str() {
                t.attach_output(id, text_of(&v["content"]));
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn lines(provider: Provider, rows: &[Value]) -> Transcript {
        let mut t = Transcript::default();
        for r in rows {
            line(&mut t, provider, r);
        }
        t
    }

    #[test]
    fn reads_cursor_times_prompts_and_edits() {
        assert_eq!(
            cursor_time("Sunday, Sep 20, 2026, 12:31 AM (UTC+3)"),
            Some(1_789_853_460)
        );
        let t = lines(
            Provider::Cursor,
            &[
                json!({"role":"user","message":{"content":[{"type":"text","text":"<timestamp>Sunday, Sep 20, 2026, 12:31 AM (UTC+3)</timestamp>\n<user_query>\nImplement WS-5\n</user_query>"}]}}),
                json!({"role":"assistant","message":{"content":[{"type":"text","text":"Done.\n\n[REDACTED]"},{"type":"tool_use","name":"StrReplace","input":{"path":"/r/a.rs","old_string":"a\n","new_string":"b\n"}}]}}),
                json!({"type":"turn_ended","status":"success"}),
            ],
        );
        assert_eq!(t.entries[0].text, "Implement WS-5");
        assert_eq!(t.entries[1].text, "Done.");
        assert_eq!(t.entries[2].edits.len(), 1);
        assert_eq!(t.open_turn, Some(false));
    }

    #[test]
    fn reads_gemini_rewrites_and_rewinds() {
        let mut t = Transcript::default();
        let text = [
            r#"{"sessionId":"s1","kind":"main"}"#,
            r#"{"id":"u1","timestamp":"2026-04-29T21:39:33Z","type":"user","content":[{"text":"Fix WS-9"}]}"#,
            r#"{"id":"g1","timestamp":"2026-04-29T21:39:35Z","type":"gemini","content":"","toolCalls":[{"id":"c1","name":"replace","args":{"file_path":"/r/a.rs","old_string":"a","new_string":"b"},"status":"executing"}]}"#,
            r#"{"id":"g1","timestamp":"2026-04-29T21:39:35Z","type":"gemini","content":"Fixed.","tokens":{"total":5},"toolCalls":[{"id":"c1","name":"replace","args":{"file_path":"/r/a.rs","old_string":"a","new_string":"b"},"status":"success","result":[{"functionResponse":{"id":"c1","response":{"output":"ok"}}}]}]}"#,
            r#"{"id":"u2","timestamp":"2026-04-29T21:40:00Z","type":"user","content":"undo me"}"#,
            r#"{"$rewindTo":"u2"}"#,
        ]
        .join("\n");
        gemini_parse(&mut t, &text, Path::new("/x/session-1.jsonl"));
        let kinds: Vec<Kind> = t.entries.iter().map(|e| e.kind).collect();
        assert_eq!(kinds, vec![Kind::User, Kind::Assistant, Kind::Tool]);
        assert_eq!(t.entries[2].output.as_deref(), Some("ok"));
        assert_eq!(t.entries[2].edits.len(), 1);
        assert_eq!(t.open_turn, Some(false));
    }

    #[test]
    fn reads_kimi_both_formats() {
        let old = lines(
            Provider::Kimi,
            &[
                json!({"timestamp":1.0,"message":{"type":"TurnBegin","payload":{"user_input":[{"type":"text","text":"Do WS-1"}]}}}),
                json!({"timestamp":2.0,"message":{"type":"ContentPart","payload":{"type":"text","text":"On "}}}),
                json!({"timestamp":2.1,"message":{"type":"ContentPart","payload":{"type":"text","text":"it."}}}),
                json!({"timestamp":3.0,"message":{"type":"ToolCall","payload":{"id":"t1","function":{"name":"Shell","arguments":"{\"command\":\"ls\"}"}}}}),
                json!({"timestamp":4.0,"message":{"type":"ToolResult","payload":{"tool_call_id":"t1","return_value":{"output":"a.rs"}}}}),
            ],
        );
        assert_eq!(old.entries[1].text, "On it.");
        assert_eq!(old.entries[2].output.as_deref(), Some("a.rs"));
        assert_eq!(old.open_turn, Some(true));
        let new = lines(
            Provider::Kimi,
            &[
                json!({"type":"turn.prompt","input":[{"type":"text","text":"Do WS-2"}],"time":1000}),
                json!({"type":"context.append_loop_event","event":{"type":"tool.call","toolCallId":"c","name":"Edit","args":{"path":"/r/a","old_string":"x","new_string":"y"}},"time":2000}),
                json!({"type":"turn.ended","reason":"completed"}),
            ],
        );
        assert_eq!(new.entries[1].edits.len(), 1);
        assert_eq!(new.open_turn, Some(false));
    }

    #[test]
    fn reads_copilot_pi_grok_and_junie() {
        let c = lines(
            Provider::Copilot,
            &[
                json!({"type":"user.message","data":{"content":"WS-3 please"},"timestamp":"2026-09-03T07:48:33Z"}),
                json!({"type":"tool.execution_start","data":{"toolCallId":"e","toolName":"edit","arguments":{"path":"/r/a","old_str":"x","new_str":"y"}}}),
                json!({"type":"tool.execution_complete","data":{"toolCallId":"e","success":true,"result":{"content":"ok"}}}),
                json!({"type":"assistant.turn_end","data":{"turnId":"0"}}),
            ],
        );
        assert_eq!(c.entries[1].output.as_deref(), Some("ok"));
        assert_eq!(c.entries[1].edits.len(), 1);
        assert_eq!(c.open_turn, Some(false));
        let p = lines(
            Provider::Pi,
            &[
                json!({"type":"session","cwd":"/r"}),
                json!({"type":"message","message":{"role":"user","content":"WS-4","timestamp":1000}}),
                json!({"type":"message","message":{"role":"assistant","content":[{"type":"toolCall","id":"k","name":"edit","arguments":{"path":"a","edits":[{"oldText":"x","newText":"y"}]}}],"stopReason":"toolUse","timestamp":2000}}),
            ],
        );
        assert_eq!(p.entries[1].edits.len(), 1);
        assert_eq!(p.open_turn, Some(true));
        let g = lines(
            Provider::Grok,
            &[
                json!({"type":"user","content":[{"type":"text","text":"<user_query>\nWS-6\n</user_query>"}],"prompt_index":0}),
                json!({"type":"assistant","content":"","tool_calls":[{"id":"z","name":"run_terminal_command","arguments":"{\"command\":\"ls\"}"}]}),
                json!({"type":"tool_result","tool_call_id":"z","content":"a"}),
            ],
        );
        assert_eq!(g.entries[0].text, "WS-6");
        assert_eq!(g.entries[1].output.as_deref(), Some("a"));
        let j = lines(
            Provider::Junie,
            &[
                json!({"kind":"UserPromptEvent","prompt":"WS-7"}),
                json!({"kind":"SessionA2uxEvent","timestampMs":5000,"event":{"agentEvent":{"kind":"MarkdownBlockUpdatedEvent","stepId":"s1","text":"Wor"}}}),
                json!({"kind":"SessionA2uxEvent","timestampMs":6000,"event":{"agentEvent":{"kind":"MarkdownBlockUpdatedEvent","stepId":"s1","text":"Working"}}}),
                json!({"kind":"SessionA2uxEvent","timestampMs":7000,"event":{"agentEvent":{"kind":"FileChangesBlockUpdatedEvent","stepId":"s2","changes":[{"afterRelativePath":"a.rs","afterContent":{"text":"x\n"}}]}}}),
            ],
        );
        assert_eq!(j.entries.len(), 3);
        assert_eq!(j.entries[1].text, "Working");
        assert!(j.entries[2].edits[0].created);
    }

    #[test]
    fn reads_amp_threads() {
        let mut t = Transcript::default();
        amp_parse(
            &mut t,
            &json!({"created":1000,"messages":[
                {"role":"user","meta":{"sentAt":2000},"content":[{"type":"text","text":"WS-8"}]},
                {"role":"assistant","content":[{"type":"tool_use","id":"u","name":"edit_file","input":{"path":"/r/a","old_str":"x","new_str":"y"},"complete":true}],"state":{"type":"complete","stopReason":"tool_use"}},
                {"role":"user","content":[{"type":"tool_result","toolUseID":"u","run":{"status":"done","result":{"diff":"--- a\n+++ a\n-x\n+y\n"}}}]}
            ]}),
        );
        assert_eq!(t.entries[1].edits[0].added, 1);
        assert_eq!(t.open_turn, Some(true));
    }

    #[test]
    fn decodes_folders() {
        let home = home();
        let name = home.display().to_string().replace('/', "-");
        assert_eq!(decode_dashes(&name), Some(home));
        assert_eq!(percent_decode("%2FUsers%2Fa%20b"), "/Users/a b");
    }
}
