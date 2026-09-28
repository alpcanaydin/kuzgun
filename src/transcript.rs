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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Provider {
    Claude,
    Codex,
}

impl Provider {
    pub fn label(self) -> &'static str {
        match self {
            Provider::Claude => "Claude Code",
            Provider::Codex => "Codex",
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
}

const MAX_TEXT: usize = 12_000;
const MAX_OUTPUT: usize = 4_000;

impl Transcript {
    /// Reads the lines added since the last call. Returns whether new
    /// entries came in.
    pub fn update(&mut self, path: &Path, provider: Provider) -> bool {
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
            }
        }
        self.offset += end as u64;
        let _ = before;
        end > 0
    }

    fn push(&mut self, kind: Kind, text: String, at: i64) {
        let text = text.trim();
        if !text.is_empty() {
            self.entries.push(Entry { kind, text: cut(text, MAX_TEXT), tool: None, output: None, call: None, at });
        }
    }

    fn push_tool(&mut self, name: &str, summary: String, call: Option<String>, at: i64) {
        self.entries.push(Entry {
            kind: Kind::Tool,
            text: summary,
            tool: Some(name.to_string()),
            output: None,
            call,
            at,
        });
    }

    fn attach_output(&mut self, call: &str, output: String) {
        if let Some(e) = self.entries.iter_mut().rev().find(|e| e.call.as_deref() == Some(call)) {
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
                            Some("text") => self.claude_prompt(p["text"].as_str().unwrap_or_default(), at),
                            Some("tool_result") => {
                                if let Some(id) = p["tool_use_id"].as_str() {
                                    self.attach_output(id, flatten(&p["content"]));
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
                    self.turn_tokens += v["message"]["usage"]["output_tokens"].as_u64().unwrap_or(0);
                    self.last_message = id;
                }
                for p in content.as_array().into_iter().flatten() {
                    match p["type"].as_str() {
                        Some("text") => self.push(Kind::Assistant, p["text"].as_str().unwrap_or_default().to_string(), at),
                        Some("thinking") => self.push(Kind::Thinking, p["thinking"].as_str().unwrap_or_default().to_string(), at),
                        Some("tool_use") => {
                            let name = p["name"].as_str().unwrap_or("tool");
                            self.push_tool(name, summarize(&p["input"]), p["id"].as_str().map(str::to_string), at);
                        }
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }

    /// A user line of Claude Code: a prompt, or a note the harness adds in
    /// the person's name (a background task finished, a subagent reported).
    fn claude_prompt(&mut self, text: &str, at: i64) {
        let text = text.trim();
        if text.contains("<task-notification>") {
            let summary = tag(text, "summary").or_else(|| tag(text, "status")).unwrap_or_else(|| "finished".into());
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
            self.turn_start = at;
            self.turn_tokens = 0;
            self.last_message = None;
            self.push(Kind::User, t, at);
        }
    }

    fn codex_line(&mut self, v: &Value) {
        let at = time_of(v);
        if v["type"].as_str() == Some("event_msg") && v["payload"]["type"].as_str() == Some("token_count") {
            self.turn_tokens += v["payload"]["info"]["last_token_usage"]["output_tokens"].as_u64().unwrap_or(0);
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
                let args = p["arguments"].as_str().and_then(|a| serde_json::from_str::<Value>(a).ok()).unwrap_or(Value::Null);
                let name = p["name"].as_str().unwrap_or("tool");
                self.push_tool(name, summarize(&args), p["call_id"].as_str().map(str::to_string), at);
            }
            Some("custom_tool_call") => {
                let name = p["name"].as_str().unwrap_or("tool");
                let input = p["input"].as_str().unwrap_or_default();
                self.push_tool(name, one_line(input), p["call_id"].as_str().map(str::to_string), at);
            }
            Some("local_shell_call") => {
                let cmd = p["action"]["command"]
                    .as_array()
                    .map(|a| a.iter().filter_map(Value::as_str).collect::<Vec<_>>().join(" "))
                    .unwrap_or_default();
                self.push_tool("shell", one_line(&cmd), p["call_id"].as_str().map(str::to_string), at);
            }
            Some("web_search_call") => {
                let q = p["action"]["query"].as_str().unwrap_or_default();
                self.push_tool("web_search", one_line(q), None, at);
            }
            Some("function_call_output" | "custom_tool_call_output" | "local_shell_call_output") => {
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
    out.lines().map(str::trim).filter(|l| !l.is_empty()).collect::<Vec<_>>().join("\n")
}

fn time_of(v: &Value) -> i64 {
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
    while let (Some(a), Some(b)) = (out.find("<system-reminder>"), out.find("</system-reminder>")) {
        if b < a {
            break;
        }
        out.replace_range(a..b + "</system-reminder>".len(), "");
    }
    let out = out.trim();
    (!out.is_empty()).then(|| out.to_string())
}

/// Text of a tool result: a string, or the text parts of a list.
fn flatten(v: &Value) -> String {
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
fn summarize(input: &Value) -> String {
    for key in ["description", "command", "cmd", "file_path", "path", "pattern", "query", "url", "prompt", "skill"] {
        match &input[key] {
            Value::String(s) if !s.trim().is_empty() => return one_line(s),
            Value::Array(a) if !a.is_empty() => {
                return one_line(&a.iter().filter_map(Value::as_str).collect::<Vec<_>>().join(" "));
            }
            _ => {}
        }
    }
    if input.is_null() { String::new() } else { one_line(&input.to_string()) }
}

fn one_line(s: &str) -> String {
    let s = s.split_whitespace().collect::<Vec<_>>().join(" ");
    cut(&s, 160)
}

fn cut(s: &str, max: usize) -> String {
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
        let path = std::env::temp_dir().join(format!("kuzgun-transcript-{}-{:?}.jsonl", std::process::id(), provider));
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
        assert_eq!(kinds, vec![Kind::User, Kind::Thinking, Kind::Tool, Kind::Assistant]);
        assert_eq!(e[0].text, "/implement 115");
        assert_eq!(e[2].text, "Run tests");
        assert_eq!(e[2].output.as_deref(), Some("ok. 16 passed"));
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
    }
}
