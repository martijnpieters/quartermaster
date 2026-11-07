use std::fs::{self, OpenOptions};
use std::os::unix::io::AsRawFd;
use std::path::PathBuf;

use serde_yaml::{Mapping, Value};

use crate::clock;
use crate::data_root;
use crate::error::{QmError, Res, UNEMITTABLE};
use crate::front_edit;
use crate::note::{Note, NOTE_KINDS};
use crate::psych;
use crate::sections;
use crate::ruby_text::strip;
use crate::yaml_scalar::{array, get, text};

pub const STATES: [&str; 3] = ["in_progress", "fridge", "resolved"];
pub const INJECTED_NOTES: usize = 5;
pub const INJECTED_OPEN_NOTES: usize = 3;

/// What `snooze` did.
pub enum Snooze {
    Cleared,
    Turn,
    Until(String),
}

pub struct WorkItem {
    pub slug: String,
    front: Mapping,
    body: String,
    front_text: String,
}

fn split(slug: &str, contents: &str) -> Res<(String, String)> {
    let malformed = || QmError::argument(format!("malformed work item file {}", WorkItem::path_for(slug).display()));
    let rest = contents.strip_prefix("---\n").ok_or_else(malformed)?;
    let at = rest.find("\n---\n").ok_or_else(malformed)?;
    Ok((rest[..at].to_string(), rest[at + 5..].to_string()))
}

fn parse_front(front_text: &str) -> Res<Mapping> {
    match serde_yaml::from_str::<Value>(front_text)? {
        Value::Mapping(m) => Ok(m),
        _ => Ok(Mapping::new()),
    }
}

impl WorkItem {
    pub fn path_for(slug: &str) -> PathBuf {
        data_root::work_items_dir().join(format!("{slug}.md"))
    }

    pub fn find(slug: &str) -> Res<WorkItem> {
        let path = Self::path_for(slug);
        let contents = fs::read_to_string(&path).map_err(|_| QmError::argument(format!("no work item '{slug}'")))?;
        let (front_text, body) = split(slug, &contents)?;
        Ok(WorkItem { slug: slug.to_string(), front: parse_front(&front_text)?, body, front_text })
    }

