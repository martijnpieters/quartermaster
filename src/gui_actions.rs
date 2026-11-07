//! `POST /api/action`, ported command by command (`GuiActions#dispatch`). A command not ported yet
//! answers `None` (the server says 501) before anything is read or written, so a half-ported action
//! never runs; a value whose Ruby behavior this port does not reproduce does the same.

use std::sync::Mutex;

use serde_json::{json, Map, Value};

use crate::attention::Attention;
use crate::data_root;
use crate::error::{QmError, UNEMITTABLE};
use crate::note::DEFAULT_NOTE_KIND;
use crate::ruby_text::strip;
use crate::work_item::{WorkItem, STATES};

/// Ruby runs actions under one mutex.
static ACTION_LOCK: Mutex<()> = Mutex::new(());

/// Every command Ruby's dispatcher knows, ported or not: the rest are `unknown action`.
const KNOWN: &[&str] = &[
    "resume", "resume-session", "start", "state", "move", "note", "retag", "describe", "retitle", "pr", "distill", "snooze", "ack", "new", "attach", "detach",
    "item-from-session", "dismiss", "work-on", "pf-switch", "ide", "terminal", "adopt", "restack", "pf-stop", "pf-mode", "run-update", "pane-start", "pane-resume",
    "pane-warp", "rotate", "aside-open", "aside-close", "aside-promote", "aside-reopen", "page-annotations", "drafts-load", "diff-viewed", "draft-put", "draft-move",
    "review-post", "thread-reply-put", "pr-thread-resolve", "review-preview", "draft-place", "issue-comment-put", "draft-edit", "draft-delete", "drafts-discard",
    "drafts-send", "issues-clear-created", "issue-acknowledge", "issues-create", "build-progress", "branch-stats-commentary", "board-answer", "issue-preview",
    "issue-start", "pr-chat", "pr-review", "pr-pf", "pr-ide", "pr-unrequire", "cleanup", "caffeinate", "benchmark-start", "benchmark-cancel", "benchmark-rerun",
    "loose-start", "loose-close", "loose-promote",
];

type R<T> = Result<T, Stop>;

/// Either the action's own refusal (a JSON reply) or "Ruby's job".
pub(crate) enum Stop {
    Reply(Value),
    Unported,
}

pub(crate) fn fail(message: impl Into<String>) -> Stop {
    Stop::Reply(json!({"ok": false, "error": message.into()}))
}

/// What `Ruby ... rescue` makes of an error: ArgumentError is the user's, the rest also lands in qm.log.
impl Stop {
    pub(crate) fn refuse(message: impl Into<String>) -> Stop {
        fail(message)
    }
}

fn error(e: QmError) -> Stop {
    if e.class == UNEMITTABLE {
        return Stop::Unported;
    }
    if e.class != "ArgumentError" {
        data_root::log_error("gui-action", e.class, &e.message);
    }
    fail(e.message)
}

pub(crate) fn ok() -> Value {
    json!({"ok": true})
}

/// `value.to_s` for the scalars a page sends; anything else is left to Ruby.
fn to_s(v: &Value) -> R<String> {
    match v {
        Value::Null => Ok(String::new()),
        Value::String(s) => Ok(s.clone()),
        Value::Bool(b) => Ok(b.to_string()),
        Value::Number(n) if n.is_i64() || n.is_u64() => Ok(n.to_string()),
        _ => Err(Stop::Unported),
    }
}

struct Args<'a>(Option<&'a Map<String, Value>>);

impl Args<'_> {
    fn get(&self, key: &str) -> R<&Value> {
        static NULL: Value = Value::Null;
        match self.0 {
            Some(m) => Ok(m.get(key).unwrap_or(&NULL)),
            None => Err(Stop::Unported),
        }
    }
    fn text(&self, key: &str) -> R<String> {
        to_s(self.get(key)?)
    }
    /// A string the action passes on as it is (nil and other types behave differently in Ruby).
    fn string(&self, key: &str) -> R<String> {
        match self.get(key)? {
            Value::String(s) => Ok(s.clone()),
            _ => Err(Stop::Unported),
        }
    }
}

pub(crate) fn find(slug: &str) -> R<WorkItem> {
    WorkItem::find(slug).map_err(error)
}

/// `Some(reply)`, or `None` when the command is not ported (or not as this input).
pub fn dispatch(payload: &Value) -> Option<Value> {
    // the benchmark actions never hold the action mutex: a run has its own start lock and meta.json flock
    let unlocked = payload.get("cmd").and_then(Value::as_str).is_some_and(|c| c.starts_with("benchmark-"));
    let _guard = (!unlocked).then(|| ACTION_LOCK.lock().unwrap_or_else(|p| p.into_inner()));
    match run(payload) {
        Ok(v) => Some(v),
        Err(Stop::Reply(v)) => Some(v),
        Err(Stop::Unported) => None,
    }
}

