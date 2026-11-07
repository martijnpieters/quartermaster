//! `qm ask QUERY`: finds work items by meaning. A compact index of every item goes to headless
//! `claude -p --model sonnet` on stdin and the answer comes back as schema-validated JSON — the same
//! call, with the same prompt, arguments and index, as the Ruby `AskSearch`. Anything this port can't
//! word exactly as Ruby does (claude missing, an answer whose shape makes Ruby raise) goes to Ruby.

use std::io::Write;
use std::process::{Command, Stdio};

use regex::Regex;
use serde_json::Value;

use crate::cli_format::puts;
use crate::outcome::{abort, done, Outcome};
use crate::ruby_text::strip;
use crate::work_item::WorkItem;

const LIMIT: usize = 8;
const DESCRIPTION_CHARS: usize = 800;
const NOTES_CHARS: usize = 2_000;
const NOTE_CHARS: usize = 300;
const ANSWER_SCHEMA: &str = r#"{"type":"object","required":["hits"],"properties":{"hits":{"type":"array","items":{"type":"object","required":["slug","why"],"properties":{"slug":{"type":"string"},"why":{"type":"string"}}}}}}"#;

fn prompt(query: &str) -> String {
    format!(
        "You are a search engine over a developer's work items (stdin holds one\n\
         block per item: slug, title, state, project, branches, description and\n\
         its newest notes). The developer half-remembers an item and describes\n\
         it below. Return the items that match, by meaning rather than keywords,\n\
         best first and at most {LIMIT}; each with a one-sentence reason that\n\
         names the note or detail that matches. Leave out weak guesses: an empty\n\
         list is the right answer when nothing plausibly matches.\n\
         \n\
         Query: {query}\n"
    )
}

fn clip(text: &str, max: usize) -> String {
    let text = Regex::new(r"[ \t\r\n\x0c\x0b]+").unwrap().replace_all(text, " ").into_owned();
    if text.chars().count() > max { format!("{}…", text.chars().take(max).collect::<String>()) } else { text }
}

fn newest_notes(item: &WorkItem) -> Vec<String> {
    let mut budget = NOTES_CHARS as i64;
    let mut kept = vec![];
    for note in item.notes() {
        let text = format!("{} {}", note.stamp, clip(&note.text, NOTE_CHARS));
        let size = text.chars().count() as i64;
        if size > budget {
            break;
        }
        budget -= size;
        kept.push(text);
    }
    kept
}

fn entry(item: &WorkItem) -> String {
    let mut lines = vec![format!("## {}", item.slug), format!("title: {}", item.title()), format!("state: {}", item.state().unwrap_or_default()), format!("project: {}", item.project().unwrap_or_default())];
    let branches = item.branches();
    if !branches.is_empty() {
        lines.push(format!("branches: {}", branches.join(", ")));
    }
    if let Some(n) = item.issue_number() {
        lines.push(format!("issue: #{n}"));
    }
    let description = item.description();
    if !description.is_empty() {
        lines.push(format!("description: {}", clip(&description, DESCRIPTION_CHARS)));
    }
    let notes = newest_notes(item);
    if !notes.is_empty() {
        lines.push("notes:".into());
        lines.extend(notes.into_iter().map(|n| format!("- {n}")));
    }
    lines.join("\n")
}

/// `value.to_s` for what a model may put in "why"; `None` where Ruby's would be an `inspect`.
fn shown(v: Option<&Value>) -> Option<String> {
    match v {
        None | Some(Value::Null) => Some(String::new()),
        Some(Value::String(s)) => Some(s.clone()),
        Some(Value::Bool(b)) => Some(b.to_string()),
        Some(Value::Number(n)) if n.is_i64() || n.is_u64() => Some(n.to_string()),
        _ => None,
    }
}

/// Why a search did not produce hits: `Ruby`'s `ArgumentError` message, or something only Ruby words exactly.
pub enum Fail {
    Message(String),
    Ruby,
}

/// `AskSearch#call`: the matching items, best first, each with the model's reason.
pub fn search(query: &str) -> Result<Vec<(String, String)>, Fail> {
    let query = strip(query);
    if query.is_empty() {
        return Err(Fail::Message("query is empty".into()));
    }
    let items = WorkItem::all().map_err(|e| Fail::Message(e.message))?;
    let index = items.iter().map(entry).collect::<Vec<_>>().join("\n\n");
    let spawned = Command::new("claude")
        .args(["-p", &prompt(query), "--model", "sonnet", "--tools", "", "--strict-mcp-config", "--no-session-persistence", "--output-format", "json", "--json-schema", ANSWER_SCHEMA])
        .env("QM_SKIP", "1")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn();
    let Ok(mut child) = spawned else { return Err(Fail::Ruby) };
    let mut stdin = child.stdin.take().unwrap();
    let writer = std::thread::spawn(move || {
        let _ = stdin.write_all(index.as_bytes());
    });
    let Ok(output) = child.wait_with_output() else { return Err(Fail::Ruby) };
    let _ = writer.join();
    let (out, err) = (String::from_utf8_lossy(&output.stdout).into_owned(), String::from_utf8_lossy(&output.stderr).into_owned());
    if !output.status.success() {
        let why = if strip(&err).is_empty() { strip(&out) } else { strip(&err) };
        return Err(Fail::Message(format!("claude -p failed: {why}")));
    }
    let head = |s: &str| strip(s).chars().take(200).collect::<String>();
    let answer = match serde_json::from_str::<Value>(&out) {
        Ok(Value::Object(o)) => o,
        Ok(_) => return Err(Fail::Ruby),
        Err(_) => return Err(Fail::Message(format!("unparseable answer from claude: {}", head(&out)))),
    };
    let hits = match answer.get("structured_output") {
        None | Some(Value::Null) => None,
        Some(Value::Object(s)) => s.get("hits"),
        Some(_) => return Err(Fail::Ruby),
    };
    let Some(Value::Array(hits)) = hits else { return Err(Fail::Message(format!("unexpected answer from claude: {}", head(&out)))) };
    let mut found: Vec<(String, String)> = vec![];
    for hit in hits {
        let Value::Object(h) = hit else { continue };
        let Some(Value::String(slug)) = h.get("slug") else { continue };
        if !items.iter().any(|i| &i.slug == slug) || found.iter().any(|(s, _)| s == slug) {
            continue;
        }
        let Some(why) = shown(h.get("why")) else { return Err(Fail::Ruby) };
        found.push((slug.clone(), why));
    }
    found.truncate(LIMIT);
    Ok(found)
}

pub fn ask(args: &[String]) -> Outcome {
    let query = args.join(" ");
    if strip(&query).is_empty() {
        return abort("usage: qm ask QUERY");
    }
    match search(&query) {
        Ok(found) if found.is_empty() => done("No work item matches.\n".into()),
        Ok(found) => done(found.iter().map(|(slug, why)| puts(&format!("{slug}\n    {why}"))).collect()),
        Err(Fail::Message(m)) => abort(&format!("qm: {m}")),
        Err(Fail::Ruby) => Outcome::Delegate,
    }
}
