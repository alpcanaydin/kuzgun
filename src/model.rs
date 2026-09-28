//! Tickets on disk: discovery, parsing and the board model.
//!
//! A board is a folder of markdown files. The layouts it reads:
//!
//! - Local issue tracker: `<root>/<project>/issues/NN-slug.md`, with a
//!   `spec.md` or `map.md` next to `issues/`.
//! - Wayfinder maps: the same, plus `Type:` and a derived status (`## Answer`).
//! - A flat folder of `NN-slug.md` files, or files with front matter.
//!
//! Properties come from three places, in this order of trust: YAML front
//! matter, `**Key:** value` lines, and plain `Key: value` lines between the
//! title and the first `## ` heading.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::LazyLock;
use std::time::SystemTime;

use regex::Regex;

/// Workflow category of a status: the icon shape and the column order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Category {
    Backlog,
    Todo,
    InProgress,
    InReview,
    Done,
    Canceled,
}

impl Category {
    pub const ALL: [Category; 6] = [
        Category::Backlog,
        Category::Todo,
        Category::InProgress,
        Category::InReview,
        Category::Done,
        Category::Canceled,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Category::Backlog => "Backlog",
            Category::Todo => "Todo",
            Category::InProgress => "In Progress",
            Category::InReview => "In Review",
            Category::Done => "Done",
            Category::Canceled => "Canceled",
        }
    }

    /// Status written when a card is dropped on an empty default column.
    pub fn default_status(self) -> &'static str {
        match self {
            Category::Backlog => "draft",
            Category::Todo => "ready-for-agent",
            Category::InProgress => "in-progress",
            Category::InReview => "in-review",
            Category::Done => "done",
            Category::Canceled => "wontfix",
        }
    }

    pub fn is_closed(self) -> bool {
        matches!(self, Category::Done | Category::Canceled)
    }
}

/// Maps a raw status string to its category.
pub fn categorize(status: &str) -> Category {
    let s = normalize(status);
    match s.as_str() {
        "draft" | "backlog" | "needs-triage" | "triage" | "needs-info" | "idea" | "icebox"
        | "someday" | "proposed" | "inbox" => Category::Backlog,
        "open" | "todo" | "to-do" | "ready" | "ready-for-agent" | "ready-for-human" | "planned"
        | "unstarted" | "new" | "blocked" | "next" | "selected" => Category::Todo,
        "claimed" | "in-progress" | "doing" | "wip" | "started" | "active" | "working"
        | "implementing" => Category::InProgress,
        "in-review" | "review" | "reviewing" | "qa" | "testing" | "verify" | "verifying"
        | "pr" | "needs-review" => Category::InReview,
        "done" | "resolved" | "closed" | "complete" | "completed" | "shipped" | "merged"
        | "fixed" | "released" | "answered" => Category::Done,
        "wontfix" | "won't-fix" | "wont-fix" | "canceled" | "cancelled" | "out-of-scope"
        | "duplicate" | "obsolete" | "rejected" | "abandoned" | "superseded" => Category::Canceled,
        _ if s.contains("progress") => Category::InProgress,
        _ if s.contains("review") => Category::InReview,
        _ if s.contains("done") || s.contains("resolv") => Category::Done,
        _ if s.contains("cancel") || s.contains("wontfix") => Category::Canceled,
        _ if s.contains("draft") || s.contains("triage") => Category::Backlog,
        _ => Category::Todo,
    }
}

/// `Ready for Agent` / `ready_for_agent` → `ready-for-agent`.
/// The status word of a status line, without a note after it.
fn clean_status(s: &str) -> String {
    let s = s.trim().trim_matches('`');
    let cut = [" (", "(", ",", ";", " - ", " — ", " – ", ": "]
        .iter()
        .filter_map(|sep| s.find(sep))
        .min()
        .unwrap_or(s.len());
    let head = s[..cut].trim().trim_matches(['`', '*']);
    if head.is_empty() { s.to_string() } else { head.to_string() }
}

pub fn normalize(status: &str) -> String {
    status
        .trim()
        .trim_matches('`')
        .to_lowercase()
        .replace(['_', ' '], "-")
}

/// A document of a project that is not a ticket (spec, map, PRD, README).
#[derive(Clone, Debug)]
pub struct Doc {
    pub name: String,
    pub path: PathBuf,
    pub title: String,
}

#[derive(Clone, Debug)]
pub struct Project {
    /// Folder name, for example `walking-skeleton`.
    pub name: String,
    /// H1 of its spec or map, else the humanized folder name.
    pub title: String,
    pub dir: PathBuf,
    /// Short identifier prefix, for example `WS`.
    pub key: String,
    pub docs: Vec<Doc>,
    /// A wayfinder map, when the project has `map.md`.
    pub map: Option<MapInfo>,
}

/// The parts of a wayfinder map a board shows.
#[derive(Clone, Debug, Default)]
pub struct MapInfo {
    pub path: PathBuf,
    pub destination: String,
    /// `## Not yet specified`: fog that graduates into tickets.
    pub fog: Vec<String>,
    pub out_of_scope: Vec<String>,
    pub decisions: usize,
}

/// Who can take a ticket: an agent alone (AFK) or a person (HITL).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Afk,
    Hitl,
}