fn run(payload: &Value) -> R<Value> {
    let empty = Map::new();
    let obj = match payload {
        Value::Object(o) => o,
        Value::String(_) => &empty,
        Value::Array(_) | Value::Number(_) if payload.is_array() || payload.as_i64().is_some() || payload.as_u64().is_some() => {
            let message = "no implicit conversion of String into Integer";
            data_root::log_error("gui-action", "TypeError", message);
            return Err(fail(message));
        }
        _ => return Err(Stop::Unported),
    };
    let cmd = match obj.get("cmd") {
        None | Some(Value::Null) => String::new(),
        Some(v) => to_s(v)?,
    };
    let known = matches!(obj.get("cmd"), Some(Value::String(_))) && KNOWN.contains(&cmd.as_str());
    if !known {
        return Err(fail(format!("unknown action '{cmd}'")));
    }
    let slug = match obj.get("slug") {
        None | Some(Value::Null) => String::new(),
        Some(Value::String(s)) => s.clone(),
        Some(_) => return Err(Stop::Unported),
    };
    let args = match obj.get("args") {
        None | Some(Value::Null) | Some(Value::Bool(false)) => Args(Some(&empty)),
        Some(Value::Object(o)) => Args(Some(o)),
        Some(_) => Args(None),
    };
    match cmd.as_str() {
        "note" => note(&slug, &args),
        "retag" => retag(&slug, &args),
        "describe" => {
            let text = args.text("text")?;
            find(&slug)?.describe(&text).map_err(error)?;
            Ok(ok())
        }
        "retitle" => {
            let title = strip(&args.text("title")?).to_string();
            if title.is_empty() {
                return Err(fail("title is empty"));
            }
            find(&slug)?.retitle(&title).map_err(error)?;
            Ok(ok())
        }
        "move" => {
            let mut item = find(&slug)?;
            item.move_to(&args.string("project")?).map_err(error)?;
            Ok(ok())
        }
        "state" => state(&slug, &args),
        "snooze" => {
            let mut item = find(&slug)?;
            let duration = match args.get("duration")? {
                Value::String(s) => s.clone(),
                other => to_s(other)?,
            };
            item.snooze(&duration).map_err(error)?;
            Ok(ok())
        }
        "ack" => {
            let mut item = find(&slug)?;
            let ids = Attention::new(std::time::SystemTime::now()).acknowledge_all(&mut item, None).map_err(error)?;
            Ok(json!({"ok": true, "acked": ids.len()}))
        }
        "detach" => {
            let mut item = find(&slug)?;
            let session = match args.get("session")? {
                Value::Null => return Ok(Value::Bool(false)),
                Value::String(s) => s.clone(),
                _ => return Err(Stop::Unported),
            };
            // Ruby answers `false` itself when nothing was attached
            Ok(if item.detach_session(&session).map_err(error)? { ok() } else { Value::Bool(false) })
        }
        "diff-viewed" => {
            let item = find(&slug)?;
            let slug = item.slug.clone();
            if !slug_shaped(&slug) {
                return Err(fail(format!("'{slug}' is not a work item slug")));
            }
            let viewed = matches!(args.get("viewed")?, Value::Bool(true));
            match crate::viewed_marks::set(&slug, args.get("path")?, args.get("digest")?, viewed) {
                Ok(v) => Ok(json!({"ok": true, "viewed": v})),
                Err(crate::viewed_marks::Halt::Refuse(m)) => Err(fail(m)),
                Err(crate::viewed_marks::Halt::Unported) => Err(Stop::Unported),
            }
        }
        "issue-acknowledge" | "issues-clear-created" => {
            let item = find(&slug)?;
            let slug = item.slug.clone();
            if !slug_shaped(&slug) {
                return Err(fail(format!("'{slug}' is not a work item slug")));
            }
            issues(&cmd, &slug, &args)
        }
        "new" => new_item(&args),
        "item-from-session" => item_from_session(&args),
        "attach" => {
            let mut item = find(&slug)?;
            let session = args.string("session")?;
            let vacated = item.claim_session(&session).map_err(error)?;
            Ok(followed(json!({"ok": true, "detached": vacated}), &session, &vacated, &slug))
        }
        "dismiss" => {
            let session = args.string("session")?;
            crate::session_log::SessionLog::dismiss(&session).map_err(error)?;
            Ok(ok())
        }
        "aside-open" | "aside-reopen" | "aside-close" | "aside-promote" => aside(&cmd, &slug, &args),
        "loose-start" => loose_start(&args),
        "loose-promote" => loose_promote(&args),
        "loose-close" => loose_close(&args),
        "cleanup" => cleanup(&args),
        "pr-unrequire" => {
            let (url, reference) = (args.text("url")?, args.text("ref")?);
            if !crate::pr_prerequisites::remove(&url, &reference).map_err(|_| Stop::Unported)? {
                return Err(fail(format!("{url} has no prerequisite {reference}")));
            }
            Ok(ok())
        }
        "pr-chat" => pr_chat(&args),
        "pr-review" => pr_review(&args),
        "pr-pf" | "pr-ide" => pr_workspace_job(&args, cmd == "pr-pf"),
        "start" => {
            let item = find(&slug)?;
            open_session(&crate::branch_diff::start_cwd(&item), &["claude"], &[("QM_ITEM", &item.slug)], &format!("qm {}", item.slug))?;
            // Ruby's `Launcher#start(item) && ok` is what `puts` returns: nil
            Ok(Value::Null)
        }
        "resume" => resume(&slug, &args),
        "resume-session" => resume_session(&args),
        "pane-warp" => {
            find(&slug)?;
            let tmux = crate::tmux::Tmux::new();
            if !tmux.live(&slug) {
                return Err(fail(format!("{slug} has no chat pane to open")));
            }
            let words = ["tmux".to_string(), "-L".to_string(), tmux.socket(), "attach-session".to_string(), "-t".to_string(), format!("={slug}")];
            let attach = words.iter().map(|w| crate::project_config::shell_escape(w)).collect::<Vec<_>>().join(" ");
            open_warp(&crate::data_root::home(), &attach, &format!("qm {slug}"))?;
            Ok(ok())
        }
        "caffeinate" => caffeinate(&args),
        "adopt" => adopt(&slug, &args),
        "restack" => restack(&slug, &args),
        "work-on" | "pf-switch" | "ide" | "terminal" => workspace_job(&cmd, &slug, &args),
        "pf-stop" => Ok(crate::gui_jobs::background("pf", "stopping the dev server…", true, || match crate::workspace::stop_pf() {
            Ok(message) => Ok(json!({"ok": true, "message": message})),
            Err(m) => Err(crate::gui_jobs::Failure::Argument(m)),
        })),
        "run-update" => run_update(&args),
        "pane-start" | "pane-resume" => pane(&slug, cmd == "pane-start", &args),
        "pf-mode" => {
            let mut item = find(&slug)?;
            item.update_pf_mode(&args.text("mode")?).map_err(error)?;
            Ok(ok())
        }
        "benchmark-start" | "benchmark-cancel" | "benchmark-rerun" => benchmark_action(&cmd, &slug, &args),
        "distill" => distill_action(&slug, &args),
        "rotate" => rotate_action(&slug, &args),
        "board-answer" => board_answer(&slug, &args),
        "build-progress" => build_progress_action(&slug),
        "branch-stats-commentary" => branch_stats_commentary(&slug, &args),
        "issue-preview" => issue_preview(&args),
        "issue-start" => issue_start(&args),
        "pr-thread-resolve" => match args.0 {
            Some(raw) => crate::gui_review_actions::resolve(raw),
            None => Err(Stop::Unported),
        },
        "review-preview" => match args.0 {
            Some(raw) => crate::gui_review_actions::preview(&slug, raw),
            None => Err(Stop::Unported),
        },
        "review-post" => match args.0 {
            Some(raw) => crate::gui_review_actions::post(&slug, raw),
            None => Err(Stop::Unported),
        },
        "draft-place" => match args.0 {
            Some(raw) => crate::gui_review_actions::place(&slug, raw),
            None => Err(Stop::Unported),
        },
        "page-annotations" => crate::gui_draft_actions::page_annotations(args.get("root")?, args.get("annotations")?, args.get("url")?, args.get("title")?),
        "drafts-load" => crate::gui_draft_actions::drafts_load(&slug, args.get("text")?),
        "drafts-send" => match args.0 {
            Some(raw) => crate::gui_draft_send::drafts_send(&slug, raw),
            None => Err(Stop::Unported),
        },
        "issues-create" => crate::gui_draft_send::issues_create(&slug, args.get("files")?),
        "draft-edit" | "draft-delete" | "drafts-discard" | "draft-put" | "draft-move" | "issue-comment-put" | "thread-reply-put" => drafts(&cmd, &slug, &args),
        _ => Err(Stop::Unported),
    }
}

fn issues(cmd: &str, slug: &str, args: &Args) -> R<Value> {
    use crate::issue_drafts as i;
    if cmd == "issue-acknowledge" {
        let name = match args.get("file")? {
            Value::String(s) => s.clone(),
            other => {
                to_s(other)?;
                return Err(fail(format!("'{}' is not an issue file", to_s(other)?)));
            }
        };
        if !i::is_issue_file(&name) {
            return Err(fail(format!("'{name}' is not an issue file")));
        }
        let acknowledged = i::acknowledge(slug, &name).map_err(|_| Stop::Unported)?;
        return Ok(json!({"ok": true, "acknowledged": acknowledged}));
    }
    if !crate::drafts::writable(slug) {
        return Err(Stop::Unported);
    }
    let loaded = i::load(slug).ok_or(Stop::Unported)?;
    loaded.record().map_err(|_| Stop::Unported)?;
    let created: Vec<String> = loaded.issues.iter().filter(|x| x.number.is_some()).map(|x| x.file.clone()).collect();
    for name in &created {
        i::delete(slug, name).map_err(|_| Stop::Unported)?;
    }
    Ok(json!({"ok": true, "cleared": created}))
}

/// `Launcher`-free item creation: a title is required, the project defaults to "No project".
fn new_item(args: &Args) -> R<Value> {
    let title = strip(&args.text("title")?).to_string();
    if title.is_empty() {
        return Err(fail("title is required"));
    }
    let description = strip(&args.text("description")?).to_string();
    let project = match args.get("project")? {
        Value::Null | Value::Bool(false) => "No project".to_string(),
        Value::String(s) => s.clone(),
        _ => return Err(Stop::Unported),
    };
    let item = WorkItem::create(&title, &project, None, None, (!description.is_empty()).then_some(description.as_str()), None).map_err(error)?;
    Ok(json!({"ok": true, "slug": item.slug}))
}

/// `WorkItem.from_session`: an item for a session, on its start event's project and branch, the
/// session claimed from every other item. Returns the item and the slugs it left.
fn from_session(session: &str, title: &str) -> R<(WorkItem, Vec<String>)> {
    let log = crate::session_log::SessionLog::read();
    let Some(start) = log.last_start_for(Some(session)) else { return Err(fail(format!("unknown session {session}"))) };
    let text_of = |key: &str| match start.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) => Ok(Some(s.clone())),
        Some(_) => Err(Stop::Unported),
    };
    let branch = text_of("branch")?.filter(|b| !["main", "master", "develop"].contains(&b.as_str()));
    let project = text_of("project")?.unwrap_or_else(|| "No project".to_string());
    let item = WorkItem::create(title, &project, branch.as_deref(), Some(session), None, None).map_err(error)?;
    let vacated = WorkItem::detach_session_elsewhere(session, &item.slug).map_err(error)?;
    Ok((item, vacated))
}

