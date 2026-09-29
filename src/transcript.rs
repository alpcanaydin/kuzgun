//! A readable conversation from an agent's own session file: what the
//! person asked, what the agent said, its thinking and its tool calls with
//! their output. Read-only. Each read takes only the bytes added since the
//! last one, so a running agent streams in.
//!
//! - Claude Code: `~/.claude/projects/<repo>/<session>.jsonl`, one entry
//!   per line (`user`, `assistant` with content blocks).
//! - Codex: `~/.codex/sessions/YYYY/MM/DD/rollout-*.jsonl`, conversation
//!   in `response_item` lines (`message`, `reasoning`, tool calls and their
//!   outputs).

use std::path::Path;

use serde_json::Value;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Provider {
    Claude,
    Codex,
    Cursor,
    Gemini,
    OpenCode,
    Kimi,
    Copilot,
    Junie,
    Hermes,
    Pi,
    Amp,
    Grok,
}

/// How a harness stores one session.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Storage {
    /// One JSON value per line, appended as the session goes.
    Jsonl,
    /// One JSON document, rewritten as the session goes.
    Document,
    /// Rows in a SQLite database; the session key is `<db>#<session id>`.
    Sqlite,
}

impl Provider {
    pub const ALL: [Provider; 12] = [
        Provider::Claude,
        Provider::Codex,
        Provider::Cursor,
        Provider::Gemini,
        Provider::OpenCode,
        Provider::Kimi,
        Provider::Copilot,
        Provider::Junie,
        Provider::Hermes,
        Provider::Pi,
        Provider::Amp,
        Provider::Grok,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Provider::Claude => "Claude Code",
            Provider::Codex => "Codex",
            Provider::Cursor => "Cursor",
            Provider::Gemini => "Gemini CLI",
            Provider::OpenCode => "OpenCode",
            Provider::Kimi => "Kimi Code",
            Provider::Copilot => "Copilot CLI",
            Provider::Junie => "Junie",
            Provider::Hermes => "Hermes",
            Provider::Pi => "Pi",
            Provider::Amp => "Amp",
            Provider::Grok => "Grok CLI",
        }
    }

    pub fn storage(self) -> Storage {
        match self {
            Provider::Gemini | Provider::Amp => Storage::Document,
            Provider::OpenCode | Provider::Hermes => Storage::Sqlite,
            _ => Storage::Jsonl,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    User,
    Assistant,
    Thinking,
    Tool,
}

#[derive(Clone, Debug)]
pub struct Entry {
    pub kind: Kind,
    /// The message, or for a tool call a one-line summary of its input.
    pub text: String,
    /// Tool name.
    pub tool: Option<String>,
    /// Tool result, cut to a readable size.
    pub output: Option<String>,
    /// Tool call id, to pair the result with its call.
    pub call: Option<String>,
    /// Unix seconds.
    pub at: i64,
    /// File changes this tool call made, when it edits files.
    pub edits: Vec<Edit>,
}

/// One file change a tool call made, as a unified diff.
#[derive(Clone, Debug)]
pub struct Edit {
    /// The path as the agent wrote it (often absolute).
    pub path: String,
    pub diff: String,
    pub added: usize,
    pub removed: usize,
    /// The call made the file.
    pub created: bool,
    /// The call removed the file.
    pub deleted: bool,
}

impl Edit {
    /// A diff between two texts of one file.
    pub fn between(path: &str, old: &str, new: &str) -> Edit {
        let diff = similar::TextDiff::from_lines(old, new);
        let (mut added, mut removed) = (0, 0);
        for c in diff.iter_all_changes() {
            match c.tag() {
                similar::ChangeTag::Insert => added += 1,
                similar::ChangeTag::Delete => removed += 1,
                similar::ChangeTag::Equal => {}
            }
        }
        let text = diff
            .unified_diff()
            .context_radius(3)
            .header(path, path)
            .to_string();
        Edit {
            path: path.to_string(),
            diff: text,
            added,
            removed,
            created: false,
            deleted: false,
        }
    }
}

/// The edits of a Claude Code tool call.
pub(crate) fn claude_edits(name: &str, input: &Value) -> Vec<Edit> {
    let path = input["file_path"].as_str().unwrap_or_default();
    if path.is_empty() {
        return Vec::new();
    }
    match name {
        "Edit" => vec![Edit::between(
            path,
            input["old_string"].as_str().unwrap_or_default(),
            input["new_string"].as_str().unwrap_or_default(),
        )],
        "MultiEdit" => input["edits"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|e| {
                Edit::between(
                    path,
                    e["old_string"].as_str().unwrap_or_default(),
                    e["new_string"].as_str().unwrap_or_default(),
                )
            })
            .collect(),
        "Write" => {
            let mut e = Edit::between(path, "", input["content"].as_str().unwrap_or_default());
            e.created = true;
            vec![e]
        }
        _ => Vec::new(),
    }
}

/// The edits of a Codex `apply_patch` text:
/// `*** Update File: path` sections with `+` and `-` lines.
pub(crate) fn patch_edits(patch: &str) -> Vec<Edit> {
    let mut out: Vec<Edit> = Vec::new();
    for line in patch.lines() {
        let head = [
            ("*** Add File: ", 'A'),
            ("*** Update File: ", 'M'),
            ("*** Delete File: ", 'D'),
        ]
        .iter()
        .find_map(|(p, k)| {
            line.strip_prefix(p)
                .map(|rest| (rest.trim().to_string(), *k))
        });
        if let Some((path, kind)) = head {
            out.push(Edit {
                diff: format!("--- {path}\n+++ {path}\n"),
                path,
                added: 0,
                removed: 0,
                created: kind == 'A',
                deleted: kind == 'D',
            });
            continue;
        }
        if line.starts_with("*** ") {
            continue;
        }
        if let Some(e) = out.last_mut() {
            if line.starts_with('+') {
                e.added += 1;
            } else if line.starts_with('-') {
                e.removed += 1;
            }
            e.diff.push_str(line);
            e.diff.push('\n');
        }
    }
    out
}

/// A conversation read so far.
#[derive(Default)]
pub struct Transcript {
    offset: u64,
    pub entries: Vec<Entry>,
    /// Unix time of the last prompt: the current turn started then.
    pub turn_start: i64,
    /// Output tokens the agent wrote since the last prompt.
    pub turn_tokens: u64,
    last_message: Option<String>,
    /// Shell commands the agent left running in the background.
    pub background: Vec<Background>,
    /// Whether the last turn is still open, for harnesses that say so.
    pub open_turn: Option<bool>,
    /// The session's size and change time at the last read, for sources
    /// that are read whole (a JSON document, a database).
    pub(crate) stamp: (u64, i64),
}

/// A shell command the agent started in the background and listens to.
#[derive(Clone, Debug)]
pub struct Background {
    pub id: String,
    /// What the command does, from its description.
    pub label: String,
    /// The file Claude Code writes its output to.
    pub output: std::path::PathBuf,
    pub started: i64,
    pub done: bool,
}

const MAX_TEXT: usize = 12_000;
const MAX_OUTPUT: usize = 4_000;

impl Transcript {
    /// Reads the lines added since the last call. Returns whether new
    /// entries came in.
    pub fn update(&mut self, path: &Path, provider: Provider) -> bool {
        match provider.storage() {
            Storage::Jsonl => self.update_lines(path, provider),
            Storage::Document => crate::harness::update_document(self, path, provider),
            Storage::Sqlite => crate::harness::update_sqlite(self, path, provider),
        }
    }

    fn update_lines(&mut self, path: &Path, provider: Provider) -> bool {
        use std::io::{Read, Seek, SeekFrom};
        let Ok(len) = std::fs::metadata(path).map(|m| m.len()) else {
            return false;
        };
        if len < self.offset {
            *self = Transcript::default();
        }
        if len == self.offset {
            return false;
        }
        let Ok(mut f) = std::fs::File::open(path) else {
            return false;
        };
        if f.seek(SeekFrom::Start(self.offset)).is_err() {
            return false;
        }
        let mut buf = Vec::with_capacity((len - self.offset) as usize);
        if f.take(len - self.offset).read_to_end(&mut buf).is_err() {
            return false;
        }
        // Keep a line that is still being written for the next read.
        let end = buf.iter().rposition(|&b| b == b'\n').map_or(0, |i| i + 1);
        let before = self.entries.len();
        for line in String::from_utf8_lossy(&buf[..end]).lines() {
            let Ok(v) = serde_json::from_str::<Value>(line) else {
                continue;
            };
            match provider {
                Provider::Claude => self.claude_line(&v),
                Provider::Codex => self.codex_line(&v),
                other => crate::harness::line(self, other, &v),
            }
        }
        self.offset += end as u64;
        let _ = before;
        end > 0
    }

    pub(crate) fn push(&mut self, kind: Kind, text: String, at: i64) {
        let text = text.trim();
        if !text.is_empty() {
            self.entries.push(Entry {
                kind,
                text: cut(text, MAX_TEXT),
                tool: None,
                output: None,
                call: None,
                at,
                edits: Vec::new(),
            });
        }
    }

    pub(crate) fn push_tool(&mut self, name: &str, summary: String, call: Option<String>, at: i64) {
        self.entries.push(Entry {
            kind: Kind::Tool,
            text: summary,
            tool: Some(name.to_string()),
            output: None,
            call,
            at,
            edits: Vec::new(),
        });
    }

    pub(crate) fn attach_output(&mut self, call: &str, output: String) {
        if let Some(e) = self
            .entries
            .iter_mut()
            .rev()
            .find(|e| e.call.as_deref() == Some(call))
        {
            e.output = Some(cut(output.trim(), MAX_OUTPUT));
        }
    }

    fn claude_line(&mut self, v: &Value) {
        if v["isMeta"].as_bool() == Some(true) {
            return;
        }
        let at = time_of(v);
        let content = &v["message"]["content"];
        match v["type"].as_str() {
            Some("user") => match content {
                Value::String(s) => self.claude_prompt(s, at),
                Value::Array(parts) => {
                    for p in parts {
                        match p["type"].as_str() {
                            Some("text") => {
                                self.claude_prompt(p["text"].as_str().unwrap_or_default(), at)
                            }
                            Some("tool_result") => {
                                if let Some(id) = p["tool_use_id"].as_str() {
                                    let out = flatten(&p["content"]);
                                    self.note_background(id, &out, at);
                                    self.attach_output(id, out);
                                }
                            }
                            _ => {}
                        }
                    }
                }
                _ => {}
            },
            Some("assistant") => {
                // One message streams as several lines that share its id.
                let id = v["message"]["id"].as_str().map(str::to_string);
                if id.is_some() && id != self.last_message {
                    self.turn_tokens +=
                        v["message"]["usage"]["output_tokens"].as_u64().unwrap_or(0);
                    self.last_message = id;
                }
                for p in content.as_array().into_iter().flatten() {
                    match p["type"].as_str() {
                        Some("text") => self.push(
                            Kind::Assistant,
                            p["text"].as_str().unwrap_or_default().to_string(),
                            at,
                        ),
                        Some("thinking") => self.push(
                            Kind::Thinking,
                            p["thinking"].as_str().unwrap_or_default().to_string(),
                            at,
                        ),
                        Some("tool_use") => {
                            let name = p["name"].as_str().unwrap_or("tool");
                            if matches!(name, "KillShell" | "KillBash" | "TaskStop") {
                                let id = p["input"]["shell_id"]
                                    .as_str()
                                    .or(p["input"]["task_id"].as_str())
                                    .unwrap_or_default();
                                for b in self.background.iter_mut().filter(|b| b.id == id) {
                                    b.done = true;
                                }
                            }
                            self.push_tool(
                                name,
                                summarize(&p["input"]),
                                p["id"].as_str().map(str::to_string),
                                at,
                            );
                            if let Some(e) = self.entries.last_mut() {
                                e.edits = claude_edits(name, &p["input"]);
                            }
                        }
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }

    /// A tool result that says the command went to the background.
    fn note_background(&mut self, call: &str, out: &str, at: i64) {
        let Some(c) = BACKGROUND.captures(out) else {
            return;
        };
        let label = self
            .entries
            .iter()
            .rev()
            .find(|e| e.call.as_deref() == Some(call))
            .map(|e| e.text.clone())
            .unwrap_or_else(|| "Background command".into());
        self.background.push(Background {
            id: c[1].to_string(),
            label,
            output: std::path::PathBuf::from(c[2].trim_end_matches('.')),
            started: at,
            done: false,
        });
    }

    /// A user line of Claude Code: a prompt, or a note the harness adds in
    /// the person's name (a background task finished, a subagent reported).
    fn claude_prompt(&mut self, text: &str, at: i64) {
        let text = text.trim();
        if text.contains("<task-notification>") {
            if let Some(id) = tag(text, "task-id") {
                for b in self.background.iter_mut().filter(|b| b.id == id) {
                    b.done = true;
                }
            }
            let summary = tag(text, "summary")
                .or_else(|| tag(text, "status"))
                .unwrap_or_else(|| "finished".into());
            self.push_tool("notice", format!("Background task: {summary}"), None, at);
            if let Some(e) = self.entries.last_mut() {
                e.output = Some(cut(&strip_tags(text), MAX_OUTPUT));
            }
            return;
        }
        if text.starts_with("Another Claude session sent a message") {
            self.push_tool("notice", "A subagent reported back".into(), None, at);
            if let Some(e) = self.entries.last_mut() {
                e.output = Some(cut(&strip_tags(text), MAX_OUTPUT));
            }
            return;
        }
        if let Some(t) = clean_prompt(text) {
            // Harness commands are not prompts to the agent.
            if t == "/clear" {
                return;
            }
            if t.starts_with("/compact") {
                self.push_tool("notice", "Compacted the conversation".into(), None, at);
                return;
            }
            self.turn_start = at;
            self.turn_tokens = 0;
            self.last_message = None;
            self.push(Kind::User, t, at);
        }
    }

    fn codex_line(&mut self, v: &Value) {
        let at = time_of(v);
        if v["type"].as_str() == Some("event_msg")
            && v["payload"]["type"].as_str() == Some("token_count")
        {
            self.turn_tokens += v["payload"]["info"]["last_token_usage"]["output_tokens"]
                .as_u64()
                .unwrap_or(0);
            return;
        }
        if v["type"].as_str() != Some("response_item") {
            return;
        }
        let p = &v["payload"];
        match p["type"].as_str() {
            Some("message") => {
                let text = p["content"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|c| c["text"].as_str())
                    .collect::<Vec<_>>()
                    .join("\n");
                match p["role"].as_str() {
                    // Context the app injects (environment, instructions,
                    // plugins) comes as tagged user messages.
                    Some("user") if !text.trim_start().starts_with('<') => {
                        self.turn_start = at;
                        self.turn_tokens = 0;
                        self.push(Kind::User, text, at);
                    }
                    Some("assistant") => self.push(Kind::Assistant, text, at),
                    _ => {}
                }
            }
            Some("reasoning") => {
                let text = p["summary"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|s| s["text"].as_str())
                    .collect::<Vec<_>>()
                    .join("\n");
                self.push(Kind::Thinking, text, at);
            }
            Some("function_call") => {
                let args = p["arguments"]
                    .as_str()
                    .and_then(|a| serde_json::from_str::<Value>(a).ok())
                    .unwrap_or(Value::Null);
                let name = p["name"].as_str().unwrap_or("tool");
                self.push_tool(
                    name,
                    summarize(&args),
                    p["call_id"].as_str().map(str::to_string),
                    at,
                );
                // `shell` can run `apply_patch <patch>`.
                let cmd: Vec<&str> = args["command"]
                    .as_array()
                    .map(|a| a.iter().filter_map(Value::as_str).collect())
                    .unwrap_or_default();
                if cmd.first() == Some(&"apply_patch")
                    && let (Some(e), Some(patch)) = (self.entries.last_mut(), cmd.get(1))
                {
                    e.edits = patch_edits(patch);
                }
            }
            Some("custom_tool_call") => {
                let name = p["name"].as_str().unwrap_or("tool");
                let input = p["input"].as_str().unwrap_or_default();
                self.push_tool(
                    name,
                    one_line(input),
                    p["call_id"].as_str().map(str::to_string),
                    at,
                );
                if name == "apply_patch"
                    && let Some(e) = self.entries.last_mut()
                {
                    e.edits = patch_edits(input);
                    if let Some(first) = e.edits.first() {
                        e.text = first.path.clone();
                    }
                }
            }
            Some("local_shell_call") => {
                let cmd = p["action"]["command"]
                    .as_array()
                    .map(|a| {
                        a.iter()
                            .filter_map(Value::as_str)
                            .collect::<Vec<_>>()
                            .join(" ")
                    })
                    .unwrap_or_default();
                self.push_tool(
                    "shell",
                    one_line(&cmd),
                    p["call_id"].as_str().map(str::to_string),
                    at,
                );
            }
            Some("web_search_call") => {
                let q = p["action"]["query"].as_str().unwrap_or_default();
                self.push_tool("web_search", one_line(q), None, at);
            }
            Some(
                "function_call_output" | "custom_tool_call_output" | "local_shell_call_output",
            ) => {
                if let Some(id) = p["call_id"].as_str() {
                    let out = match &p["output"] {
                        Value::String(s) => serde_json::from_str::<Value>(s)
                            .ok()
                            .and_then(|o| o["output"].as_str().map(str::to_string))
                            .unwrap_or_else(|| s.clone()),
                        other => flatten(other),
                    };
                    self.attach_output(id, out);
                }
            }
            _ => {}
        }
    }
}

static BACKGROUND: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
    regex::Regex::new(r"running in background with ID: (\w+)\. Output is being written to: (\S+)")
        .unwrap()
});

/// The text inside `<name>…</name>`.
fn tag(s: &str, name: &str) -> Option<String> {
    let open = format!("<{name}>");
    let a = s.find(&open)? + open.len();
    let b = s[a..].find(&format!("</{name}>"))? + a;
    Some(s[a..b].trim().to_string())
}

/// Text without its XML-like tags.
fn strip_tags(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut inside = false;
    for c in s.chars() {
        match c {
            '<' => inside = true,
            '>' if inside => inside = false,
            _ if !inside => out.push(c),
            _ => {}
        }
    }
    out.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

pub(crate) fn time_of(v: &Value) -> i64 {
    v["timestamp"]
        .as_str()
        .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
        .map_or(0, |d| d.timestamp())
}

/// A prompt as the person typed it: a slash command shows as
/// `/name args`; harness notes and reminders drop out.
fn clean_prompt(s: &str) -> Option<String> {
    let s = s.trim();
    if let (Some(a), Some(b)) = (s.find("<command-name>"), s.find("</command-name>")) {
        let name = s[a + 14..b].trim();
        let args = s
            .find("<command-args>")
            .zip(s.find("</command-args>"))
            .map(|(x, y)| s[x + 14..y].trim())
            .unwrap_or_default();
        return Some(format!("{name} {args}").trim().to_string());
    }
    if s.starts_with("<local-command") || s.starts_with("[Request interrupted") {
        return None;
    }
    let mut out = s.to_string();
    while let (Some(a), Some(b)) = (
        out.find("<system-reminder>"),
        out.find("</system-reminder>"),
    ) {
        if b < a {
            break;
        }
        out.replace_range(a..b + "</system-reminder>".len(), "");
    }
    let out = out.trim();
    (!out.is_empty()).then(|| out.to_string())
}

/// Text of a tool result: a string, or the text parts of a list.
pub(crate) fn flatten(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Array(parts) => parts
            .iter()
            .filter_map(|p| p["text"].as_str().or_else(|| p.as_str()))
            .collect::<Vec<_>>()
            .join("\n"),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

/// One line that says what a tool call does.
pub(crate) fn summarize(input: &Value) -> String {
    for key in [
        "description",
        "command",
        "cmd",
        "file_path",
        "path",
        "pattern",
        "query",
        "url",
        "prompt",
        "skill",
    ] {
        match &input[key] {
            Value::String(s) if !s.trim().is_empty() => return one_line(s),
            Value::Array(a) if !a.is_empty() => {
                return one_line(
                    &a.iter()
                        .filter_map(Value::as_str)
                        .collect::<Vec<_>>()
                        .join(" "),
                );
            }
            _ => {}
        }
    }
    if input.is_null() {
        String::new()
    } else {
        one_line(&input.to_string())
    }
}

pub(crate) fn one_line(s: &str) -> String {
    let s = s.split_whitespace().collect::<Vec<_>>().join(" ");
    cut(&s, 160)
}

pub(crate) fn cut(s: &str, max: usize) -> String {
    if s.len() <= max {
        return s.to_string();
    }
    let mut end = max;
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…", &s[..end])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read(provider: Provider, lines: &[&str]) -> Vec<Entry> {
        let path = std::env::temp_dir().join(format!(
            "kuzgun-transcript-{}-{:?}.jsonl",
            std::process::id(),
            provider
        ));
        std::fs::write(&path, lines.join("\n") + "\n").unwrap();
        let mut t = Transcript::default();
        t.update(&path, provider);
        let _ = std::fs::remove_file(path);
        t.entries
    }

    #[test]
    fn reads_a_claude_session() {
        let e = read(
            Provider::Claude,
            &[
                r#"{"type":"user","message":{"content":"<command-message>implement</command-message> <command-name>/implement</command-name> <command-args>115</command-args>"}}"#,
                r#"{"type":"assistant","message":{"content":[{"type":"thinking","thinking":"Read the ticket."}]}}"#,
                r#"{"type":"assistant","message":{"content":[{"type":"tool_use","id":"t1","name":"Bash","input":{"command":"cargo test","description":"Run tests"}}]}}"#,
                r#"{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"t1","content":"ok. 16 passed"}]}}"#,
                r#"{"type":"assistant","message":{"content":[{"type":"text","text":"Done."}]}}"#,
                r#"{"type":"user","isMeta":true,"message":{"content":"hidden"}}"#,
            ],
        );
        let kinds: Vec<Kind> = e.iter().map(|x| x.kind).collect();
        assert_eq!(
            kinds,
            vec![Kind::User, Kind::Thinking, Kind::Tool, Kind::Assistant]
        );
        assert_eq!(e[0].text, "/implement 115");
        assert_eq!(e[2].text, "Run tests");
        assert_eq!(e[2].output.as_deref(), Some("ok. 16 passed"));
        let edit = read(
            Provider::Claude,
            &[
                r#"{"type":"assistant","message":{"content":[{"type":"tool_use","id":"t2","name":"Edit","input":{"file_path":"/r/a.rs","old_string":"a\nb\n","new_string":"a\nc\n"}}]}}"#,
            ],
        );
        assert_eq!((edit[0].edits[0].added, edit[0].edits[0].removed), (1, 1));
    }

    #[test]
    fn reads_a_codex_session() {
        let e = read(
            Provider::Codex,
            &[
                r#"{"type":"session_meta","payload":{"cwd":"/r"}}"#,
                r#"{"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"<environment_context>x</environment_context>"}]}}"#,
                r#"{"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"Fix the parser"}]}}"#,
                r#"{"type":"response_item","payload":{"type":"function_call","name":"shell","arguments":"{\"command\":[\"cargo\",\"test\"]}","call_id":"c1"}}"#,
                r#"{"type":"response_item","payload":{"type":"function_call_output","call_id":"c1","output":"{\"output\":\"all green\"}"}}"#,
                r#"{"type":"response_item","payload":{"type":"message","role":"assistant","content":[{"type":"output_text","text":"Fixed."}]}}"#,
            ],
        );
        let kinds: Vec<Kind> = e.iter().map(|x| x.kind).collect();
        assert_eq!(kinds, vec![Kind::User, Kind::Tool, Kind::Assistant]);
        assert_eq!(e[1].text, "cargo test");
        assert_eq!(e[1].output.as_deref(), Some("all green"));
        let patch = patch_edits(
            "*** Begin Patch\n*** Add File: src/new.rs\n+fn main() {}\n*** Update File: src/a.rs\n@@\n-old\n+new\n+more\n*** End Patch",
        );
        assert_eq!(patch.len(), 2);
        assert!(patch[0].created);
        assert_eq!((patch[1].added, patch[1].removed), (2, 1));
    }
}