impl Mode {
    pub fn label(self) -> &'static str {
        match self {
            Mode::Afk => "AFK",
            Mode::Hitl => "HITL",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct CheckItem {
    pub done: bool,
    pub text: String,
    /// 0-based line in the file.
    pub line: usize,
    pub depth: usize,
}

/// One ticket reference as written in `Blocked by:`.
#[derive(Clone, Debug, PartialEq)]
pub struct RefSpec {
    pub num: Option<u32>,
    pub path: Option<PathBuf>,
    pub text: String,
}

#[derive(Clone, Debug)]
pub struct Ticket {
    pub path: PathBuf,
    /// Path relative to the board root.
    pub rel: String,
    pub project: usize,
    pub num: Option<u32>,
    pub slug: String,
    /// `WS-12`.
    pub key: String,
    pub title: String,
    /// Status as written in the file (or derived), if any.
    pub status: String,
    /// `normalize(status)`, the column key.
    pub status_key: String,
    pub status_derived: bool,
    pub category: Category,
    /// `Type:` (research, grilling, task…).
    pub kind: Option<String>,
    /// Every header property, in file order (key as written).
    pub props: Vec<(String, String)>,
    pub blocked_refs: Vec<RefSpec>,
    /// Resolved later against the whole board (indexes into `Board::tickets`).
    pub blocked_by: Vec<usize>,
    pub blocks: Vec<usize>,
    pub related: Vec<usize>,
    /// Links to other markdown files in the body (resolved paths).
    pub links: Vec<PathBuf>,
    pub checklist: Vec<CheckItem>,
    pub comments: usize,
    /// Markdown after the H1 without the header property lines the side
    /// panel shows anyway.
    pub body: String,
    pub raw: String,
    pub words: usize,
    pub bytes: u64,
    pub modified: Option<SystemTime>,
    pub created: Option<SystemTime>,
    pub labels: Vec<String>,
    pub assignee: Option<String>,
    pub needs_human: Option<String>,
    /// A coding agent working on (or done with) this ticket right now,
    /// from the agent transcripts. It overrides `status_key` / `category`;
    /// `status` keeps what the file says.
    pub agent: Option<crate::agents::AgentRun>,
    /// Why the shown status differs from the file (agent run, all
    /// sub-tasks checked), when it does.
    pub inferred: Option<String>,
    pub spec_link: Option<PathBuf>,
    /// The wayfinder map links it under `## Out of scope` and it has no
    /// answer: closed without being done.
    pub ruled_out: bool,
}

pub const RULED_OUT: &str = "The map rules it out of scope";

impl Ticket {
    /// Shows a ruled-out ticket as out of scope, whatever its file says.
    pub fn apply_ruled_out(&mut self) {
        if self.ruled_out {
            self.status_key = "out-of-scope".into();
            self.category = Category::Canceled;
            self.inferred = Some(RULED_OUT.into());
        }
    }
}

impl Ticket {
    pub fn prop(&self, key: &str) -> Option<&str> {
        self.props
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(key))
            .map(|(_, v)| v.as_str())
    }

    /// Values of a header field; list fields (labels, tags) split on commas.
    pub fn field_values(&self, key: &str) -> Vec<String> {
        let k = normalize(key);
        if matches!(k.as_str(), "labels" | "label" | "tags") {
            return self.labels.clone();
        }
        self.props
            .iter()
            .filter(|(pk, _)| normalize(pk) == k)
            .map(|(_, v)| v.trim().trim_matches('`').to_string())
            .filter(|v| !v.is_empty())
            .collect()
    }

    pub fn checklist_counts(&self) -> (usize, usize) {
        let done = self.checklist.iter().filter(|c| c.done).count();
        (done, self.checklist.len())
    }
}

#[derive(Clone, Debug, Default)]
pub struct Board {
    pub root: PathBuf,
    pub projects: Vec<Project>,
    pub tickets: Vec<Ticket>,
    /// Files that looked like tickets but failed to read.
    pub errors: Vec<(PathBuf, String)>,
    /// Short fields that repeat across tickets (Type, Tranche, Priority,
    /// Labels…): filters and card chips.
    pub facets: Vec<Facet>,
    /// Repo label → (canonical triage role, meaning), from
    /// `docs/agents/triage-labels.md`.
    pub roles: HashMap<String, (String, String)>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Facet {
    /// Key as written in the files (first spelling seen).
    pub key: String,
    pub values: Vec<String>,
}

/// One column of the board: a raw status and its category.
#[derive(Clone, Debug, PartialEq)]
pub struct Column {
    pub status: String,
    pub category: Category,
}

impl Board {
    pub fn load(root: &Path) -> Board {
        let root = root.to_path_buf();
        let mut files = Vec::new();
        walk(&root, &root, 0, &mut files);
        files.sort();

        let mut projects: Vec<Project> = Vec::new();
        let mut project_ix: HashMap<PathBuf, usize> = HashMap::new();
        let mut tickets = Vec::new();
        let mut errors = Vec::new();
        let mut docs: Vec<(PathBuf, Doc)> = Vec::new();

        for file in files {
            let text = match std::fs::read_to_string(&file) {
                Ok(t) => t,
                Err(e) => {
                    errors.push((file.clone(), e.to_string()));
                    continue;
                }
            };
            let fname = file.file_name().unwrap_or_default().to_string_lossy().to_string();
            let parent = file.parent().unwrap_or(&root).to_path_buf();
            let in_issue_dir = is_issue_dir(&parent);
            let fm = front_matter(&text).0;
            let has_status = fm.keys().any(|k| k.eq_ignore_ascii_case("status"))
                || header_props(&text).iter().any(|(k, _)| k.eq_ignore_ascii_case("status"));
            // Tickets live in `issues/`. A small-numbered file next to a spec
            // or map with a status also counts. Dated plans, ADRs
            // (`0001-x.md`) and notes with a status line do not.
            let small_num = NUM_FILE
                .captures(&fname)
                .is_some_and(|c| c[1].len() <= 3 && !DATED_FILE.is_match(&fname));
            let beside_spec = ["spec.md", "map.md", "PRD.md", "prd.md"].iter().any(|d| parent.join(d).is_file());
            let is_ticket = if in_issue_dir {
                !is_doc_name(&fname)
            } else {
                small_num && has_status && beside_spec
            };
            let project_dir = if in_issue_dir {
                parent.parent().unwrap_or(&root).to_path_buf()
            } else {
                parent.clone()
            };
            if !is_ticket {
                if is_doc_name(&fname) {
                    let title = first_h1(&text).unwrap_or_else(|| fname.clone());
                    docs.push((
                        project_dir,
                        Doc {
                            name: fname,
                            path: file,
                            title,
                        },
                    ));
                }
                continue;
            }
            let pix = *project_ix.entry(project_dir.clone()).or_insert_with(|| {
                // The path from the board root keeps two `plans` folders apart.
                let rel = project_dir
                    .strip_prefix(&root)
                    .ok()
                    .map(|r| r.display().to_string())
                    .filter(|r| !r.is_empty())
                    .unwrap_or_else(|| dir_name(&project_dir));
                projects.push(Project {
                    name: rel,
                    title: humanize(&dir_name(&project_dir)),
                    dir: project_dir.clone(),
                    key: String::new(),
                    docs: Vec::new(),
                    map: None,
                });
                projects.len() - 1
            });
            let meta = std::fs::metadata(&file).ok();
            let mut t = parse_ticket(&file, &text);
            t.project = pix;
            t.rel = file
                .strip_prefix(&root)
                .map(|p| p.display().to_string())
                .unwrap_or_else(|_| file.display().to_string());
            t.bytes = meta.as_ref().map(|m| m.len()).unwrap_or(0);
            t.modified = meta.as_ref().and_then(|m| m.modified().ok());
            t.created = meta.as_ref().and_then(|m| m.created().ok());
            tickets.push(t);
        }

        for (dir, doc) in docs {
            if let Some(&ix) = project_ix.get(&dir) {
                projects[ix].docs.push(doc);
            }
        }
        for p in &mut projects {
            p.docs.sort_by_key(|d| doc_rank(&d.name));
            p.map = p
                .docs
                .iter()
                .find(|d| d.name.eq_ignore_ascii_case("map.md"))
                .and_then(|d| std::fs::read_to_string(&d.path).ok().map(|t| parse_map(&d.path, &t)));
            if let Some(d) = p.docs.first() {
                let t = d.title.trim();
                if !t.is_empty() {
                    p.title = t.to_string();
                }
            }
        }
        disambiguate_titles(&mut projects);
        assign_keys(&mut projects);
        projects_sorted(&mut projects, &mut tickets);
        for t in &mut tickets {
            let p = &projects[t.project];
            t.key = match t.num {
                Some(n) => format!("{}-{n}", p.key),
                None => format!("{}-{}", p.key, t.slug),
            };
        }
        tickets.sort_by(|a, b| {
            (a.project, a.num.unwrap_or(u32::MAX), &a.slug).cmp(&(
                b.project,
                b.num.unwrap_or(u32::MAX),
                &b.slug,
            ))
        });
        let roles = triage_roles(&root);
        for t in &mut tickets {
            if let Some((role, _)) = roles.get(&t.status_key) {
                t.category = categorize(role);
            }
        }
        mark_ruled_out(&projects, &mut tickets);
        resolve_relations(&projects, &mut tickets);
        let facets = find_facets(&tickets);

        Board {
            root,
            projects,
            tickets,
            errors,
            facets,
            roles,
        }
    }