fn item_from_session(args: &Args) -> R<Value> {
    let title = strip(&args.text("title")?).to_string();
    if title.is_empty() {
        return Err(fail("title is required"));
    }
    let session = args.string("session")?;
    let (item, vacated) = from_session(&session, &title)?;
    Ok(followed(json!({"ok": true, "slug": item.slug, "detached": vacated}), &session, &vacated, &item.slug))
}

/// A chat pane running the moved session follows it to the new item; one that can't says so.
fn followed(mut reply: Value, session: &str, vacated: &[String], to: &str) -> Value {
    if let Some(warning) = crate::pane_follows_session::follow(session, vacated, to) {
        reply.as_object_mut().unwrap().insert("warning".into(), json!(warning));
    }
    reply
}

fn aside_halt(h: crate::aside::Halt) -> Stop {
    match h {
        crate::aside::Halt::Refuse(m) => fail(m),
        crate::aside::Halt::Unported => Stop::Unported,
    }
}

/// The aside drawer's actions (`Aside`): its own tmux session beside the item's chat.
fn aside(cmd: &str, slug: &str, args: &Args) -> R<Value> {
    let mut item = find(slug)?;
    let session = crate::chats_commands::aside_name(slug);
    match cmd {
        "aside-open" => {
            let started = crate::aside::open(&item, &args.text("model")?).map_err(aside_halt)?;
            Ok(json!({"ok": true, "session": session, "attached": !started}))
        }
        "aside-reopen" => {
            crate::aside::reopen(&item, &args.text("session")?).map_err(aside_halt)?;
            Ok(json!({"ok": true, "session": session}))
        }
        "aside-close" => {
            crate::aside::close(&item);
            Ok(ok())
        }
        _ => {
            let given = args.text("session")?;
            let wanted = (!strip(&given).is_empty()).then_some(given);
            let note = crate::aside::promote(&mut item, wanted.as_deref()).map_err(aside_halt)?;
            Ok(json!({"ok": true, "note": note}))
        }
    }
}

/// `String#strip`, and nil when nothing is left (`presence` in the loose actions).
fn presence_of(args: &Args, key: &str) -> R<Option<String>> {
    let text = strip(&args.text(key)?).to_string();
    Ok((!text.is_empty()).then_some(text))
}

/// `LooseChat.kickoff`: a starter's prompt, which has the chat load something before the first question,
/// ending in the typed question (or asking for one); else the question alone.
fn loose_kickoff(starter: Option<&str>, question: Option<&str>) -> R<Option<String>> {
    let question = question.map(|q| strip(q).to_string()).filter(|q| !q.is_empty());
    let Some(starter) = starter else { return Ok(question) };
    let config = crate::project_config::ProjectConfig::current();
    let Some(known) = config.loose_starters().iter().find(|s| s.key == starter) else {
        let shown = crate::ruby_text::inspect_ascii(starter).ok_or(Stop::Unported)?;
        let keys: Vec<&str> = config.loose_starters().iter().map(|s| s.key.as_str()).collect();
        return Err(fail(format!("unknown starter {shown} (config.yml loose_starters: {})", keys.join(", "))));
    };
    Ok(Some(match question {
        Some(q) => format!("{} answer my question about them:\n\n{q}", known.prompt),
        None => format!("{} ask me what I want to know about them.", known.prompt),
    }))
}

/// `loose-start`: a chat that belongs to no item, in its own tmux session `loose--<id>` under `QM_LOOSE`.
fn loose_start(args: &Args) -> R<Value> {
    let model = presence_of(args, "model")?;
    if let Some(m) = &model {
        if !["sonnet", "opus", "haiku"].contains(&m.as_str()) {
            let shown = crate::ruby_text::inspect_ascii(m).ok_or(Stop::Unported)?;
            return Err(fail(format!("unknown model {shown} (sonnet, opus, haiku)")));
        }
    }
    let starter = presence_of(args, "starter")?;
    let prompt = loose_kickoff(starter.as_deref(), presence_of(args, "prompt")?.as_deref())?;
    let project = presence_of(args, "project")?.unwrap_or_else(|| "No project".to_string());
    let command = crate::launcher::chat_command().ok_or(Stop::Unported)?;
    let id = crate::gui_draft_actions::random_hex(4);
    let root = crate::project_map::root_for(Some(&project)).filter(|r| std::path::Path::new(r).is_dir());
    let cwd = root.unwrap_or_else(crate::data_root::home);
    let mut argv = command;
    if let Some(m) = model {
        argv.extend(["--model".to_string(), m]);
    }
    argv.extend(prompt);
    let name = format!("loose--{id}");
    match crate::tmux::Tmux::new().open(&name, &cwd, &argv, &[("QM_LOOSE".to_string(), id.clone()), ("QM_ITEM".to_string(), String::new())]) {
        Ok(_) => Ok(json!({"ok": true, "name": name, "id": id})),
        Err(message) => Err(fail(message)),
    }
}

/// `Launcher#open_warp_tab`: a Warp tab, or the abort Ruby's GUI reports as "operation aborted".
fn open_warp(cwd: &str, command: &str, title: &str) -> R<()> {
    match crate::workspace::warp_tab(cwd, title, Some(command)) {
        Ok(opened) => {
            println!("{opened}");
            Ok(())
        }
        Err(m) => {
            eprintln!("qm: {m}");
            Err(fail("operation aborted"))
        }
    }
}

/// `Launcher#open_session` without `here`: the command in a new Warp tab.
fn open_session(cwd: &str, argv: &[&str], env: &[(&str, &str)], title: &str) -> R<()> {
    let cwd = if std::path::Path::new(cwd).is_dir() { cwd.to_string() } else { crate::data_root::home() };
    let escape = crate::project_config::shell_escape;
    let mut words: Vec<String> = env.iter().map(|(k, v)| format!("{k}={}", escape(v))).collect();
    words.push(argv.iter().map(|a| escape(a)).collect::<Vec<_>>().join(" "));
    open_warp(&cwd, &words.join(" "), title)
}

const RUNNING_NOW: &str = "that session is running right now — switch to its Warp tab";

/// Resume for an item's chat in a Warp tab: the newest resumable session, else rehydrate.
fn resume(slug: &str, args: &Args) -> R<Value> {
    let item = find(slug)?;
    // a given id, even an empty one, narrows the item's sessions to it
    let wanted = match args.get("session")? {
        Value::Null => None,
        Value::String(s) => Some(s.clone()),
        _ => return Err(Stop::Unported),
    };
    match crate::launcher::resume_plan(&item, wanted.as_deref()) {
        crate::launcher::Resume::Live => Err(fail(RUNNING_NOW)),
        crate::launcher::Resume::Resume { session_id, cwd } => {
            open_session(&cwd, &["claude", "--resume", &session_id], &[], &format!("resume {slug}"))?;
            Ok(ok())
        }
        crate::launcher::Resume::Rehydrate { cwd } => {
            open_session(&cwd, &["claude"], &[("QM_ITEM", &item.slug)], &format!("qm {}", item.slug))?;
            Ok(ok())
        }
    }
}

/// Resume for a session with no work item (the inbox's session sheet).
fn resume_session(args: &Args) -> R<Value> {
    let id = args.string("session")?;
    match crate::launcher::session_resume_plan(&id) {
        Err(true) => Err(fail(RUNNING_NOW)),
        Err(false) => Err(fail("no surviving transcript to resume")),
        Ok(cwd) => {
            let short: String = id.chars().take(8).collect();
            open_session(&cwd, &["claude", "--resume", &id], &[], &format!("resume {short}"))?;
            Ok(ok())
        }
    }
}

/// Any live session, linked or not, mid-turn or with background jobs out (`sessions_busy?`).
pub fn sessions_busy() -> bool {
    let log = crate::session_log::SessionLog::read();
    let mut attention = crate::chat_attention::ChatAttention::new();
    log.sessions().iter().filter(|s| s.live).any(|s| matches!(attention.turn_for(s).state.as_str(), "working" | "waiting"))
}

fn pr_of(args: &Args) -> R<Value> {
    crate::pr_inbox::find(&args.text("url")?).map_err(fail)
}