    /// Every item, in file-name order.
    pub fn all() -> Res<Vec<WorkItem>> {
        let Ok(dir) = fs::read_dir(data_root::work_items_dir()) else { return Ok(vec![]) };
        let mut names: Vec<String> = dir
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n.ends_with(".md") && !n.starts_with('.'))
            .collect();
        names.sort();
        names.iter().map(|n| Self::find(n.trim_end_matches(".md"))).collect()
    }

    fn field(&self, key: &str) -> Option<String> {
        get(&self.front, key).map(text)
    }

    pub fn title(&self) -> String {
        self.field("title").unwrap_or_default()
    }
    pub fn state(&self) -> Option<String> {
        self.field("state")
    }
    pub fn project(&self) -> Option<String> {
        self.field("project")
    }
    pub fn created(&self) -> Option<String> {
        self.field("created")
    }
    pub fn board(&self) -> Option<String> {
        self.field("board")
    }
    pub fn snooze_until_raw(&self) -> Option<String> {
        self.field("snooze_until")
    }

    /// The number of the first branch named `issue/<number>…`, if any.
    pub fn issue_number(&self) -> Option<String> {
        let issue = regex::Regex::new(r"issue/([0-9]+)").unwrap();
        self.branches().iter().find_map(|b| issue.captures(b).map(|c| c[1].to_string()))
    }

    /// The session was handed over (wrapped up or distilled): its context is spent.
    pub fn rotated_from(&self, session_id: &str) -> bool {
        array(get(&self.front, "rotations")).iter().any(|r| match r {
            Value::Mapping(m) => get(m, "from").map(text).is_some_and(|f| f == session_id),
            _ => false,
        })
    }

    /// Whether the chat's current your-turn (started at `since`) was snoozed: a snooze stamp at or after it.
    pub fn turn_snoozed(&self, since: Option<f64>) -> bool {
        let (Some(stored), Some(since)) = (self.field("turn_snoozed_at"), since) else { return false };
        crate::clock::parse_f64(&stored).is_some_and(|at| at >= since)
    }

    /// The one note to show when only one fits: the newest open one, else the newest that is not history.
    pub fn headline_note(&self) -> Option<Note> {
        let all = self.notes();
        all.iter().find(|n| n.is_open()).or_else(|| all.iter().find(|n| !n.is_history())).or_else(|| all.first()).cloned()
    }

    /// The body with each stored note line rendered by `f` (the position and kind handles `qm retag` takes).
    pub fn body_with_notes(&self, f: impl FnMut(Note) -> String) -> String {
        sections::map_notes(&self.body, f)
    }

    /// `sessions.size`: the entries of the item's `sessions` list.
    pub fn sessions_count(&self) -> usize {
        array(get(&self.front, "sessions")).len()
    }

    /// The rotation entries as the front matter holds them.
    pub fn rotations(&self) -> Vec<Value> {
        array(get(&self.front, "rotations")).into_iter().cloned().collect()
    }

    /// How many times the item's chat was handed over.
    pub fn rotation_count(&self) -> usize {
        array(get(&self.front, "rotations")).len()
    }

    /// The item's dev-server mode: its own, else its project's default; None without a dev server.
    pub fn pf_mode(&self) -> Option<String> {
        let config = crate::project_config::ProjectConfig::current();
        let server = config.settings(self.project().as_deref())?.dev_server.as_ref()?;
        Some(self.pf_mode_override().unwrap_or_else(|| server.default_mode.clone()))
    }

    /// The `## Snapshots` section as `{stamp, session, text}`, newest first.
    pub fn snapshots(&self) -> Vec<serde_json::Value> {
        let section = self.section_body("## Snapshots");
        let split = regex::Regex::new(r"(?m)^### ").unwrap();
        let header = regex::Regex::new(r"\A([0-9]{4}-[0-9]{2}-[0-9]{2}) — session ([^ \t\r\n\x0c\x0b]+)").unwrap();
        let mut out: Vec<serde_json::Value> = vec![];
        let mut start = 0;
        let mut chunks: Vec<&str> = vec![];
        for m in split.find_iter(&section) {
            chunks.push(&section[start..m.start()]);
            start = m.end();
        }
        chunks.push(&section[start..]);
        for chunk in chunks {
            if strip(chunk).is_empty() {
                continue;
            }
            let mut lines = chunk.split_inclusive('\n');
            let head = lines.next().unwrap_or("");
            let rest: String = lines.collect();
            let meta = header.captures(head);
            out.push(serde_json::json!({
                "stamp": meta.as_ref().map(|m| m[1].to_string()),
                "session": meta.as_ref().map(|m| m[2].to_string()),
                "text": strip(&rest),
            }));
        }
        out.reverse();
        out
    }

    pub fn pf_mode_override(&self) -> Option<String> {
        self.field("pf_mode")
    }

    pub fn session_ids(&self) -> Vec<String> {
        array(get(&self.front, "sessions"))
            .into_iter()
            .filter_map(|s| match s {
                Value::Mapping(m) => get(m, "id").map(text),
                _ => None,
            })
            .collect()
    }

    pub fn session_count(&self) -> usize {
        array(get(&self.front, "sessions")).len()
    }

    pub fn branches(&self) -> Vec<String> {
        let v = get(&self.front, "branches").or_else(|| get(&self.front, "branch"));
        array(v).into_iter().map(text).collect()
    }

    pub fn temp_branches(&self) -> Vec<String> {
        array(get(&self.front, "temp_branches")).into_iter().map(text).collect()
    }

    fn section_body(&self, heading: &str) -> String {
        let lines: Vec<&str> = self.body.split_inclusive('\n').collect();
        let Some(idx) = lines.iter().position(|l| l.starts_with(heading)) else { return String::new() };
        let end = (idx + 1..lines.len()).find(|&i| lines[i].starts_with("## ")).unwrap_or(lines.len());
        lines[idx + 1..end].concat()
    }

    pub fn description(&self) -> String {
        strip(&self.section_body("## Description")).to_string()
    }

    /// Newest first; `position` is the 1-based place in the file.
    pub fn notes(&self) -> Vec<Note> {
        let mut stored = vec![];
        for line in self.section_body("## Notes").split_inclusive('\n') {
            if let Some(note) = Note::parse(line, stored.len() + 1) {
                stored.push(note);
            }
        }
        stored.reverse();
        stored
    }

    /// Every open note (newest few) plus the newest non-history notes, returned oldest first.
    pub fn injectable_notes(&self) -> Vec<Note> {
        let carried: Vec<Note> = self.notes().into_iter().filter(|n| !n.is_history()).collect();
        let open: Vec<usize> = (0..carried.len()).filter(|&i| carried[i].is_open()).take(INJECTED_OPEN_NOTES).collect();
        let recent: Vec<usize> = (0..carried.len()).take(INJECTED_NOTES).collect();
        let mut chosen: Vec<usize> = open;
        for i in recent {
            if !chosen.contains(&i) {
                chosen.push(i);
            }
        }
        chosen.sort();
        chosen.reverse();
        chosen.into_iter().map(|i| carried[i].clone()).collect()
    }

    /// The `## Snapshots` section cut at its `### ` headings, blank pieces dropped.
    fn snapshot_pieces(&self) -> Vec<String> {
        let section = self.section_body("## Snapshots");
        let mut pieces: Vec<String> = vec![String::new()];
        for line in section.split_inclusive('\n') {
            if let Some(rest) = line.strip_prefix("### ") {
                pieces.push(rest.to_string());
            } else {
                pieces.last_mut().unwrap().push_str(line);
            }
        }
        pieces.into_iter().filter(|p| !strip(p).is_empty()).collect()
    }

    pub fn snapshot_count(&self) -> usize {
        self.snapshot_pieces().len()
    }

    /// Whether the session was distilled: a snapshot headed `YYYY-MM-DD — session <id>` exists.
    pub fn snapshot_for(&self, session_id: &str) -> bool {
        self.snapshot_pieces().iter().any(|piece| {
            let header = piece.lines().next().unwrap_or("");
            let dated = header.get(..10).is_some_and(|d| {
                d.bytes().enumerate().all(|(i, c)| if i == 4 || i == 7 { c == b'-' } else { c.is_ascii_digit() })
            });
            dated
                && header[10..]
                    .strip_prefix(" — session ")
                    .and_then(|rest| rest.split_whitespace().next())
                    .is_some_and(|id| id == session_id)
        })
    }

    /// The attached sessions with whether each has had its transcript degradation acknowledged.
    pub fn attached_sessions(&self) -> Vec<(String, bool)> {
        array(get(&self.front, "sessions"))
            .into_iter()
            .filter_map(|s| match s {
                Value::Mapping(m) => get(m, "id").map(|id| (text(id), !matches!(m.get("ack_degrade"), None | Some(Value::Null) | Some(Value::Bool(false))))),
                _ => None,
            })
            .collect()
    }

    /// The item file's modification time, in seconds since the epoch.
    pub fn file_mtime(&self) -> Option<f64> {
        let modified = fs::metadata(Self::path_for(&self.slug)).and_then(|m| m.modified()).ok()?;
        Some(match modified.duration_since(std::time::UNIX_EPOCH) {
            Ok(d) => d.as_secs_f64(),
            Err(e) => -e.duration().as_secs_f64(),
        })
    }

    /// Whether the item is snoozed through `date`; an unreadable `snooze_until` snoozes nothing.
    pub fn snoozed_at(&self, date: chrono::NaiveDate) -> bool {
        let Some(stored) = self.field("snooze_until") else { return false };
        stored.get(..10).and_then(|d| chrono::NaiveDate::parse_from_str(d, "%Y-%m-%d").ok()).is_some_and(|until| until >= date)
    }

    // ---- writes

    /// Runs `f` under the global work-item flock, as the Ruby `WorkItem.with_lock`: one lock spans a
    /// whole read-modify-write, and it is reentrant on this thread so compound moves nest.
    fn with_lock<T>(f: impl FnOnce() -> Res<T>) -> Res<T> {
        thread_local! {
            static HELD: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
        }
        struct Release;
        impl Drop for Release {
            fn drop(&mut self) {
                HELD.with(|h| h.set(false));
            }
        }
        if HELD.with(|h| h.get()) {
            return f();
        }
        data_root::ensure_setup()?;
        let lock = OpenOptions::new().create(true).truncate(false).write(true).open(data_root::work_items_dir().join(".lock"))?;
        // SAFETY: the descriptor stays open until `lock` drops after `f` returns.
        unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX) };
        HELD.with(|h| h.set(true));
        let _release = Release;
        f()
    }

    fn write_file(slug: &str, contents: &str) -> Res<()> {
        let path = Self::path_for(slug);
        let temp = path.with_file_name(format!(".{slug}.md.tmp-{}", std::process::id()));
        fs::write(&temp, contents)?;
        fs::rename(&temp, &path)?;
        Ok(())
    }

    /// Re-reads the file under the lock, lets `edit` change the front lines and body (returning
    /// whether to save), validates the result parses, and writes it.
    fn edit(&mut self, edit: impl FnOnce(&WorkItem, &mut Vec<String>, &mut String) -> Res<bool>) -> Res<bool> {
        Self::with_lock(|| {
            let fresh = Self::find(&self.slug)?;
            let mut lines: Vec<String> = fresh.front_text.lines().map(String::from).collect();
            let mut body = fresh.body.clone();
            if !edit(&fresh, &mut lines, &mut body)? {
                *self = fresh;
                return Ok(false);
            }
            let front_text = lines.join("\n");
            let front = parse_front(&front_text)
                .map_err(|e| QmError::new("RuntimeError", format!("refusing to write unparsable front matter: {e}")))?;
            Self::write_file(&self.slug, &format!("---\n{front_text}\n---\n{body}"))?;
            *self = WorkItem { slug: fresh.slug, front, body, front_text };
            Ok(true)
        })
    }

    pub fn slugify(title: &str) -> String {
        let mut slug = String::new();
        let mut dash = false;
        for c in title.to_lowercase().chars() {
            if c.is_ascii_lowercase() || c.is_ascii_digit() {
                if dash && !slug.is_empty() {
                    slug.push('-');
                }
                dash = false;
                slug.push(c);
            } else {
                dash = true;
            }
        }
        slug
    }

    /// A new item; `Unemittable` when its front matter is something only Ruby writes faithfully.
    pub fn create(title: &str, project: &str, branch: Option<&str>, session_id: Option<&str>, description: Option<&str>, pf_mode: Option<&str>) -> Res<WorkItem> {
        let slug = Self::slugify(title);
        let unemittable = |what: &str| QmError::new(UNEMITTABLE, format!("cannot write {what} like Ruby"));
        if slug.is_empty() {
            return Err(unemittable("an empty slug"));
        }
        let mut lines = vec!["v: 1".to_string()];
        lines.extend(psych::pair("title", title).ok_or_else(|| unemittable("the title"))?);
        lines.extend(psych::pair("state", "in_progress").unwrap());
        lines.extend(psych::pair("project", project).ok_or_else(|| unemittable("the project"))?);
        lines.extend(psych::pair("created", &clock::today()).unwrap());
        if let Some(b) = branch {
            lines.extend(psych::list("branches", &[b.to_string()]).ok_or_else(|| unemittable("the branch"))?);
        }
        if let Some(id) = session_id {
            let text = psych::scalar(6, id).ok_or_else(|| unemittable("the session id"))?;
            lines.push("sessions:".to_string());
            lines.push(format!("- id: {text}"));
        }
        if let Some(mode) = pf_mode {
            lines.extend(psych::pair("pf_mode", mode).ok_or_else(|| unemittable("the dev-server mode"))?);
        }
        let body = format!("\n## Description\n\n{}\n\n## Notes\n\n## Snapshots\n", description.unwrap_or("_TODO_"));
        Self::with_lock(|| {
            if Self::path_for(&slug).exists() {
                return Err(QmError::argument(format!("work item '{slug}' already exists")));
            }
            if let Some(mode) = pf_mode {
                Self::validate_pf_mode(Some(project), mode)?;
            }
            let front_text = lines.join("\n");
            Self::write_file(&slug, &format!("---\n{front_text}\n---\n{body}"))?;
            Self::find(&slug)
        })
    }

    /// The project's dev server serves one checkout in one of its modes; the item remembers which.
    fn validate_pf_mode(project: Option<&str>, mode: &str) -> Res<()> {
        let config = crate::project_config::ProjectConfig::current();
        let Some(server) = config.settings(project).and_then(|s| s.dev_server.as_ref()) else {
            let project = project.unwrap_or("");
            return Err(QmError::argument(format!("{project} has no dev server: set projects.{project}.dev_server in config.yml")));
        };
        if server.modes.iter().any(|m| m == mode) {
            Ok(())
        } else {
            Err(QmError::argument(format!("{} mode must be one of: {}", server.command, server.modes.join(", "))))
        }
    }

    /// Attaching is a move: claim the session for this item and vacate it everywhere else. Returns
    /// the slugs it left.
    pub fn claim_session(&mut self, session_id: &str) -> Res<Vec<String>> {
        Self::with_lock(|| {
            self.attach_session(session_id)?;
            Self::detach_session_elsewhere(session_id, &self.slug.clone())
        })
    }

    /// A session belongs to one work item: detaches it from every item but `keep`.
    pub fn detach_session_elsewhere(session_id: &str, keep: &str) -> Res<Vec<String>> {
        let mut left = vec![];
        for mut item in Self::all()? {
            if item.slug != keep && item.detach_session(session_id)? {
                left.push(item.slug.clone());
            }
        }
        Ok(left)
    }

    pub fn attach_session(&mut self, session_id: &str) -> Res<bool> {
        let id = session_id.to_string();
        self.edit(move |fresh, lines, _| {
            if fresh.session_ids().contains(&id) {
                return Ok(false);
            }
            let text = psych::scalar(6, &id).ok_or_else(|| QmError::new(UNEMITTABLE, "session id"))?;
            front_edit::push_item(lines, "sessions", vec![format!("- id: {text}")])?;
            Ok(true)
        })
    }

    pub fn detach_session(&mut self, session_id: &str) -> Res<bool> {
        let id = session_id.to_string();
        self.edit(move |fresh, lines, _| {
            if !fresh.session_ids().contains(&id) {
                return Ok(false);
            }
            let (at, end) = front_edit::block(lines, "sessions").expect("sessions present");
            if lines[at] != "sessions:" {
                return Err(QmError::new("RuntimeError", "front matter 'sessions' is not a block list; edit it with the Ruby qm"));
            }
            let mut keep: Vec<String> = vec![];
            for (s, e) in front_edit::items(lines, at, end) {
                let entry = lines[s..e].join("\n");
                let listed = serde_yaml::from_str::<Value>(&entry).ok().and_then(|v| match v {
                    Value::Sequence(items) => items.first().and_then(|i| i.as_mapping().and_then(|m| get(m, "id").map(text))),
                    _ => None,
                });
                if listed.as_deref() != Some(id.as_str()) {
                    keep.extend(lines[s..e].iter().cloned());
                }
            }
            let replacement = if keep.is_empty() { vec!["sessions: []".to_string()] } else { [vec!["sessions:".to_string()], keep].concat() };
            lines.splice(at..end, replacement);
            Ok(true)
        })
    }

    fn set_list(lines: &mut Vec<String>, key: &str, items: &[String]) -> Res<()> {
        let emitted = psych::list(key, items).ok_or_else(|| QmError::new(UNEMITTABLE, format!("the {key} list")))?;
        front_edit::set(lines, key, emitted);
        Ok(())
    }

    fn set_pair(lines: &mut Vec<String>, key: &str, value: &str) -> Res<()> {
        let emitted = psych::pair(key, value).ok_or_else(|| QmError::new(UNEMITTABLE, format!("the {key} value")))?;
        front_edit::set(lines, key, emitted);
        Ok(())
    }

    /// Records a scratch branch (a `claude --worktree` session's); a branch linked for real stays only that.
    pub fn add_temp_branch(&mut self, branch: &str) -> Res<bool> {
        let branch = branch.to_string();
        self.edit(move |fresh, lines, _| {
            if fresh.branches().contains(&branch) || fresh.temp_branches().contains(&branch) {
                return Ok(false);
            }
            let mut all = fresh.temp_branches();
            all.push(branch.clone());
            Self::set_list(lines, "temp_branches", &all)?;
            Ok(true)
        })
    }

    /// Links branches the item lacks (a stack's others); returns those added, none when all were there.
    pub fn link_branches(&mut self, list: &[String]) -> Res<Vec<String>> {
        let list = list.to_vec();
        let mut added = vec![];
        self.edit(|fresh, lines, _| {
            let mut branches = fresh.branches();
            for b in &list {
                if !branches.contains(b) && !added.contains(b) {
                    added.push(b.clone());
                }
            }
            if added.is_empty() {
                return Ok(false);
            }
            branches.extend(added.iter().cloned());
            Self::store_branches(lines, &branches)?;
            Ok(true)
        })?;
        Ok(added)
    }

    pub fn update_state(&mut self, state: &str) -> Res<()> {
        if !STATES.contains(&state) {
            return Err(QmError::argument(format!("state must be one of: {}", STATES.join(", "))));
        }
        self.edit(|_, lines, _| Self::set_pair(lines, "state", state).map(|_| true))?;
        if state == "resolved" {
            self.clear_viewed_marks()?;
        }
        Ok(())
    }

    /// Resolving drops the diff pane's viewed marks (viewed/<slug>.json).
    fn clear_viewed_marks(&self) -> Res<()> {
        if !crate::cleanup_command::valid_slug(&self.slug) {
            return Err(QmError::argument(format!("'{}' is not a work item slug", self.slug)));
        }
        let _ = fs::remove_file(data_root::root().join("viewed").join(format!("{}.json", self.slug)));
        Ok(())
    }

    pub fn update_pf_mode(&mut self, mode: &str) -> Res<()> {
        Self::validate_pf_mode(self.project().as_deref(), mode)?;
        self.edit(|_, lines, _| Self::set_pair(lines, "pf_mode", mode).map(|_| true)).map(|_| ())
    }

    pub fn move_to(&mut self, project: &str) -> Res<()> {
        self.edit(|_, lines, _| Self::set_pair(lines, "project", project).map(|_| true)).map(|_| ())
    }

    pub fn retitle(&mut self, title: &str) -> Res<()> {
        self.edit(|_, lines, _| Self::set_pair(lines, "title", title).map(|_| true)).map(|_| ())
    }

    /// A deleted temp branch has nothing left to point at.
    pub fn unlink_temp_branch(&mut self, branch: &str) -> Res<()> {
        let branch = branch.to_string();
        self.edit(move |fresh, lines, _| {
            let temps: Vec<String> = fresh.temp_branches().into_iter().filter(|b| *b != branch).collect();
            if temps.is_empty() {
                front_edit::delete(lines, "temp_branches");
            } else {
                Self::set_list(lines, "temp_branches", &temps)?;
            }
            Ok(true)
        })
        .map(|_| ())
    }

    fn store_branches(lines: &mut Vec<String>, list: &[String]) -> Res<()> {
        front_edit::delete(lines, "branch");
        Self::set_list(lines, "branches", list)
    }

    /// Links a branch for real: a temp branch of that name is promoted.
    pub fn link_branch(&mut self, branch: &str) -> Res<()> {
        let branch = branch.to_string();
        self.edit(move |fresh, lines, _| {
            let temps: Vec<String> = fresh.temp_branches().into_iter().filter(|b| *b != branch).collect();
            if temps.is_empty() {
                front_edit::delete(lines, "temp_branches");
            } else {
                Self::set_list(lines, "temp_branches", &temps)?;
            }
            let mut branches = fresh.branches();
            if !branches.contains(&branch) {
                branches.push(branch.clone());
            }
            Self::store_branches(lines, &branches)?;
            Ok(true)
        })
        .map(|_| ())
    }

    pub fn unlink_branch(&mut self, branch: &str) -> Res<()> {
        let branch = branch.to_string();
        self.edit(move |fresh, lines, _| {
            let rest: Vec<String> = fresh.branches().into_iter().filter(|b| *b != branch).collect();
            Self::store_branches(lines, &rest)?;
            Ok(true)
        })
        .map(|_| ())
    }

    pub fn board_answers_dir(&self) -> Option<String> {
        self.field("board_answers_dir")
    }

    /// One status board per item; a different board drops the old answers directory.
    pub fn link_board(&mut self, url: &str, answers_dir: Option<&str>) -> Res<()> {
        let url = url.to_string();
        let dir = answers_dir.map(|d| data_root::expand_path(d).to_string_lossy().into_owned());
        self.edit(move |fresh, lines, _| {
            if fresh.board().as_deref() != Some(url.as_str()) {
                front_edit::delete(lines, "board_answers_dir");
            }
            Self::set_pair(lines, "board", &url)?;
            if let Some(d) = &dir {
                Self::set_pair(lines, "board_answers_dir", d)?;
            }
            Ok(true)
        })
        .map(|_| ())
    }

    pub fn unlink_board(&mut self) -> Res<()> {
        self.edit(|_, lines, _| {
            front_edit::delete(lines, "board");
            front_edit::delete(lines, "board_answers_dir");
            Ok(true)
        })
        .map(|_| ())
    }

    fn validate_kind(kind: &str) -> Res<()> {
        if NOTE_KINDS.contains(&kind) {
            Ok(())
        } else {
            Err(QmError::argument(format!("unknown note kind '{kind}' ({})", NOTE_KINDS.join(" | "))))
        }
    }

    pub fn add_note(&mut self, text: &str, kind: &str) -> Res<()> {
        Self::validate_kind(kind)?;
        let note = Note { stamp: clock::today(), kind: kind.to_string(), text: strip_leading_stamp(text), position: 0 };
        self.edit(move |_, _, body| {
            *body = sections::append_to_section(body, "## Notes", &format!("{}\n", note.to_line()));
            Ok(true)
        })
        .map(|_| ())
    }

    /// The one snapshot of a session, replacing an earlier one: `### <date> — session <id>` in `## Snapshots`.
    pub fn add_snapshot(&mut self, session_id: &str, text: &str) -> Res<()> {
        let (id, text) = (session_id.to_string(), strip(text).to_string());
        self.edit(move |fresh, _, body| {
            let section = fresh.section_body("## Snapshots");
            let header = format!(" — session {id}");
            let mut chunks: Vec<String> = vec![];
            for line in section.split_inclusive('\n') {
                if line.starts_with("### ") || chunks.is_empty() {
                    chunks.push(String::new());
                }
                chunks.last_mut().unwrap().push_str(line);
            }
            let is_mine = |chunk: &str| {
                let first = chunk.split_inclusive('\n').next().unwrap_or("");
                let Some(rest) = first.strip_prefix("### ") else { return false };
                let dated = rest.get(..10).is_some_and(|d| d.bytes().enumerate().all(|(i, c)| if i == 4 || i == 7 { c == b'-' } else { c.is_ascii_digit() }));
                dated && rest.get(10..).and_then(|r| r.strip_prefix(header.as_str())).is_some_and(|tail| strip(tail).is_empty())
            };
            let mut next = body.clone();
            if chunks.iter().any(|c| is_mine(c)) {
                let kept = chunks.iter().filter(|c| !is_mine(c)).cloned().collect::<String>();
                let kept = strip(&kept).to_string();
                next = sections::replace_section(&next, "## Snapshots", &if kept.is_empty() { "\n".to_string() } else { format!("\n{kept}\n") });
            }
            *body = sections::append_to_section(&next, "## Snapshots", &format!("\n### {} — session {id}\n\n{text}\n", clock::today()));
            Ok(true)
        })
        .map(|_| ())
    }

    /// Records that the chat was handed over, with the notes the fresh session is told about: the item's
    /// `rotations` list grows by `{at, from, via, carried, decisions, open}`.
    pub fn record_rotation(&mut self, from_session: &str, via: &str, at: f64) -> Res<serde_json::Value> {
        let (from, via) = (from_session.to_string(), via.to_string());
        let mut recorded = serde_json::Value::Null;
        self.edit(|fresh, lines, _| {
            let carried = fresh.injectable_notes();
            let decisions = carried.iter().filter(|n| n.kind == "decision").count();
            let open = carried.iter().filter(|n| n.is_open()).count();
            let positions: Vec<usize> = carried.iter().map(|n| n.position).collect();
            let stamp = chrono::DateTime::from_timestamp(at.floor() as i64, 0).map(|t| t.to_rfc3339_opts(chrono::SecondsFormat::Secs, true)).unwrap_or_default();
            let unemittable = |what: &str| QmError::new(UNEMITTABLE, format!("cannot write {what} like Ruby"));
            let mut entry = vec![format!("- at: {}", psych::scalar(6, &stamp).ok_or_else(|| unemittable("the time"))?)];
            entry.push(format!("  from: {}", psych::scalar(8, &from).ok_or_else(|| unemittable("the session id"))?));
            entry.push(format!("  via: {}", psych::scalar(7, &via).ok_or_else(|| unemittable("the way"))?));
            if positions.is_empty() {
                entry.push("  carried: []".to_string());
            } else {
                entry.push("  carried:".to_string());
                entry.extend(positions.iter().map(|p| format!("  - {p}")));
            }
            entry.push(format!("  decisions: {decisions}"));
            entry.push(format!("  open: {open}"));
            front_edit::push_item(lines, "rotations", entry)?;
            recorded = serde_json::json!({"at": stamp, "from": from, "via": via, "carried": positions, "decisions": decisions, "open": open});
            Ok(true)
        })?;
        Ok(recorded)
    }

    /// Retags the note at 1-based `position`; every stored note line is rewritten in its normal form.
    pub fn update_note_kind(&mut self, position: usize, kind: &str) -> Res<Note> {
        Self::validate_kind(kind)?;
        let mut retagged = None;
        self.edit(|fresh, _, body| {
            let stored = fresh.notes();
            let note = stored
                .iter()
                .find(|n| n.position == position)
                .ok_or_else(|| QmError::argument(format!("note #{position} does not exist ({} note(s))", stored.len())))?;
            let new = note.with_kind(kind);
            *body = sections::rewrite_notes(body, |n| if n.position == position { new.clone() } else { n });
            retagged = Some(new);
            Ok(true)
        })?;
        Ok(retagged.expect("set above"))
    }

    /// Marks an attached session's transcript degradation as accepted; false when it is not attached.
    pub fn ack_degrade(&mut self, session_id: &str) -> Res<bool> {
        let id = session_id.to_string();
        self.edit(move |fresh, lines, _| {
            if !fresh.session_ids().contains(&id) {
                return Ok(false);
            }
            let (at, end) = front_edit::block(lines, "sessions").expect("sessions present");
            if lines[at] != "sessions:" {
                return Err(QmError::new("RuntimeError", "front matter 'sessions' is not a block list; edit it with the Ruby qm"));
            }
            for (s, e) in front_edit::items(lines, at, end) {
                let entry = lines[s..e].join("\n");
                let listed = serde_yaml::from_str::<Value>(&entry).ok().and_then(|v| match v {
                    Value::Sequence(items) => items.first().and_then(|i| i.as_mapping().and_then(|m| get(m, "id").map(text))),
                    _ => None,
                });
                if listed.as_deref() != Some(id.as_str()) {
                    continue;
                }
                match (s + 1..e).find(|&i| lines[i].starts_with("  ack_degrade:")) {
                    Some(i) => lines[i] = "  ack_degrade: true".to_string(),
                    None => lines.insert(e, "  ack_degrade: true".to_string()),
                }
                return Ok(true);
            }
            Ok(false)
        })
    }

    /// `Nd` snoozes N days from today, `turn` quiets the chat's current your-turn, `clear` wakes the item.
    pub fn snooze(&mut self, duration: &str) -> Res<Snooze> {
        if duration == "clear" {
            self.edit(|_, lines, _| {
                front_edit::delete(lines, "snooze_until");
                front_edit::delete(lines, "turn_snoozed_at");
                Ok(true)
            })?;
            return Ok(Snooze::Cleared);
        }
        if duration == "turn" {
            let at = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
            self.edit(move |_, lines, _| Self::set_pair(lines, "turn_snoozed_at", &at).map(|_| true))?;
            return Ok(Snooze::Turn);
        }
        let days = duration.strip_suffix('d').filter(|n| !n.is_empty() && n.bytes().all(|c| c.is_ascii_digit()));
        let Some(days) = days else { return Err(QmError::argument("snooze duration must be Nd, turn or clear")) };
        let n: u64 = days.parse().ok().filter(|n| *n <= 100_000).ok_or_else(|| QmError::new(UNEMITTABLE, "a snooze this long"))?;
        let until = chrono::Local::now().date_naive() + chrono::Days::new(n);
        let until = until.format("%Y-%m-%d").to_string();
        let written = until.clone();
        self.edit(move |_, lines, _| Self::set_pair(lines, "snooze_until", &written).map(|_| true))?;
        Ok(Snooze::Until(until))
    }

    pub fn describe(&mut self, text: &str) -> Res<()> {
        let content = format!("\n{}\n\n", strip(text));
        self.edit(move |_, _, body| {
            *body = sections::replace_section(body, "## Description", &content);
            Ok(true)
        })
        .map(|_| ())
    }
}