    /// Columns in workflow order: every status in use, plus the default status
    /// of each of the first five categories when none of it is in use, so
    /// every stage stays a drop target.
    /// Columns for the statuses in use in one project (all when None).
    pub fn columns_of(&self, project: Option<usize>, include_defaults: bool) -> Vec<Column> {
        let mut seen: BTreeMap<(Category, String), String> = BTreeMap::new();
        for t in self.tickets.iter().filter(|t| project.is_none_or(|p| t.project == p)) {
            seen.entry((t.category, rank_key(&t.status_key)))
                .or_insert_with(|| t.status_key.clone());
        }
        if include_defaults {
            for c in Category::ALL {
                if c == Category::Canceled {
                    continue;
                }
                if !seen.keys().any(|(cat, _)| *cat == c) {
                    let s = c.default_status().to_string();
                    seen.insert((c, rank_key(&s)), s);
                }
            }
        }
        seen.into_iter()
            .map(|((category, _), status)| Column { status, category })
            .collect()
    }

    /// Is every blocker of this ticket closed?
    pub fn is_blocked(&self, ix: usize) -> bool {
        let t = &self.tickets[ix];
        !t.category.is_closed()
            && t.blocked_by
                .iter()
                .any(|&b| !self.tickets[b].category.is_closed())
    }


    pub fn mode(&self, ix: usize) -> Option<Mode> {
        let t = &self.tickets[ix];
        let role = self
            .roles
            .get(&t.status_key)
            .map(|(r, _)| r.as_str())
            .unwrap_or(t.status_key.as_str());
        match role {
            "ready-for-human" => return Some(Mode::Hitl),
            "ready-for-agent" if t.needs_human.is_none() => return Some(Mode::Afk),
            _ => {}
        }
        if t.needs_human.is_some() {
            return Some(Mode::Hitl);
        }
        match t.kind.as_deref() {
            Some("research") => Some(Mode::Afk),
            Some("prototype" | "grilling") => Some(Mode::Hitl),
            _ => None,
        }
    }