/// A PR's own chat: the item holding its branch, or a new one, with its pane opened or resumed.
fn pr_chat(args: &Args) -> R<Value> {
    let pr = pr_of(args)?;
    let project = crate::pr_items::project_for_repo(pr["repo"].as_str().unwrap_or("")).unwrap_or_else(|| "No project".to_string());
    let items = WorkItem::all().map_err(error)?;
    let slug = match crate::pr_items::pr_item(&pr, Some(&project), &items) {
        Some(i) => i.slug.clone(),
        None => {
            let title = format!("{}: {}", crate::pr_items::pr_label(&pr), pr["title"].as_str().unwrap_or(""));
            find_or_create(&title, &project, pr["head"].as_str(), &format!("Pull request {}.", crate::pr_items::pr_summary(&pr)))?
        }
    };
    let mut reply = pane(&slug, false, &Args(Some(&Map::new())))?;
    reply["slug"] = json!(slug);
    Ok(reply)
}

fn find_or_create(title: &str, project: &str, branch: Option<&str>, description: &str) -> R<String> {
    let slug = WorkItem::slugify(title);
    if WorkItem::path_for(&slug).exists() {
        return Ok(find(&slug)?.slug);
    }
    Ok(WorkItem::create(title, project, branch, None, Some(description), None).map_err(error)?.slug)
}

/// A review runs in a chat of its own — a separate item, so it never lands in the PR's own chat — with the
/// review skill as its first prompt.
fn pr_review(args: &Args) -> R<Value> {
    let pr = pr_of(args)?;
    let project = crate::pr_items::project_for_repo(pr["repo"].as_str().unwrap_or("")).unwrap_or_else(|| "No project".to_string());
    let items = WorkItem::all().map_err(error)?;
    let slug = match crate::pr_items::review_item(&pr, &items) {
        Some(i) => i.slug.clone(),
        None => {
            let title = format!("Review {}: {}", crate::pr_items::pr_label(&pr), pr["title"].as_str().unwrap_or(""));
            find_or_create(&title, &project, None, &format!("Review of pull request {}.", crate::pr_items::pr_summary(&pr)))?
        }
    };
    let tmux = crate::tmux::Tmux::new();
    if tmux.live(&slug) {
        return Ok(json!({"ok": true, "slug": slug, "attached": true}));
    }
    let mut item = find(&slug)?;
    if item.state().as_deref() == Some("resolved") {
        item.update_state("in_progress").map_err(error)?;
    }
    let plan = crate::launcher::chat_plan(&item, true, None, None).ok_or(Stop::Unported)?;
    if matches!(plan.mode, crate::launcher::Mode::Live) {
        return Err(fail(format!("{slug} has a chat running outside the GUI — review from there, or end it first")));
    }
    let skill = crate::project_config::ProjectConfig::current().review_skill(Some(&project));
    let argv: Vec<String> = plan.argv.iter().cloned().chain([crate::pr_items::review_kickoff(&skill, &pr, &slug)]).collect();
    match tmux.open(&slug, &plan.cwd, &argv, &plan.env) {
        Ok(_) => Ok(json!({"ok": true, "slug": slug, "mode": "start"})),
        Err(m) => Err(fail(m)),
    }
}

/// The dev server / the IDE on the PR's own branch (a new worktree when none has it).
fn pr_workspace_job(args: &Args, pf: bool) -> R<Value> {
    let pr = pr_of(args)?;
    let repo = pr["repo"].as_str().unwrap_or("").to_string();
    let Some(project) = crate::pr_items::project_for_repo(&repo) else {
        return Err(fail(format!("{repo} is not a project in config.yml — qm doesn't know where it is checked out")));
    };
    let config = crate::project_config::ProjectConfig::current();
    let label = if pf {
        "switching the dev server…".to_string()
    } else {
        format!("opening {}…", config.ide(Some(&project)).ok_or_else(|| fail("no IDE configured: set ide_command (and ide_label) in config.yml"))?.label)
    };
    let items = WorkItem::all().map_err(error)?;
    let item = crate::pr_items::pr_item(&pr, Some(&project), &items);
    let key = item.map(|i| i.slug.clone()).unwrap_or_else(|| crate::pr_items::pr_label(&pr));
    let mode = item.and_then(|i| i.pf_mode());
    let (head, shown) = (pr["head"].as_str().unwrap_or("").to_string(), crate::pr_items::pr_label(&pr));
    Ok(crate::gui_jobs::background(&key, &label, pf, move || {
        let report = if pf { crate::workspace::switch_pf_to_branch(&project, &head, mode.as_deref()) } else { crate::workspace::open_ide_on_branch(&project, &head) }.map_err(crate::gui_jobs::Failure::Argument)?;
        Ok(json!({"ok": true, "message": format!("{shown}: {}", report.steps.join("; ")), "switched": report.mode.is_some(), "path": report.target.path}))
    }))
}

/// The ☕ toggle, and `qm caffeinate on|off|auto|status` from the CLI.
fn caffeinate(args: &Args) -> R<Value> {
    let failed = |m: String| {
        data_root::log_error("gui-action", "Errno::ENOENT", &m);
        fail(m)
    };
    let status = match args.get("mode")? {
        Value::String(m) if m == "on" => crate::caffeinate::on().map_err(failed)?,
        Value::String(m) if m == "off" => crate::caffeinate::off(),
        Value::String(m) if m == "auto" => crate::caffeinate::auto(sessions_busy()).map_err(failed)?,
        Value::String(m) if m == "status" => crate::caffeinate::status(),
        Value::Null => crate::caffeinate::status(),
        _ => return Err(fail("caffeinate: mode must be on, off, auto or status")),
    };
    Ok(json!({"ok": true, "caffeinate": status}))
}

/// The running chat of an item, if any: its newest live session's turn.
fn live_turn(item: &WorkItem) -> Option<crate::chat_attention::Turn> {
    let session = crate::session_log::SessionLog::read().item_sessions(item).ok()?.into_iter().find(|s| s.live)?;
    Some(crate::chat_attention::ChatAttention::new().turn_for(&session))
}

pub(crate) fn refuse_mid_turn(item: &WorkItem) -> R<()> {
    if let Some(turn) = live_turn(item) {
        if turn.state == "working" || turn.state == "blocked" {
            return Err(fail(format!("{}'s chat is mid-turn ({}) — try again once it is your turn", item.slug, turn.state)));
        }
    }
    Ok(())
}

/// Background work out: the turn has ended, but its report would land on top of a typed prompt.
fn refuse_waiting(item: &WorkItem) -> R<()> {
    if let Some(turn) = live_turn(item) {
        if turn.state == "waiting" {
            let on: Vec<String> = turn.waiting_on.unwrap_or_default().into_iter().map(Option::unwrap_or_default).collect();
            return Err(fail(format!("{}'s chat is waiting on background work ({}) — try again once it has reported back", item.slug, on.join(", "))));
        }
    }
    Ok(())
}

/// Types a line into the item's chat pane and submits it, once it is the user's turn.
fn type_into_chat(item: &WorkItem, text: &str) -> R<Value> {
    refuse_mid_turn(item)?;
    let tmux = crate::tmux::Tmux::new();
    let cannot = || fail(format!("could not type into {}'s chat", item.slug));
    if !tmux.send_text(&item.slug, text) {
        return Err(cannot());
    }
    std::thread::sleep(std::time::Duration::from_millis(500));
    if !tmux.send_enter(&item.slug) {
        return Err(cannot());
    }
    Ok(json!({"ok": true, "attached": true}))
}

/// Adopt hands the worktree to the /adopt-worktree skill in the item's chat: typed into a chat in the
/// pane, or a fresh chat started with it as its first prompt in the main checkout.
fn adopt(slug: &str, args: &Args) -> R<Value> {
    let item = find(slug)?;
    let project = item.project();
    let root = crate::project_map::root_for(project.as_deref()).ok_or_else(|| fail(format!("no repo root configured for project '{}'", project.clone().unwrap_or_default())))?;
    let main = crate::data_root::expand_path(&root).to_string_lossy().into_owned();
    let path = crate::data_root::expand_path(&args.text("worktree")?).to_string_lossy().into_owned();
    let shown = crate::cli_format::short_path(Some(&path));
    if path == main {
        return Err(fail(format!("{shown} is the main checkout — nothing to adopt")));
    }
    let Some((branch, _)) = crate::branch_diff::branch_worktrees(&main).into_iter().find(|(_, p)| *p == path) else {
        return Err(fail(format!("{shown} is not a worktree of {}", project.unwrap_or_default())));
    };
    let prompt = format!("/adopt-worktree {path}");
    let tmux = crate::tmux::Tmux::new();
    if tmux.live(slug) {
        return type_into_chat(&item, &prompt);
    }
    let plan = crate::launcher::chat_plan(&item, true, None, None).ok_or(Stop::Unported)?;
    if matches!(plan.mode, crate::launcher::Mode::Live) {
        return Err(fail(format!("{slug} has a chat running outside the GUI — adopt from there, or end it first")));
    }
    let argv: Vec<String> = plan.argv.iter().cloned().chain([prompt]).collect();
    match tmux.open(slug, &main, &argv, &plan.env) {
        Ok(_) => Ok(json!({"ok": true, "mode": "start", "branch": branch})),
        Err(m) => Err(fail(m)),
    }
}