/// Claude tends to open note text with the date the writer adds itself; drop its copy.
fn strip_leading_stamp(text: &str) -> String {
    let t = strip(text);
    let b = t.as_bytes();
    let stamped = b.len() >= 11
        && b[..10].iter().enumerate().all(|(i, c)| if i == 4 || i == 7 { *c == b'-' } else { c.is_ascii_digit() })
        && b[10] == b':';
    if stamped {
        t[11..].trim_start_matches(|c: char| c.is_ascii_whitespace()).to_string()
    } else {
        t.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(front: &str, body: &str) -> WorkItem {
        WorkItem { slug: "t".into(), front: parse_front(front).unwrap(), body: body.into(), front_text: front.into() }
    }

    const BODY: &str = "\n## Description\n\nDesc line.\n\n## Notes\n- 2026-01-01: a\n- 2026-01-02 [open]: b\n- 2026-01-03 [history]: c\nfree text\n- 2026-01-04: d\n\n## Snapshots\n\n### 2026-01-05 — session s1\n\ntext\n";

    #[test]
    fn reads_front_matter_fields_with_the_legacy_branch_key() {
        let i = item("title: T\nstate: fridge\nbranch: old\nsessions:\n- id: s1\n- id: s2\n", "");
        assert_eq!((i.title().as_str(), i.state().as_deref()), ("T", Some("fridge")));
        assert_eq!(i.branches(), vec!["old"]);
        assert_eq!((i.session_ids(), i.session_count()), (vec!["s1".to_string(), "s2".to_string()], 2));
        assert!(i.board().is_none() && i.project().is_none());
    }

    #[test]
    fn sections_notes_and_snapshots() {
        let i = item("title: T", BODY);
        assert_eq!(i.description(), "Desc line.");
        let notes = i.notes();
        assert_eq!(notes.iter().map(|n| n.position).collect::<Vec<_>>(), vec![4, 3, 2, 1]);
        assert_eq!(i.snapshot_count(), 1);
    }

    #[test]
    fn injectable_notes_window_open_notes_and_drop_history() {
        let mut body = String::from("## Notes\n- 2026-01-01 [open]: old blocker\n");
        for n in 0..8 {
            body.push_str(&format!("- 2026-01-02: n{n}\n"));
        }
        body.push_str("- 2026-01-03 [history]: mechanics\n");
        let shown = item("title: T", &body).injectable_notes();
        let texts: Vec<&str> = shown.iter().map(|n| n.text.as_str()).collect();
        assert_eq!(texts, vec!["old blocker", "n3", "n4", "n5", "n6", "n7"]);
    }

    #[test]
    fn split_requires_both_fences() {
        assert!(split("t", "no front matter").is_err());
        assert!(split("t", "---\ntitle: T\nbody").is_err());
        assert_eq!(split("t", "---\ntitle: T\n---\nbody\n").unwrap(), ("title: T".into(), "body\n".into()));
    }
}