    pub fn find_path(&self, path: &Path) -> Option<usize> {
        self.tickets.iter().position(|t| t.path == path)
    }
}

/// Fields that are not free text: at least two tickets carry them, every
/// value is short, and there are few distinct values. Status, relations and
/// prose fields are left out; they have their own places.
fn find_facets(tickets: &[Ticket]) -> Vec<Facet> {
    const SKIP: [&str; 16] = [
        "status", "blocked-by", "blocked_by", "depends-on", "spec", "prd", "map", "what-to-build",
        "needs-a-human", "title", "claimed-by", "claimed_by", "claimed-at", "claimed_at", "parent",
        "undermined-by",
    ];
    let mut order: Vec<String> = Vec::new();
    let mut spelled: HashMap<String, String> = HashMap::new();
    let mut vals: HashMap<String, Vec<String>> = HashMap::new();
    let mut hits: HashMap<String, usize> = HashMap::new();
    let mut bad: std::collections::HashSet<String> = Default::default();
    for t in tickets {
        let mut seen_here: Vec<String> = Vec::new();
        let mut keys: Vec<String> = t.props.iter().map(|(k, _)| k.clone()).collect();
        if !t.labels.is_empty() && !keys.iter().any(|k| matches!(normalize(k).as_str(), "labels" | "label" | "tags")) {
            keys.push("Labels".into());
        }
        for key in keys {
            let k = normalize(&key);
            if SKIP.contains(&k.as_str()) || seen_here.contains(&k) {
                continue;
            }
            seen_here.push(k.clone());
            if !spelled.contains_key(&k) {
                spelled.insert(k.clone(), key.clone());
                order.push(k.clone());
            }
            *hits.entry(k.clone()).or_default() += 1;
            for v in t.field_values(&key) {
                if v.chars().count() > 28 || v.contains("](") {
                    bad.insert(k.clone());
                }
                let list = vals.entry(k.clone()).or_default();
                if !list.contains(&v) {
                    list.push(v);
                }
            }
        }
    }
    order
        .into_iter()
        .filter(|k| !bad.contains(k) && hits.get(k).copied().unwrap_or(0) >= 2)
        .filter_map(|k| {
            let mut values = vals.remove(&k)?;
            if values.is_empty() || values.len() > 16 {
                return None;
            }
            values.sort_by(|a, b| natural_key(a).cmp(&natural_key(b)));
            Some(Facet { key: spelled.remove(&k).unwrap_or(k), values })
        })
        .collect()
}

/// `T10` after `T9`: digits compare as numbers.
pub fn natural_key(s: &str) -> (String, u64) {
    let digits: String = s.chars().filter(|c| c.is_ascii_digit()).collect();
    let letters: String = s.chars().filter(|c| !c.is_ascii_digit()).collect::<String>().to_lowercase();
    (letters, digits.parse().unwrap_or(0))
}

/// `docs/agents/triage-labels.md` above the board: the repo's label for
/// each canonical triage role. Rows look like
/// `| \`needs-triage\` | \`our-label\` | meaning |`.
fn triage_roles(root: &Path) -> HashMap<String, (String, String)> {
    let mut out = HashMap::new();
    let mut dir = Some(root);
    let mut file = None;
    for _ in 0..8 {
        let Some(d) = dir else { break };
        let f = d.join("docs/agents/triage-labels.md");
        if f.is_file() {
            file = Some(f);
            break;
        }
        if d.join(".git").exists() {
            break;
        }
        dir = d.parent();
    }
    let Some(text) = file.and_then(|f| std::fs::read_to_string(f).ok()) else {
        return out;
    };
    for line in text.lines() {
        let cells: Vec<&str> = line.split('|').map(str::trim).filter(|c| !c.is_empty()).collect();
        if cells.len() < 2 || !cells[0].starts_with('`') || !cells[1].starts_with('`') {
            continue;
        }
        let role = normalize(cells[0]);
        let label = normalize(cells[1]);
        let meaning = cells.get(2).map(|m| m.to_string()).unwrap_or_default();
        out.insert(label, (role, meaning));
    }
    out
}

/// The standard meaning of the triage roles and wayfinder states, for
/// column tooltips when the repo does not define its own.
pub fn role_meaning(status_key: &str) -> Option<&'static str> {
    Some(match status_key {
        "needs-triage" => "Maintainer needs to evaluate this issue",
        "needs-info" => "Waiting on the reporter for more information",
        "ready-for-agent" => "Fully specified, ready for an AFK agent",
        "ready-for-human" => "Needs a human to build it",
        "wontfix" => "Will not be actioned",
        "open" => "Wayfinder: not claimed yet; on the frontier once unblocked",
        "claimed" => "Wayfinder: a session is working on it",
        "resolved" => "Wayfinder: answered; the gist is in the map's Decisions so far",
        "waiting-on-blockers" => "Not started and waiting: at least one blocker is still open",
        _ => return None,
    })
}

/// Destination, fog and out-of-scope bullets of a wayfinder map.
pub fn parse_map(path: &Path, text: &str) -> MapInfo {
    let mut m = MapInfo { path: path.to_path_buf(), ..Default::default() };
    let mut section = String::new();
    for line in text.lines() {
        if let Some(h) = line.strip_prefix("## ") {
            section = h.trim().to_lowercase();
            continue;
        }
        let bullet = line.strip_prefix("- ").or_else(|| line.strip_prefix("* ")).map(str::trim);
        match section.as_str() {
            "destination" if !line.trim().is_empty() => {
                if !m.destination.is_empty() {
                    m.destination.push(' ');
                }
                m.destination.push_str(line.trim());
            }
            "not yet specified" | "fog" => {
                if let Some(b) = bullet {
                    m.fog.push(b.to_string());
                }
            }
            "out of scope" => {
                if let Some(b) = bullet {
                    m.out_of_scope.push(b.to_string());
                }
            }
            "decisions so far" if bullet.is_some() => m.decisions += 1,
            _ => {}
        }
    }
    m
}

/// Field names that mean a delivery stage. They keep their own name and
/// get the milestone diamond.
const MILESTONE_KEYS: [&str; 7] = ["tranche", "milestone", "phase", "stage", "sprint", "iteration", "release"];

pub fn is_milestone(key: &str) -> bool {
    MILESTONE_KEYS.contains(&normalize(key).as_str())
}

/// The name a field goes by in the interface.
pub fn field_label(key: &str) -> String {
    match normalize(key).as_str() {
        "labels" | "label" | "tags" => "Labels".into(),
        _ => humanize(key),
    }
}

/// Order statuses inside one category: known triage roles first.
fn rank_key(s: &str) -> String {
    let order = [
        "needs-triage", "needs-info", "draft", "backlog", "open", "ready-for-agent",
        "ready-for-human", "todo", "claimed", "in-progress", "in-review", "done", "resolved",
        "wontfix",
    ];
    match order.iter().position(|o| *o == s) {
        Some(i) => format!("{i:02}"),
        None => format!("99{s}"),
    }
}

fn projects_sorted(projects: &mut [Project], _tickets: &mut [Ticket]) {
    // Projects keep discovery order (sorted paths). Kept as a hook for a
    // future manual order.
    let _ = projects;
}

fn resolve_relations(projects: &[Project], tickets: &mut [Ticket]) {
    let mut by_num: HashMap<(usize, u32), usize> = HashMap::new();
    let mut by_path: HashMap<PathBuf, usize> = HashMap::new();
    for (i, t) in tickets.iter().enumerate() {
        if let Some(n) = t.num {
            by_num.entry((t.project, n)).or_insert(i);
        }
        by_path.insert(canon(&t.path), i);
    }
    let _ = projects;
    let n = tickets.len();
    let mut blocks: Vec<Vec<usize>> = vec![Vec::new(); n];
    for i in 0..n {
        let mut out = Vec::new();
        for r in &tickets[i].blocked_refs {
            let hit = r
                .path
                .as_ref()
                .and_then(|p| by_path.get(&canon(p)).copied())
                .or_else(|| r.num.and_then(|num| by_num.get(&(tickets[i].project, num)).copied()))
                .or_else(|| {
                    // "Blocked by" may name a ticket by its title.
                    let want = r.text.trim().trim_matches(['`', '"', '*']).to_lowercase();
                    let same = |j: &usize| tickets[*j].project == tickets[i].project;
                    let hits: Vec<usize> = (0..n).filter(|&j| !want.is_empty() && tickets[j].title.to_lowercase() == want).collect();
                    hits.iter().copied().find(same).or_else(|| hits.first().copied())
                });
            if let Some(j) = hit
                && j != i
                && !out.contains(&j)
            {
                out.push(j);
            }
        }
        for &j in &out {
            blocks[j].push(i);
        }
        let related: Vec<usize> = tickets[i]
            .links
            .iter()
            .filter_map(|p| by_path.get(&canon(p)).copied())
            .filter(|&j| j != i && !out.contains(&j))
            .fold(Vec::new(), |mut acc, j| {
                if !acc.contains(&j) {
                    acc.push(j);
                }
                acc
            });
        tickets[i].blocked_by = out;
        tickets[i].related = related;
    }
    for (i, b) in blocks.into_iter().enumerate() {
        tickets[i].blocks = b;
    }
}

fn canon(p: &Path) -> PathBuf {
    std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf())
}