/// One line: typed into a pane, each newline would submit.
fn restack_prompt(chain: &Value) -> String {
    let layers = chain["layers"].as_array().cloned().unwrap_or_default();
    let name = |l: &Value| l["branch"].as_str().unwrap_or("").to_string();
    let branches: Vec<String> = layers.iter().map(name).collect();
    let behind: Vec<String> = layers.iter().filter(|l| l["needsRestack"].as_bool().unwrap_or(false)).map(name).collect();
    let status = if behind.is_empty() { "no layer is flagged as behind its parent".to_string() } else { format!("behind their parent: {}", behind.join(", ")) };
    let trunk = chain["trunk"].as_str().unwrap_or("");
    format!("Restack the gh-stack {trunk} ← {} (bottom → top; {status}) with `gh stack rebase` (see the gh-stack skill). Resolve the conflicts it hits, or stop and report any you can't resolve safely.", branches.join(" ← "))
}

/// The ⟳ chip, only ever on the user's click: the item's chat restacks with gh-stack — qm itself never
/// runs git or gh for it.
fn restack(slug: &str, args: &Args) -> R<Value> {
    let item = find(slug)?;
    let branch = presence_of(args, "branch")?;
    let empty = serde_json::Map::new();
    let stacks: Vec<Value> = crate::gh_stack::chains(&item, &empty, &empty).into_iter().filter(|c| c["kind"] == "stack").collect();
    let has = |c: &Value, b: &str| c["layers"].as_array().is_some_and(|l| l.iter().any(|x| x["branch"] == b));
    let chain = match &branch {
        Some(b) => stacks.iter().find(|c| has(c, b)),
        None => stacks.first(),
    };
    let Some(chain) = chain else {
        return Err(fail(match &branch {
            Some(b) => format!("{b} is in none of {slug}'s stacks"),
            None => format!("{slug} has no stack to restack"),
        }));
    };
    let prompt = restack_prompt(chain);
    let tmux = crate::tmux::Tmux::new();
    if tmux.live(slug) {
        refuse_waiting(&item)?;
        return type_into_chat(&item, &prompt);
    }
    let plan = crate::launcher::chat_plan(&item, true, None, Some(&prompt)).ok_or(Stop::Unported)?;
    if matches!(plan.mode, crate::launcher::Mode::Live) {
        return Err(fail(format!("{slug} has a chat running outside the GUI — restack from there, or end it first")));
    }
    match tmux.open(slug, &plan.cwd, &plan.argv, &plan.env) {
        Ok(_) => Ok(json!({"ok": true, "mode": "start"})),
        Err(m) => Err(fail(m)),
    }
}

/// Work on this / Switch the dev server / Open in the IDE / Terminal: a worktree may have to be created
/// and a switch takes seconds, so each runs on a job; only one drives the shared dev server at a time.
fn workspace_job(cmd: &str, slug: &str, args: &Args) -> R<Value> {
    let item = find(slug)?;
    let label = match cmd {
        "work-on" => "working on it…".to_string(),
        "pf-switch" => "switching the dev server…".to_string(),
        "ide" => {
            let config = crate::project_config::ProjectConfig::current();
            let ide = config.ide(item.project().as_deref()).ok_or_else(|| fail("no IDE configured: set ide_command (and ide_label) in config.yml"))?;
            format!("opening {}…", ide.label)
        }
        _ => "opening a terminal…".to_string(),
    };
    let worktree = presence_of(args, "worktree")?;
    let branch = presence_of(args, "branch")?;
    if let Some(b) = &branch {
        crate::workspace::require_linked(&item, b).map_err(fail)?;
    }
    let pf_action = cmd == "work-on" || cmd == "pf-switch";
    let mode = if pf_action { presence_of(args, "mode")? } else { None };
    let uses_pf = pf_action && crate::project_config::ProjectConfig::current().settings(item.project().as_deref()).is_some_and(|s| s.dev_server.is_some());
    let (cmd, slug) = (cmd.to_string(), slug.to_string());
    Ok(crate::gui_jobs::background(&slug.clone(), &label, uses_pf, move || {
        let mut item = WorkItem::find(&slug).map_err(|e| crate::gui_jobs::Failure::Argument(e.message))?;
        let (w, b, m) = (worktree.as_deref(), branch.as_deref(), mode.as_deref());
        let report = match cmd.as_str() {
            "work-on" => crate::workspace::work_on(&mut item, m, w, b),
            "pf-switch" => crate::workspace::switch_pf(&mut item, m, w, b),
            "ide" => crate::workspace::open_ide(&mut item, w, b),
            _ => crate::workspace::open_terminal(&mut item, w, b),
        }
        .map_err(crate::gui_jobs::Failure::Argument)?;
        Ok(crate::workspace::job_reply(&slug, &report))
    }))
}

/// `run-update`: bin/update in a tmux session of its own, waiting for Enter once it finishes.
fn run_update(args: &Args) -> R<Value> {
    let path = crate::data_root::expand_path(&args.text("path")?).to_string_lossy().into_owned();
    let exe = std::path::Path::new(&path).join("bin/update");
    let executable = std::fs::metadata(&exe).is_ok_and(|m| {
        use std::os::unix::fs::PermissionsExt;
        m.is_file() && m.permissions().mode() & 0o111 != 0
    });
    if !executable {
        return Err(fail(format!("no bin/update in {}", crate::cli_format::short_path(Some(&path)))));
    }
    let base = std::path::Path::new(&path).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let name = format!("bin-update-{}", base.chars().map(|c| if c.is_ascii_alphanumeric() || c == '_' || c == '-' { c } else { '-' }).collect::<String>());
    let script = "bin/update; echo; echo \"[bin/update exited $?, Enter closes]\"; read _";
    match crate::tmux::Tmux::new().open(&name, &path, &["sh".to_string(), "-c".to_string(), script.to_string()], &[]) {
        Ok(_) => Ok(json!({"ok": true, "session": name})),
        Err(m) => Err(fail(m)),
    }
}

/// `Array(value).map(&:to_s)` for the scalars a page sends.
fn strings_of(v: &Value) -> R<Vec<String>> {
    match v {
        Value::Null => Ok(vec![]),
        Value::Array(items) => items.iter().map(to_s).collect(),
        Value::Object(_) => Err(Stop::Unported),
        other => Ok(vec![to_s(other)?]),
    }
}

/// Whether the cleanup of this scope involves anything only Ruby reads or writes exactly.
fn cleanup_leaves_to_ruby(scope: &str) -> bool {
    if scope == "sweep" {
        return !crate::cleanup::sweep_supported() || !crate::cleanup::swept_slugs().is_some_and(|s| s.iter().all(|slug| crate::drafts::writable(slug)));
    }
    crate::cleanup_command::leaves_to_ruby(scope) || !crate::drafts::writable(scope)
}

/// `Cleanup::Outcome#to_h`, and the result the dialog shows (`cleanup_result`).
fn cleanup_result(outcomes: &[crate::cleanup_run::Outcome]) -> Value {
    let mut counts: Vec<(&str, usize)> = vec![];
    for o in outcomes {
        match counts.iter_mut().find(|(r, _)| *r == o.result) {
            Some((_, n)) => *n += 1,
            None => counts.push((o.result, 1)),
        }
    }
    let summary = if counts.is_empty() { "nothing to do".to_string() } else { counts.iter().map(|(r, n)| format!("{n} {r}")).collect::<Vec<_>>().join(", ") };
    let failed: Vec<&crate::cleanup_run::Outcome> = outcomes.iter().filter(|o| o.result != "removed").collect();
    let ok = !outcomes.iter().any(|o| o.result == "failed");
    let list: Vec<Value> = outcomes.iter().map(|o| json!({"id": o.id, "kind": o.kind, "result": o.result, "message": o.message})).collect();
    let mut result = json!({"ok": ok, "message": format!("cleanup: {summary}"), "outcomes": list});
    if !ok {
        let prefix = regex::Regex::new(r"\A[A-Za-z0-9_]+:").unwrap();
        let why: Vec<String> = failed.iter().map(|o| format!("{}: {}", prefix.replace(&o.id, ""), o.message)).collect();
        result["error"] = json!(format!("cleanup: {summary} — {}", why.join("; ")));
    }
    result
}

