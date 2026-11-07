//! `drafts-send` and `issues-create`: what the tray and the issue drafts say to the item's chat, pasted
//! into its pane as one message.

use serde_json::{json, Map, Value};

use crate::draft_messages::{self as m, Fail};
use crate::drafts::{self, update};
use crate::gui_actions::{fail, find, ok, refuse_mid_turn, slug_shaped, Stop};
use crate::ruby_text::strip;
use crate::tmux::Tmux;

type R<T> = Result<T, Stop>;

const CHAT_KINDS: [&str; 3] = ["diff-comment", "annotation", "issue-comment"];

fn stop(f: Fail) -> Stop {
    match f {
        Fail::Refuse(msg) => Stop::refuse(msg),
        Fail::Unported => Stop::Unported,
    }
}

fn text(v: Option<&Value>) -> R<String> {
    match v {
        None | Some(Value::Null) => Ok(String::new()),
        Some(Value::String(t)) => Ok(t.clone()),
        Some(Value::Bool(b)) => Ok(b.to_string()),
        Some(Value::Number(n)) if n.is_i64() || n.is_u64() => Ok(n.to_string()),
        _ => Err(Stop::Unported),
    }
}

fn paste_into_chat(item: &crate::work_item::WorkItem, message: &str) -> R<()> {
    refuse_mid_turn(item)?;
    let tmux = Tmux::new();
    if !tmux.paste(&item.slug, message) {
        return Err(fail(format!("could not paste into {}'s chat", item.slug)));
    }
    std::thread::sleep(std::time::Duration::from_millis(500));
    if !tmux.send_enter(&item.slug) {
        return Err(fail(format!("could not submit in {}'s chat", item.slug)));
    }
    Ok(())
}

fn issue_data(slug: &str) -> R<(Vec<(String, String, String)>, Vec<String>, Value)> {
    let data = crate::issue_drafts::read(slug).ok_or(Stop::Unported)?;
    let field = |v: &Value, k: &str| v.get(k).and_then(Value::as_str).unwrap_or("").to_string();
    let issues = data["issues"].as_array().map(|l| l.iter().map(|i| (field(i, "file"), field(i, "title"), field(i, "body"))).collect()).unwrap_or_default();
    let broken = data["errors"].as_array().map(|l| l.iter().map(|e| field(e, "file")).collect()).unwrap_or_default();
    Ok((issues, broken, data))
}

pub(crate) fn drafts_send(slug: &str, args: &Map<String, Value>) -> R<Value> {
    let item = find(slug)?;
    if !Tmux::new().live(slug) {
        return Err(fail(format!("{slug} has no chat open in the GUI")));
    }
    if !slug_shaped(&item.slug) {
        return Err(fail(format!("'{}' is not a work item slug", item.slug)));
    }
    let all = drafts::entries(&item.slug).ok_or(Stop::Unported)?;
    let mut entries: Vec<Map<String, Value>> = all.into_iter().filter(|e| e.get("kind").and_then(Value::as_str).is_some_and(|k| CHAT_KINDS.contains(&k))).collect();
    if let Some(Value::Array(ids)) = args.get("ids") {
        entries.retain(|e| ids.contains(e.get("id").unwrap_or(&Value::Null)));
    }
    if entries.is_empty() {
        return Err(fail("no drafts to send"));
    }
    let message = message(&item.slug, &entries, args)?;
    paste_into_chat(&item, &message)?;
    let revs: Vec<(Value, Value)> = entries.iter().map(|e| (e.get("id").cloned().unwrap_or(Value::Null), e.get("rev").cloned().unwrap_or(Value::Null))).collect();
    let gone = update(&item.slug, |list| {
        let before = list.len();
        list.retain(|e| {
            let id = e.get("id").cloned().unwrap_or(Value::Null);
            let want = revs.iter().rev().find(|(k, _)| *k == id).map(|(_, v)| v.clone()).unwrap_or(Value::Null);
            want != e.get("rev").cloned().unwrap_or(Value::Null)
        });
        Ok(before - list.len())
    });
    if gone.is_err() {
        return Err(Stop::Unported);
    }
    let mut out = ok();
    out["sent"] = json!(entries.len());
    Ok(out)
}