/// Two to four capital letters per project, unique on the board.
fn assign_keys(projects: &mut [Project]) {
    let mut used: HashMap<String, usize> = HashMap::new();
    for p in projects.iter_mut() {
        let base = dir_name(&p.dir);
        let words: Vec<&str> = base
            .split(|c: char| !c.is_alphanumeric())
            .filter(|w| !w.is_empty())
            .collect();
        let mut key: String = if words.len() >= 2 {
            words.iter().take(3).filter_map(|w| w.chars().next()).collect()
        } else {
            words
                .first()
                .map(|w| w.chars().filter(|c| c.is_alphanumeric()).take(3).collect())
                .unwrap_or_else(|| "T".into())
        };
        key = key.to_uppercase();
        if key.is_empty() {
            key = "T".into();
        }
        let n = used.entry(key.clone()).or_insert(0);
        *n += 1;
        if *n > 1 {
            key = format!("{key}{n}");
        }
        p.key = key;
    }
}

/// Where a picked folder keeps its tickets.
#[derive(Clone, Debug, PartialEq)]
pub enum Tracker {
    /// A local markdown tracker folder.
    Local(PathBuf),
    /// The repo tracks issues elsewhere, for example "GitHub".
    Remote(String),
    /// No sign of mattpocock/skills: no tracker doc, no `.scratch/`, no
    /// ticket folder.
    Missing,
}

/// Finds the local tracker of a picked folder. A repo root resolves
/// through `docs/agents/issue-tracker.md`, else through `.scratch/`.
pub fn locate_tracker(picked: &Path) -> Tracker {
    let scratch = picked.join(".scratch");
    let config = std::fs::read_to_string(picked.join("docs/agents/issue-tracker.md")).unwrap_or_default();
    let heading = config.lines().find(|l| l.starts_with("# ")).unwrap_or_default();
    if !config.is_empty() && !heading.to_lowercase().contains("local") {
        // A remote tracker wins over a stray `.scratch/` folder.
        let name = heading.trim_start_matches("# ");
        let name = name.split_once(':').map_or(name, |(_, n)| n).trim();
        let name = if name.is_empty() { "another tracker".to_string() } else { name.to_string() };
        return Tracker::Remote(name);
    }
    // The local template names its folder in backticks: "files in `.scratch/`".
    let configured = Regex::new(r"markdown files in `([^`]+)`")
        .ok()
        .and_then(|re| re.captures(&config).map(|c| c[1].trim_end_matches('/').to_string()))
        .map(|rel| picked.join(rel))
        .filter(|p| p.is_dir());
    if let Some(dir) = configured {
        return Tracker::Local(dir);
    }
    if scratch.is_dir() {
        return Tracker::Local(scratch);
    }
    if picked.file_name().is_some_and(|n| n == ".scratch") || has_tracker_files(picked, 0) {
        Tracker::Local(picked.to_path_buf())
    } else {
        Tracker::Missing
    }
}

/// A folder holds a local tracker: an `issues/` folder with markdown, or a
/// `spec.md` or `map.md`, within three levels.
fn has_tracker_files(dir: &Path, depth: usize) -> bool {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return false;
    };
    let entries: Vec<PathBuf> = rd.flatten().map(|e| e.path()).collect();
    if entries.iter().any(|p| p.file_name().is_some_and(|n| n == "spec.md" || n == "map.md")) {
        return true;
    }
    if is_issue_dir(dir) && entries.iter().any(|p| p.extension().is_some_and(|e| e == "md")) {
        return true;
    }
    depth < 3
        && entries.iter().filter(|p| p.is_dir()).any(|p| {
            let name = p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
            !matches!(name.as_str(), "node_modules" | ".git" | "target" | "dist" | "build") && has_tracker_files(p, depth + 1)
        })
}

/// Tickets a map links under `## Out of scope`. A linked ticket with an
/// `## Answer` was resolved on the route; the link only gives context.
fn mark_ruled_out(projects: &[Project], tickets: &mut [Ticket]) {
    for (pix, p) in projects.iter().enumerate() {
        let Some(map) = &p.map else {
            continue;
        };
        let dir = map.path.parent().unwrap_or(&p.dir);
        let targets: Vec<PathBuf> = map
            .out_of_scope
            .iter()
            .flat_map(|line| MD_LINK.captures_iter(line).map(|c| canon(&dir.join(&c[2]))).collect::<Vec<_>>())
            .collect();
        for t in tickets.iter_mut().filter(|t| t.project == pix) {
            let answered = t.raw.lines().any(|l| l.trim().eq_ignore_ascii_case("## answer"));
            if !answered && targets.contains(&canon(&t.path)) {
                t.ruled_out = true;
                t.apply_ruled_out();
            }
        }
    }
}

/// Two projects with one title get their parent path: "Plans · apps/api".
fn disambiguate_titles(projects: &mut [Project]) {
    let mut seen: HashMap<String, usize> = HashMap::new();
    for p in projects.iter() {
        *seen.entry(p.title.to_lowercase()).or_default() += 1;
    }
    for p in projects.iter_mut() {
        if seen[&p.title.to_lowercase()] > 1
            && let Some((parent, _)) = p.name.rsplit_once('/')
        {
            p.title = format!("{} · {parent}", p.title);
        }
    }
}

fn dir_name(p: &Path) -> String {
    p.file_name()
        .map(|n| n.to_string_lossy().trim_start_matches('.').to_string())
        .unwrap_or_default()
}

/// `walking-skeleton` → `Walking skeleton`.
pub fn humanize(s: &str) -> String {
    let s = s.replace(['-', '_'], " ");
    let mut c = s.chars();
    match c.next() {
        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
        None => String::new(),
    }
}

fn is_issue_dir(p: &Path) -> bool {
    matches!(
        p.file_name().map(|n| n.to_string_lossy().to_lowercase()).as_deref(),
        Some("issues" | "tickets" | "tasks" | "todo")
    )
}

fn is_doc_name(name: &str) -> bool {
    doc_rank(name) < 9
}

fn doc_rank(name: &str) -> usize {
    match name.to_lowercase().as_str() {
        "spec.md" => 0,
        "map.md" => 1,
        "prd.md" => 2,
        "readme.md" => 3,
        "plan.md" => 4,
        "notes.md" => 5,
        _ => 9,
    }
}

const SKIP_DIRS: [&str; 8] = [
    "node_modules", "target", ".git", "dist", "build", ".next", "vendor", ".turbo",
];