/// `cleanup`: the checked entries of a fresh plan, removed on a job keyed by the scope.
fn cleanup(args: &Args) -> R<Value> {
    let scope = args.text("scope")?;
    let selected = strings_of(args.get("selected")?)?;
    let force = strings_of(args.get("force")?)?;
    if selected.is_empty() && force.is_empty() {
        return Err(fail("nothing selected to clean up"));
    }
    if scope != "sweep" {
        find(&scope)?;
    }
    if cleanup_leaves_to_ruby(&scope) {
        return Err(Stop::Unported);
    }
    let key = scope.clone();
    let name = scope.clone();
    Ok(crate::gui_jobs::background(&name, "cleaning up…", false, move || {
        let planned = if scope == "sweep" {
            crate::cleanup::plan_sweep()
        } else {
            WorkItem::find(&scope).and_then(|item| crate::cleanup::plan_item(&item))
        };
        let plan = planned.map_err(|e| crate::gui_jobs::Failure::Other(e.message, e.class))?;
        let outcomes = crate::cleanup_run::execute_with(&plan, &selected, &force, &mut |index, total| crate::gui_jobs::relabel(&key, &format!("cleaning up {index}/{total}…")));
        Ok(cleanup_result(&outcomes))
    }))
}

/// The checklist a cleanup or sweep dialog shows (`/api/cleanup-plan`); `None` for what is Ruby's.
pub fn cleanup_plan(scope: &Value) -> Option<Value> {
    let scope = to_s(scope).ok()?;
    if cleanup_leaves_to_ruby(&scope) && (scope == "sweep" || WorkItem::find(&scope).is_ok()) {
        return None;
    }
    let planned = if scope == "sweep" {
        crate::cleanup::plan_sweep()
    } else {
        match WorkItem::find(&scope) {
            Ok(item) => crate::cleanup::plan_item(&item),
            Err(e) => return Some(json!({"ok": false, "error": e.message})),
        }
    };
    match planned {
        Ok(plan) => Some(json!({"ok": true, "plan": plan_payload(&plan)})),
        Err(e) if e.class == UNEMITTABLE => None,
        Err(e) => {
            if e.class != "ArgumentError" {
                data_root::log_error("gui-cleanup-plan", e.class, &e.message);
            }
            Some(json!({"ok": false, "error": e.message}))
        }
    }
}

/// `loose-close`: `/exit` waits for claude to go, so it runs on a job keyed by the pane.
fn loose_close(args: &Args) -> R<Value> {
    let id = args.text("id")?;
    if id.is_empty() {
        return Err(fail("no loose chat given"));
    }
    let name = format!("loose--{id}");
    let pane = name.clone();
    Ok(crate::gui_jobs::background(&name, "closing…", false, move || {
        let tmux = crate::tmux::Tmux::new();
        if tmux.live(&pane) {
            tmux.send_text(&pane, "/exit");
            std::thread::sleep(std::time::Duration::from_millis(500));
            tmux.send_enter(&pane);
            for _ in 0..20 {
                if !tmux.live(&pane) || tmux.pane_dead(&pane) {
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(250));
            }
            tmux.kill(&pane);
        }
        for session in crate::session_log::SessionLog::read().sessions().into_iter().filter(|s| s.loose_id().as_deref() == Some(id.as_str())) {
            crate::session_log::SessionLog::dismiss(&session.id).map_err(|e| crate::gui_jobs::Failure::Other(e.message, e.class))?;
        }
        Ok(json!({"ok": true, "message": "loose chat closed"}))
    }))
}

/// `loose-promote`: an item from the chat's newest session, the tmux session renamed to its slug.
/// "Tell the chat" types into the chat and stays Ruby's.
fn loose_promote(args: &Args) -> R<Value> {
    let tell = matches!(args.get("tell")?, Value::Bool(true));
    let id = args.text("id")?;
    let title = strip(&args.text("title")?).to_string();
    if title.is_empty() {
        return Err(fail("title is required"));
    }
    let log = crate::session_log::SessionLog::read();
    let Some(session) = crate::gui_state::loose_session(&log, &id) else { return Err(fail("the loose chat has no captured session yet — give it a moment")) };
    let (item, _) = from_session(&session.id, &title)?;
    let chat = format!("loose--{id}");
    let renamed = crate::tmux::Tmux::new().rename(&chat, &item.slug);
    let mut reply = json!({"ok": true, "slug": item.slug});
    if tell {
        reply["told"] = json!(renamed && tell_item(&item.slug));
    }
    if !renamed {
        reply["warning"] = json!(format!("{} is created, but its chat stays under {chat}: a chat named {} already runs", item.slug, item.slug));
    }
    Ok(reply)
}

/// `LooseChat#tell_item`: one line into the promoted chat naming its item, typed once settled.
fn tell_item(slug: &str) -> bool {
    let tmux = crate::tmux::Tmux::new();
    if !tmux.live(slug) || tmux.pane_dead(slug) {
        return false;
    }
    if !tmux.send_text(slug, &format!("This chat is now work item {slug}; run qm show {slug} and follow its work item protocol from here.")) {
        return false;
    }
    std::thread::sleep(std::time::Duration::from_millis(500));
    tmux.send_enter(slug)
}

/// `gui_pane`: the item's chat in a tmux session of its own — already running is `attached`, a live
/// session outside the GUI refuses, else Start or Resume opens it.
fn pane(slug: &str, fresh: bool, args: &Args) -> R<Value> {
    let item = find(slug)?;
    let tmux = crate::tmux::Tmux::new();
    if tmux.live(slug) {
        return Ok(json!({"ok": true, "attached": true}));
    }
    let session = if fresh {
        None
    } else {
        let given = args.text("session")?;
        (!strip(&given).is_empty()).then_some(given)
    };
    let plan = crate::launcher::chat_plan(&item, fresh, session.as_deref(), None).ok_or(Stop::Unported)?;
    if matches!(plan.mode, crate::launcher::Mode::Live) {
        return Err(fail(format!("{slug} has a chat running outside the GUI — take it over here once it ends")));
    }
    match tmux.open(slug, &plan.cwd, &plan.argv, &plan.env) {
        Ok(_) => Ok(json!({"ok": true, "mode": plan.mode.name()})),
        Err(message) => Err(fail(message)),
    }
}

pub(crate) fn slug_shaped(slug: &str) -> bool {
    slug.chars().next().is_some_and(|c| c.is_ascii_alphanumeric()) && slug.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-'))
}

/// The draft actions open the item first (`drafts_of`), then its drafts, whose slug must be a slug.
fn drafts(cmd: &str, slug: &str, args: &Args) -> R<Value> {
    let item = find(slug)?;
    let slug = item.slug.clone();
    if !slug_shaped(&slug) {
        return Err(fail(format!("'{slug}' is not a work item slug")));
    }
    use crate::gui_draft_actions as d;
    match cmd {
        "draft-edit" => d::edit(&slug, &args.text("id")?, args.get("comment")?),
        "draft-delete" => d::delete(&slug, &args.text("id")?),
        "drafts-discard" => d::discard(&slug, args.get("ids")?),
        "draft-move" => {
            let id = args.text("id")?;
            if args.get("to")?.as_str() == Some("github") {
                crate::gui_review_actions::guard_move(&slug, &id)?;
            }
            d::move_to(&slug, &id, args.get("to")?)
        }
        "issue-comment-put" => match args.0 {
            Some(raw) => d::issue_comment_put(&slug, raw),
            None => Err(Stop::Unported),
        },
        "thread-reply-put" => match args.0 {
            Some(raw) => d::thread_reply_put(&slug, raw),
            None => Err(Stop::Unported),
        },
        _ => match args.0 {
            Some(raw) => {
                if raw.get("kind") == Some(&Value::String("github-comment".into())) && !strip(&args.text("comment")?).is_empty() {
                    crate::gui_review_actions::guard_put(&slug, raw)?;
                }
                d::put(&slug, raw)
            }
            None => Err(Stop::Unported),
        },
    }
}