fn message(slug: &str, entries: &[Map<String, Value>], args: &Map<String, Value>) -> R<String> {
    let of = |kind: &str| -> Vec<&Map<String, Value>> { entries.iter().filter(|e| e.get("kind") == Some(&json!(kind))).collect() };
    let mut comments = of("diff-comment");
    let annotations = of("annotation");
    let issue_comments = of("issue-comment");
    let mut keyed = vec![];
    for (index, e) in comments.iter().enumerate() {
        let line = crate::transcript_usage::to_i(e.get("line")).map_err(|_| Stop::Unported)?;
        keyed.push((text(e.get("path"))?, line, index));
    }
    let mut order: Vec<usize> = (0..comments.len()).collect();
    order.sort_by(|a, b| keyed[*a].cmp(&keyed[*b]));
    comments = order.into_iter().map(|i| comments[i]).collect();
    let mut sections = vec![];
    if !comments.is_empty() {
        let branch = text(args.get("branch"))?;
        let base = text(args.get("base"))?;
        sections.push(m::diff_comments(&comments, &branch, &base).map_err(stop)?);
    }
    if !annotations.is_empty() {
        let mut paged: Vec<Map<String, Value>> = vec![];
        for e in &annotations {
            let mut e = (*e).clone();
            if strip(&text(e.get("page"))?).is_empty() {
                let url = e.get("url").cloned().unwrap_or(Value::Null);
                e.insert("page".into(), url);
            }
            paged.push(e);
        }
        let refs: Vec<&Map<String, Value>> = paged.iter().collect();
        let last = annotations.last().unwrap();
        sections.push(m::page_annotations(&refs, last.get("url"), last.get("title")).map_err(stop)?);
    }
    if !issue_comments.is_empty() {
        let (issues, broken, _) = issue_data(slug)?;
        let dir = crate::cli_format::short_path(Some(&crate::issue_drafts::dir(slug).to_string_lossy()));
        sections.push(m::issue_comments(&issue_comments, &issues, &broken, &dir).map_err(stop)?);
    }
    if sections.is_empty() {
        return Err(fail("no drafts of a kind this qm can send"));
    }
    Ok(sections.join("\n\n"))
}

pub(crate) fn issues_create(slug: &str, files: &Value) -> R<Value> {
    let item = find(slug)?;
    if !Tmux::new().live(slug) {
        return Err(fail(format!("{slug} has no chat open in the GUI")));
    }
    let (issues, broken, data) = issue_data(&item.slug)?;
    let created = |file: &str| data["issues"].as_array().is_some_and(|l| l.iter().any(|i| i["file"] == file && i["created"] == Value::Bool(true)));
    let uncreated: Vec<&(String, String, String)> = issues.iter().filter(|(f, _, _)| !created(f)).collect();
    let chosen: Vec<&(String, String, String)> = match files {
        Value::Array(list) => {
            if list.is_empty() {
                return Err(fail("no issue drafts chosen"));
            }
            let mut out: Vec<&(String, String, String)> = vec![];
            for file in list {
                let found = file.as_str().and_then(|f| uncreated.iter().find(|(n, _, _)| n == f));
                let Some(found) = found else {
                    let name = text(Some(file))?;
                    let why = if issues.iter().any(|(n, _, _)| *n == name) {
                        format!("{name} is already created on GitHub")
                    } else if broken.contains(&name) {
                        format!("{name} doesn't parse — fix it first")
                    } else {
                        format!("no issue draft {name}")
                    };
                    return Err(fail(why));
                };
                if !out.iter().any(|(n, _, _)| *n == found.0) {
                    out.push(found);
                }
            }
            out
        }
        _ => uncreated,
    };
    if chosen.is_empty() {
        return Err(fail("no issue drafts left to create"));
    }
    let dir = crate::cli_format::short_path(Some(&crate::issue_drafts::dir(&item.slug).to_string_lossy()));
    let count = format!("{} drafted issue{}", chosen.len(), if chosen.len() == 1 { "" } else { "s" });
    let list: Vec<String> = chosen.iter().map(|(f, t, _)| format!("- {f} — {t}")).collect();
    let message = [format!("Create {count} on GitHub, per the /draft-issues skill's create step. The files are in {dir}:"), list.join("\n"), "Then add `number: <n>` to each created file's frontmatter.".to_string()].join("\n\n");
    paste_into_chat(&item, &message)?;
    let mut out = ok();
    out["requested"] = json!(chosen.iter().map(|(f, _, _)| f.clone()).collect::<Vec<_>>());
    Ok(out)
}