fn walk(root: &Path, dir: &Path, depth: usize, out: &mut Vec<PathBuf>) {
    if depth > 6 {
        return;
    }
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    for e in rd.flatten() {
        let p = e.path();
        let name = e.file_name().to_string_lossy().to_string();
        let Ok(ft) = e.file_type() else {
            continue;
        };
        if ft.is_dir() {
            if SKIP_DIRS.contains(&name.as_str()) || (name.starts_with('.') && dir != root) {
                continue;
            }
            walk(root, &p, depth + 1, out);
        } else if name.to_lowercase().ends_with(".md") {
            out.push(p);
        }
    }
}

static DATED_FILE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\d{4}-\d{2}-\d{2}").unwrap());
static NUM_FILE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^(\d+)[-_ ](.+)\.md$").unwrap());
static BOLD_PROP: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^\*\*([A-Za-z][A-Za-z0-9 '/_-]{0,40}?):?\*\*:?\s*(.*)$").unwrap());
static PLAIN_PROP: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^([A-Z][A-Za-z0-9 '/_-]{0,30}):\s+(\S.*)$").unwrap());
static CHECK: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^(\s*)[-*+] \[( |x|X)\] (.*)$").unwrap());
static MD_LINK: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\[([^\]]*)\]\(([^)\s]+\.md)(#[^)]*)?\)").unwrap());
static TITLE_NUM: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^#?(\d{1,4})\s*[:.)-]\s*(.+)$").unwrap());
static LEAD_NUM: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^#?(\d{1,4})\b").unwrap());

/// YAML front matter as flat `key → raw value`, and the line where the body starts.
pub fn front_matter(text: &str) -> (BTreeMap<String, String>, usize) {
    let mut map = BTreeMap::new();
    let mut lines = text.lines();
    if lines.next().map(str::trim) != Some("---") {
        return (map, 0);
    }
    let mut last_key: Option<String> = None;
    for (i, line) in text.lines().enumerate().skip(1) {
        if line.trim() == "---" {
            return (map, i + 1);
        }
        if let Some(item) = line.trim_start().strip_prefix("- ")
            && line.starts_with([' ', '-'])
            && let Some(k) = &last_key
        {
            let v = map.entry(k.clone()).or_insert_with(String::new);
            if !v.is_empty() {
                v.push_str(", ");
            }
            v.push_str(item.trim().trim_matches('"'));
            continue;
        }
        if let Some((k, v)) = line.split_once(':')
            && !k.starts_with(' ')
        {
            let v = v.trim().trim_matches('"').trim_matches('\'');
            let v = v.trim_start_matches('[').trim_end_matches(']').to_string();
            map.insert(k.trim().to_string(), v);
            last_key = Some(k.trim().to_string());
        }
    }
    (BTreeMap::new(), 0)
}

fn first_h1(text: &str) -> Option<String> {
    let (_, start) = front_matter(text);
    text.lines()
        .skip(start)
        .find_map(|l| l.strip_prefix("# ").map(|t| t.trim().to_string()))
}

/// Header properties between the H1 and the first `## ` heading.
pub fn header_props(text: &str) -> Vec<(String, String)> {
    header_lines(text).into_iter().map(|(_, k, v, _)| (k, v)).collect()
}

/// `(line index, key, value, bold)` of each header property line.
pub fn header_lines(text: &str) -> Vec<(usize, String, String, bool)> {
    let (_, start) = front_matter(text);
    let mut out = Vec::new();
    let mut in_fence = false;
    for (i, line) in text.lines().enumerate().skip(start) {
        let t = line.trim_end();
        if t.starts_with("```") || t.starts_with("~~~") {
            in_fence = !in_fence;
        }
        if in_fence {
            continue;
        }
        if t.starts_with("## ") {
            break;
        }
        if let Some(c) = BOLD_PROP.captures(t) {
            out.push((i, c[1].trim().to_string(), c[2].trim().to_string(), true));
        } else if let Some(c) = PLAIN_PROP.captures(t)
            && c[1].split_whitespace().count() <= 4
        {
            out.push((i, c[1].trim().to_string(), c[2].trim().to_string(), false));
        }
    }
    out
}

/// Keys the side panel shows, so the body drops their lines.
pub const PANEL_KEYS: [&str; 16] = [
    "status", "type", "blocked by", "tranche", "spec", "label", "labels", "priority",
    "assignee", "estimate", "due", "claimed by", "claimed at", "parent", "prd", "map",
];

pub fn parse_ticket(path: &Path, text: &str) -> Ticket {
    let fname = path.file_name().unwrap_or_default().to_string_lossy().to_string();
    let (num, slug) = match NUM_FILE.captures(&fname) {
        Some(c) => (c[1].parse().ok(), c[2].to_string()),
        None => (None, fname.trim_end_matches(".md").to_string()),
    };
    let (fm, body_start) = front_matter(text);
    let mut props: Vec<(String, String)> = fm
        .iter()
        .filter(|(k, _)| !k.eq_ignore_ascii_case("title"))
        .map(|(k, v)| (humanize(k), v.clone()))
        .collect();
    let header = header_lines(text);
    for (_, k, v, _) in &header {
        if !props.iter().any(|(pk, _)| pk.eq_ignore_ascii_case(k)) {
            props.push((k.clone(), v.clone()));
        }
    }
    let get = |key: &str| -> Option<String> {
        props
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(key) || normalize(k) == normalize(key))
            .map(|(_, v)| v.clone())
            .filter(|v| !v.trim().is_empty())
    };

    let title = fm
        .get("title")
        .cloned()
        .or_else(|| first_h1(text))
        .unwrap_or_else(|| humanize(&slug));
    // `# 12: Title` repeats the number the key already shows.
    let title = match TITLE_NUM.captures(&title) {
        Some(c) if num.is_some() && c[1].parse::<u32>().ok() == num => c[2].trim().to_string(),
        _ => title,
    };

    let lines: Vec<&str> = text.lines().collect();
    let mut sections = Vec::new();
    let mut checklist = Vec::new();
    let mut in_fence = false;
    let mut comments = 0usize;
    let mut in_comments = false;
    let mut answer_body = false;
    let mut in_answer = false;
    let mut in_blocked_section = false;
    let mut blocked_section: Vec<String> = Vec::new();
    for (i, line) in lines.iter().enumerate() {
        if line.trim_start().starts_with("```") || line.trim_start().starts_with("~~~") {
            in_fence = !in_fence;
            continue;
        }
        if in_fence {
            continue;
        }
        if let Some(h) = line.strip_prefix("## ") {
            let h = h.trim().to_string();
            in_comments = h.eq_ignore_ascii_case("comments");
            in_answer = h.eq_ignore_ascii_case("answer");
            in_blocked_section = h.eq_ignore_ascii_case("blocked by");
            sections.push(h);
            continue;
        }
        if in_answer && !line.trim().is_empty() {
            answer_body = true;
        }
        if in_comments
            && (line.starts_with("### ") || line.starts_with("- ") || line.starts_with("* ") || line.starts_with("**"))
        {
            comments += 1;
        }
        if in_blocked_section {
            let item = line.trim().trim_start_matches(['-', '*']).trim();
            if !item.is_empty() {
                blocked_section.push(item.to_string());
            }
        }
        if let Some(c) = CHECK.captures(line) {
            checklist.push(CheckItem {
                done: &c[2] != " ",
                text: c[3].trim().to_string(),
                line: i,
                depth: c[1].len() / 2,
            });
        }
    }

    let claimed_by = get("claimed by").or_else(|| get("claimed_by"));
    let (status, status_derived) = match get("status") {
        // `done (merged #12)` or `resolved - see answer` is still `done`.
        Some(s) => (clean_status(&s), false),
        None if answer_body => ("resolved".to_string(), true),
        None if sections.iter().any(|s| s.eq_ignore_ascii_case("ruled out")) => {
            ("out-of-scope".to_string(), true)
        }
        None if claimed_by.is_some() => ("claimed".to_string(), true),
        None => ("open".to_string(), true),
    };
    let category = categorize(&status);

    let dir = path.parent().unwrap_or(Path::new("."));
    let blocked_raw = get("blocked by")
        .or_else(|| get("blocked_by"))
        .or_else(|| get("depends on"))
        .or_else(|| (!blocked_section.is_empty()).then(|| blocked_section.join(", ")));
    let blocked_refs = blocked_raw
        .as_deref()
        .map(|v| parse_refs(v, dir))
        .unwrap_or_default();

    let mut links = Vec::new();
    for c in MD_LINK.captures_iter(text) {
        let target = &c[2];
        if target.contains("://") {
            continue;
        }
        let p = dir.join(target);
        if !links.contains(&p) {
            links.push(p);
        }
    }
    let spec_link = get("spec")
        .or_else(|| get("prd"))
        .or_else(|| get("map"))
        .and_then(|v| MD_LINK.captures(&v).map(|c| dir.join(&c[2])));

    let header_ix: Vec<usize> = header
        .iter()
        .filter(|(_, k, _, _)| PANEL_KEYS.contains(&k.to_lowercase().as_str()))
        .map(|(i, _, _, _)| *i)
        .collect();
    let mut body = String::new();
    let mut skipped_h1 = false;
    let mut prev_blank = true;
    for (i, line) in lines.iter().enumerate().skip(body_start) {
        if !skipped_h1 && line.starts_with("# ") {
            skipped_h1 = true;
            continue;
        }
        if header_ix.contains(&i) {
            continue;
        }
        let blank = line.trim().is_empty();
        if blank && prev_blank {
            continue;
        }
        body.push_str(line);
        body.push('\n');
        prev_blank = blank;
    }

    let labels = get("labels")
        .or_else(|| get("label"))
        .or_else(|| get("tags"))
        .map(|v| {
            v.split(',')
                .map(|s| s.trim().trim_matches('`').to_string())
                .filter(|s| !s.is_empty())
                .collect()
        })
        .unwrap_or_default();

    Ticket {
        path: path.to_path_buf(),
        rel: String::new(),
        project: 0,
        num,
        slug,
        key: String::new(),
        title,
        status_key: normalize(&status),
        status,
        status_derived,
        category,
        kind: get("type").map(|s| s.to_lowercase()),
        props: props.clone(),
        blocked_refs,
        blocked_by: Vec::new(),
        blocks: Vec::new(),
        related: Vec::new(),
        links,
        checklist,
        comments,
        body: body.trim().to_string(),
        raw: text.to_string(),
        words: text.split_whitespace().count(),
        bytes: text.len() as u64,
        modified: None,
        created: None,
        labels,
        assignee: get("assignee").or(claimed_by),
        needs_human: get("needs a human"),
        agent: None,
        inferred: None,
        spec_link,
        ruled_out: false,
    }
}

/// `01 (Repo toolchain), 03; [x](../issues/04-y.md)` → refs.
pub fn parse_refs(value: &str, dir: &Path) -> Vec<RefSpec> {
    let v = value.trim();
    let lower = v.to_lowercase();
    if lower.starts_with("none") || lower == "-" || lower == "n/a" || lower.is_empty() {
        return Vec::new();
    }
    let mut parts = Vec::new();
    let mut depth = 0i32;
    let mut cur = String::new();
    for ch in v.chars() {
        match ch {
            '(' | '[' => depth += 1,
            ')' | ']' => depth -= 1,
            _ => {}
        }
        if (ch == ',' || ch == ';') && depth <= 0 {
            parts.push(std::mem::take(&mut cur));
        } else {
            cur.push(ch);
        }
    }
    parts.push(cur);
    parts
        .into_iter()
        .map(|p| p.trim().to_string())
        .filter(|p| !p.is_empty())
        .flat_map(|p| {
            // `01 and 02` / `01, 02` written without commas.
            if p.contains(" and ") && !p.contains('(') {
                p.split(" and ").map(str::to_string).collect::<Vec<_>>()
            } else {
                vec![p]
            }
        })
        .map(|p| {
            let p = p.trim().to_string();
            let path = MD_LINK.captures(&p).map(|c| dir.join(&c[2]));
            let num = path
                .as_ref()
                .and_then(|pp| pp.file_name())
                .and_then(|f| NUM_FILE.captures(&f.to_string_lossy()).and_then(|c| c[1].parse().ok()))
                .or_else(|| {
                    let s = p.trim_start_matches(['*', '`', '[']);
                    LEAD_NUM.captures(s).and_then(|c| c[1].parse().ok())
                });
            RefSpec { num, path, text: p }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_repo(name: &str, tracker: Option<&str>, scratch: bool) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("kuzgun-test-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("docs/agents")).unwrap();
        if let Some(t) = tracker {
            std::fs::write(dir.join("docs/agents/issue-tracker.md"), t).unwrap();
        }
        if scratch {
            std::fs::create_dir_all(dir.join(".scratch")).unwrap();
        }
        dir
    }

    #[test]
    fn cleans_status_notes() {
        assert_eq!(clean_status("done (merged #12)"), "done");
        assert_eq!(clean_status("`resolved` - see answer"), "resolved");
        assert_eq!(clean_status("in progress"), "in progress");
    }

    #[test]
    fn loads_only_real_tickets() {
        let root = temp_repo("strict", None, false);
        let w = |rel: &str, body: &str| {
            let p = root.join(rel);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, body).unwrap();
        };
        w("feat/spec.md", "# Feature\n");
        w("feat/issues/01-first.md", "# First\n\nStatus: ready-for-agent\n");
        w("feat/issues/02-second.md", "# Second\n\n## Blocked by\n\n- First\n");
        w("plans/2026-05-21-brand.md", "# Plan\n\nStatus: draft\n");
        w("adr/0001-choose-db.md", "# ADR\n\nStatus: accepted\n");
        w("research/note.md", "# Note\n\nStatus: beta\n");
        let b = Board::load(&root);
        let titles: Vec<&str> = b.tickets.iter().map(|t| t.title.as_str()).collect();
        assert_eq!(titles, vec!["First", "Second"]);
        assert_eq!(b.tickets[1].blocked_by, vec![0]);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn locates_trackers() {
        let local = temp_repo("local", Some("# Issue tracker: Local Markdown\n\nIssues live as markdown files in `.scratch/`.\n"), true);
        assert_eq!(locate_tracker(&local), Tracker::Local(local.join(".scratch")));

        let github = temp_repo("github", Some("# Issue tracker: GitHub\n\nIssues live as GitHub issues.\n"), true);
        assert_eq!(locate_tracker(&github), Tracker::Remote("GitHub".into()));

        let bare = temp_repo("bare", None, true);
        assert_eq!(locate_tracker(&bare), Tracker::Local(bare.join(".scratch")));

        let plain = temp_repo("plain", None, false);
        assert_eq!(locate_tracker(&plain), Tracker::Missing);

        let picked = local.join(".scratch");
        assert_eq!(locate_tracker(&picked), Tracker::Local(picked.clone()));
        for d in [local, github, bare, plain] {
            let _ = std::fs::remove_dir_all(d);
        }
    }

    const LOCAL: &str = "# 100: Design system lint\n\n**Spec:** [Walking skeleton](../spec.md)\n\n**What to build:** oxlint checks classes.\n\n**Blocked by:** 01 (Repo toolchain and local stack), 03\n\n**Status:** done\n\n**Tranche:** T1\n\n- [x] Add lint.\n- [ ] Turn on rules.\n\n## Comments\n";

    #[test]
    fn parses_local_ticket() {
        let t = parse_ticket(Path::new("/b/ws/issues/100-design-system-lint.md"), LOCAL);
        assert_eq!(t.num, Some(100));
        assert_eq!(t.title, "Design system lint");
        assert_eq!(t.status, "done");
        assert_eq!(t.category, Category::Done);
        assert_eq!(t.field_values("Tranche"), vec!["T1"]);
        assert_eq!(t.checklist_counts(), (1, 2));
        let nums: Vec<_> = t.blocked_refs.iter().map(|r| r.num).collect();
        assert_eq!(nums, vec![Some(1), Some(3)]);
        assert!(t.body.contains("What to build"));
        assert!(!t.body.contains("**Status:**"));
        assert_eq!(t.spec_link, Some(PathBuf::from("/b/ws/issues/../spec.md")));
    }

    #[test]
    fn parses_wayfinder_ticket() {
        let text = "# Paddle\n\nType: research\nStatus: resolved\n\n## Question\n\nQ\n\n## Answer\n\nA\n";
        let t = parse_ticket(Path::new("/b/p/issues/01-paddle.md"), text);
        assert_eq!(t.kind.as_deref(), Some("research"));
        assert_eq!(t.category, Category::Done);
        let derived = parse_ticket(
            Path::new("/b/p/issues/02-x.md"),
            "# X\n\nType: task\n\n## Answer\n\nyes\n",
        );
        assert!(derived.status_derived);
        assert_eq!(derived.status, "resolved");
    }

    #[test]
    fn front_matter_status() {
        let text = "---\ntitle: Hello\nstatus: in-progress\nlabels: [a, b]\n---\n\nBody\n";
        let t = parse_ticket(Path::new("/b/hello.md"), text);
        assert_eq!(t.title, "Hello");
        assert_eq!(t.category, Category::InProgress);
        assert_eq!(t.labels, vec!["a", "b"]);
    }

    #[test]
    fn categories() {
        assert_eq!(categorize("ready-for-agent"), Category::Todo);
        assert_eq!(categorize("Needs Triage"), Category::Backlog);
        assert_eq!(categorize("claimed"), Category::InProgress);
        assert_eq!(categorize("in-review"), Category::InReview);
        assert_eq!(categorize("wontfix"), Category::Canceled);
    }

    #[test]
    fn refs() {
        let r = parse_refs("None — can start immediately", Path::new("/"));
        assert!(r.is_empty());
        let r = parse_refs("02, 03 (Sign in (server))", Path::new("/"));
        assert_eq!(r.len(), 2);
        assert_eq!(r[1].num, Some(3));
    }

    #[test]
    fn map_sections() {
        let text = "# Map\n\n## Destination\n\nShip it.\n\n## Decisions so far\n\n- [A](issues/01-a.md): yes\n\n## Not yet specified\n\n- **Billing**: later\n- Search\n\n## Out of scope\n\n- Mobile\n";
        let m = parse_map(Path::new("/b/map.md"), text);
        assert_eq!(m.destination, "Ship it.");
        assert_eq!(m.decisions, 1);
        assert_eq!(m.fog, vec!["**Billing**: later", "Search"]);
        assert_eq!(m.out_of_scope, vec!["Mobile"]);
    }

    #[test]
    fn facets_skip_prose() {
        let a = parse_ticket(Path::new("/b/p/issues/01-a.md"), "# A\n\n**Tranche:** T1\n\n**What to build:** a long text\n\n**Status:** open\n");
        let b = parse_ticket(Path::new("/b/p/issues/02-b.md"), "# B\n\n**Tranche:** T2\n\n**What to build:** more\n\n**Status:** done\n");
        let f = find_facets(&[a, b]);
        assert_eq!(f.len(), 1);
        assert_eq!(f[0].key, "Tranche");
        assert_eq!(f[0].values, vec!["T1", "T2"]);
    }
}