fn note(slug: &str, args: &Args) -> R<Value> {
    let text = strip(&args.text("text")?).to_string();
    if text.is_empty() {
        return Err(fail("note text is empty"));
    }
    let kind = match args.get("kind")? {
        Value::Null => DEFAULT_NOTE_KIND.to_string(),
        Value::String(s) if s.is_empty() => DEFAULT_NOTE_KIND.to_string(),
        Value::String(s) => s.clone(),
        _ => return Err(Stop::Unported),
    };
    find(slug)?.add_note(&text, &kind).map_err(error)?;
    Ok(ok())
}

fn retag(slug: &str, args: &Args) -> R<Value> {
    let mut item = find(slug)?;
    let position = match args.get("position")? {
        Value::Null => 0,
        Value::Number(n) => n.as_u64().ok_or(Stop::Unported)? as usize,
        Value::String(s) if s == "0" || (s.starts_with(|c: char| ('1'..='9').contains(&c)) && s.bytes().all(|c| c.is_ascii_digit())) => s.parse().map_err(|_| Stop::Unported)?,
        _ => return Err(Stop::Unported),
    };
    let kind = args.string("kind")?;
    item.update_note_kind(position, &kind).map_err(error)?;
    Ok(ok())
}

fn state(slug: &str, args: &Args) -> R<Value> {
    let mut item = find(slug)?;
    let state = match args.get("state")? {
        Value::String(s) => s.clone(),
        _ => return Err(fail(format!("state must be one of: {}", STATES.join(", ")))),
    };
    if state == "resolved" && crate::cleanup_command::leaves_to_ruby(&item.slug) {
        return Err(Stop::Unported); // the plan lists issue drafts, and only Ruby reads some of those
    }
    item.update_state(&state).map_err(error)?;
    if state != "resolved" {
        return Ok(ok());
    }
    // the plan is the page's offer to clean up: a failing plan must never fail the resolve
    let plan = match crate::cleanup::plan_item(&item) {
        Ok(plan) => plan,
        Err(e) => {
            data_root::log_error("cleanup-offer", e.class, &e.message);
            return Ok(ok());
        }
    };
    if plan.is_empty() {
        return Ok(ok());
    }
    let mut reply = ok();
    reply.as_object_mut().unwrap().insert("cleanup".into(), plan_payload(&plan));
    Ok(reply)
}

/// `Cleanup::Plan#to_h` with each entry's path also shown short, as the dialog prints it.
pub(crate) fn plan_payload(plan: &crate::cleanup::Plan) -> Value {
    let entries: Vec<Value> = plan
        .entries
        .iter()
        .map(|e| {
            let mut m = Map::new();
            m.insert("id".into(), json!(e.id));
            m.insert("kind".into(), json!(e.kind));
            m.insert("repo".into(), json!(e.repo));
            m.insert("path".into(), json!(e.path));
            m.insert("branch".into(), json!(e.branch));
            m.insert("item".into(), json!(e.item));
            m.insert("status".into(), json!(e.status));
            m.insert("reasons".into(), json!(e.reasons));
            m.insert("notes".into(), json!(e.notes));
            m.insert("ticked".into(), json!(e.ticked));
            m.insert("db_names".into(), json!(e.db_names));
            m.insert("db_status".into(), if e.is_worktree() { json!(e.db_status) } else { Value::Null });
            m.insert("requires".into(), json!(e.requires));
            m.insert("title".into(), json!(e.title));
            if let Some(path) = &e.path {
                m.insert("short".into(), json!(crate::cli_format::short_path(Some(path))));
            }
            Value::Object(m)
        })
        .collect();
    json!({"scope": plan.scope, "summary": plan.summary(), "notes": plan.notes, "entries": entries})
}

fn issue_fail(f: crate::issue_start::Fail) -> Stop {
    match f {
        crate::issue_start::Fail::Arg(m) => fail(m),
        crate::issue_start::Fail::Unported => Stop::Unported,
    }
}

/// Where an issue number with no project goes.
fn issue_project(args: &Args) -> R<String> {
    if let Some(p) = presence_of(args, "project")? {
        return Ok(p);
    }
    Ok(crate::project_config::ProjectConfig::current().default_project().unwrap_or_else(|| "No project".to_string()))
}

/// `issue-preview`: what Start would do with the issue.
fn issue_preview(args: &Args) -> R<Value> {
    let project = issue_project(args)?;
    let preview = crate::issue_start::preview(args.get("number")?, &project).map_err(issue_fail)?;
    let mut out = ok();
    out["issue"] = preview;
    Ok(out)
}

/// `issue-start`: the issue as an item with its own worktree, and a chat opened on it with the kickoff
/// prompt; an item that already holds the issue just opens.
fn issue_start(args: &Args) -> R<Value> {
    let project = issue_project(args)?;
    let preview = crate::issue_start::preview(args.get("number")?, &project).map_err(issue_fail)?;
    if let Some(existing) = preview["existing"].as_object() {
        let slug = existing["slug"].as_str().unwrap_or("").to_string();
        let mut item = find(&slug)?;
        if item.state().as_deref() == Some("resolved") {
            item.update_state("in_progress").map_err(error)?;
        }
        let mut reply = pane(&item.slug, false, &Args(Some(&Map::new())))?;
        reply["slug"] = json!(item.slug);
        reply["existing"] = json!(true);
        reply["issue"] = preview.clone();
        return Ok(reply);
    }
    let from = presence_of(args, "from")?;
    if let Some(f) = &from {
        find(f)?;
    }
    let slug = preview["slug"].as_str().unwrap_or("").to_string();
    let number = preview["number"].as_i64().unwrap_or(0);
    let (job_slug, job_preview) = (slug.clone(), preview.clone());
    let mut reply = crate::gui_jobs::background(&slug, &format!("starting #{number}…"), false, move || {
        use crate::gui_jobs::Failure;
        let relabel = |label: &str| crate::gui_jobs::relabel(&job_slug, label);
        let (item, path) = crate::issue_start::start(&job_preview, from.as_deref(), &relabel).map_err(|f| match f {
            crate::issue_start::Fail::Arg(m) => Failure::Argument(m),
            crate::issue_start::Fail::Unported => Failure::Other("a value this port can't start an issue from yet".into(), "NotImplementedError"),
        })?;
        let plan = crate::launcher::chat_plan(&item, true, None, Some(&crate::issue_start::kickoff(number))).ok_or_else(|| Failure::Other("no chat command is configured".into(), "NotImplementedError"))?;
        if !matches!(plan.mode, crate::launcher::Mode::Live) {
            crate::tmux::Tmux::new().open(&item.slug, &plan.cwd, &plan.argv, &plan.env).map_err(Failure::Argument)?;
        }
        Ok(json!({"ok": true, "slug": item.slug, "branch": job_preview["branch"], "path": path, "message": format!("#{number} started as {} in {}", item.slug, crate::cli_format::short_path(Some(&path)))}))
    });
    reply["slug"] = json!(slug);
    reply["issue"] = preview;
    Ok(reply)
}

/// `branch-stats-commentary`: one headless haiku call over the branch's stats, a background job per branch.
fn branch_stats_commentary(slug: &str, args: &Args) -> R<Value> {
    use crate::branch_diff::Failure as Fail;
    let mut item = find(slug)?;
    let branch = match args.get("branch")? {
        Value::Null => None,
        Value::String(b) => Some(b.clone()),
        _ => return Err(Stop::Unported),
    };
    let target = match crate::branch_stats::target(&mut item, branch.as_deref()) {
        Ok(t) => t,
        Err(Fail::Arg(m)) => return Err(fail(m)),
        Err(Fail::Unported) => return Err(Stop::Unported),
    };
    let key = crate::branch_stats::job_key(&target);
    let label = format!("reading {}'s stats…", target["branch"].as_str().unwrap_or(""));
    Ok(crate::gui_jobs::background(&key, &label, false, move || {
        use crate::gui_jobs::Failure;
        crate::branch_stats::comment(&target).map_err(|f| match f {
            Fail::Arg(m) => Failure::Argument(m),
            Fail::Unported => Failure::Other("the branch stats cache holds a value this port can't update yet".into(), "NotImplementedError"),
        })
    }))
}

/// `build-progress`: one headless haiku call over every active sub-agent of the item's running Workflow
/// runs, a background job under its own key; its outcome, failures included, replaces the old estimate.
fn build_progress_action(slug: &str) -> R<Value> {
    let item = find(slug)?;
    let key = crate::build_progress::job_key(&item.slug);
    Ok(crate::gui_jobs::background(&key, "estimating build progress…", false, move || {
        use crate::build_progress::Fail;
        let result = match crate::build_progress::call(&item) {
            Ok(v) => v,
            Err(Fail::Arg(m)) => json!({"ok": false, "taken_at": crate::clock::now_iso(), "error": m}),
            Err(Fail::Unported) => return Err(crate::gui_jobs::Failure::Other("a build transcript holds a value this port can't read yet".into(), "NotImplementedError")),
        };
        crate::gui_jobs::store_progress(&item.slug, result.clone());
        Ok(if result["ok"] == Value::Bool(true) { json!({"ok": true, "message": format!("{}: build progress updated", item.slug)}) } else { result })
    }))
}

/// `board-answer`: the user's answer to a blocked row, as the file its sub-agent polls for.
fn board_answer(slug: &str, args: &Args) -> R<Value> {
    let item = find(slug)?;
    let (doc_id, text) = (args.text("doc_id")?, args.text("text")?);
    match crate::board_mirror::answer(&item, &doc_id, &text) {
        Ok(_) => Ok(ok()),
        Err(Ok(m)) => Err(fail(m)),
        Err(Err(_)) => Err(Stop::Unported),
    }
}

fn benchmark_fail(f: crate::benchmark_run::Fail) -> Stop {
    match f {
        crate::benchmark_run::Fail::Arg(m) => fail(m),
        crate::benchmark_run::Fail::Unported => Stop::Unported,
    }
}

/// Refused before the pipeline is touched: the modal links to the busy run.
fn benchmark_busy() -> Option<Value> {
    let busy = crate::benchmark_run::busy(&crate::benchmark_run::all());
    let o = busy.as_object()?;
    let text = |k: &str| match o.get(k) {
        Some(Value::String(t)) => t.clone(),
        Some(Value::Null) | None => String::new(),
        Some(other) => other.to_string(),
    };
    Some(json!({"ok": false, "busy": busy, "error": format!("a benchmark is already running: {} ({}, run {})", text("branch"), text("slug"), text("id"))}))
}

fn benchmark_launched(meta: Value) -> Value {
    json!({"ok": true, "run": meta["id"]})
}

/// `benchmark-start`, `benchmark-cancel` and `benchmark-rerun` (`GuiBenchmarkActions`).
fn benchmark_action(cmd: &str, slug: &str, args: &Args) -> R<Value> {
    use crate::benchmark_run as b;
    match cmd {
        "benchmark-start" => {
            let mut item = find(slug)?;
            b::settings_checked(&item.project().unwrap_or_default()).map_err(benchmark_fail)?;
            let model = match args.get("model")? {
                Value::Null => "opus".to_string(),
                Value::String(m) if m.is_empty() => "opus".to_string(),
                Value::String(m) => m.clone(),
                _ => "\0not a model".to_string(), // no model of that type: the run refuses it by name
            };
            if let Some(busy) = benchmark_busy() {
                return Ok(busy);
            }
            let (branch, request) = (args.text("branch")?, args.text("request")?);
            b::start(&mut item, &branch, &request, &model).map(benchmark_launched).map_err(benchmark_fail)
        }
        "benchmark-rerun" => {
            let id = args.text("run")?;
            if let Some(busy) = benchmark_busy() {
                return Ok(busy);
            }
            b::rerun(&id).map(benchmark_launched).map_err(benchmark_fail)
        }
        _ => {
            let id = args.text("run")?;
            let meta = b::read(&id).map_err(benchmark_fail)?;
            if b::is_final(&meta) {
                let state = meta.get("state").and_then(Value::as_str).unwrap_or("");
                return Err(fail(format!("run {id} already {state}")));
            }
            let key = format!("benchmark-{id}");
            Ok(crate::gui_jobs::background(&key, "cancelling the benchmark…", false, move || {
                crate::benchmark_run::cancel(&id).map_err(|f| match f {
                    crate::benchmark_run::Fail::Arg(m) => crate::gui_jobs::Failure::Argument(m),
                    crate::benchmark_run::Fail::Unported => crate::gui_jobs::Failure::Other("a run folder this port can't update yet".into(), "NotImplementedError"),
                })?;
                Ok(json!({"ok": true, "message": format!("benchmark {id} cancelled")}))
            }))
        }
    }
}

/// `distill`: the plan is made at once (so the already-distilled confirmation works), the haiku calls run
/// on a thread; the page shows the item as distilling until a state build reports the outcome.
fn distill_action(slug: &str, args: &Args) -> R<Value> {
    use crate::distill::Fail;
    let session = match args.get("session")? {
        Value::Null => None,
        Value::String(s) => Some(s.clone()),
        Value::Bool(_) | Value::Number(_) => {
            // an id of another type matches no attached session
            let item = find(slug)?;
            return Err(fail(format!("session {} of '{}' has no surviving transcript to distill", args.text("session")?, item.slug)));
        }
        _ => return Err(Stop::Unported),
    };
    let force = matches!(args.get("force")?, Value::Bool(true));
    let (item, sessions) = match crate::distill::plan(slug, session.as_deref(), force) {
        Ok(p) => p,
        Err(Fail::Arg(m)) => return Err(fail(m)),
        Err(Fail::Already(m)) => return Err(Stop::Reply(json!({"ok": false, "needs_confirmation": true, "error": m}))),
        Err(Fail::Unported) => return Err(Stop::Unported),
    };
    if !crate::gui_jobs::start_distill(slug, sessions.len()) {
        return Ok(json!({"ok": false, "error": format!("'{slug}' is already being distilled")}));
    }
    let (name, count) = (slug.to_string(), sessions.len());
    std::thread::spawn(move || {
        let mut item = item;
        for session in &sessions {
            if let Err(f) = crate::distill::session(&mut item, session) {
                let message = match f {
                    Fail::Arg(m) | Fail::Already(m) => m,
                    Fail::Unported => "a transcript this port can't distill yet".to_string(),
                };
                crate::data_root::log_error("gui-distill", "ArgumentError", &message);
                crate::gui_jobs::finish_distill(&name, json!({"ok": false, "error": message}));
                return;
            }
        }
        crate::gui_jobs::finish_distill(&name, json!({"ok": true, "sessions": count}));
    });
    Ok(json!({"ok": true, "background": true, "sessions": count}))
}

/// `rotate`: hand the item's chat over to a fresh session as the item's background job.
fn rotate_action(slug: &str, args: &Args) -> R<Value> {
    use crate::rotation::Fail;
    let item = find(slug)?;
    let flag = |key: &str| -> R<bool> { Ok(matches!(args.get(key)?, Value::Bool(true))) };
    let (close, finish, by_chat) = (flag("close")?, flag("finish")?, flag("self")?);
    let session = presence_of(args, "session")?;
    let plan = match crate::rotation::plan(&item, close, finish, by_chat, session.as_deref()) {
        Ok(p) => p,
        Err(Fail::Arg(m)) => return Err(fail(m)),
        Err(Fail::Unported) => return Err(Stop::Unported),
    };
    let label = crate::rotation::label(&plan);
    let (mode, closed) = (plan.mode.name(), plan.close);
    let name = slug.to_string();
    let mut reply = crate::gui_jobs::background(slug, label, false, move || {
        use crate::gui_jobs::Failure;
        let mut item = item;
        let relabel = |l: &str| crate::gui_jobs::relabel(&name, l);
        crate::rotation::run(&mut item, &plan, &relabel).map_err(|f| match f {
            Fail::Arg(m) => Failure::Argument(m),
            Fail::Unported => Failure::Other("a value this port can't rotate yet".into(), "NotImplementedError"),
        })
    });
    reply["mode"] = json!(mode);
    reply["close"] = closed.map(|c| json!(c)).unwrap_or(Value::Null);
    Ok(reply)
}
