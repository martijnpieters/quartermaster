//! The messages a chat receives for its drafts: `DiffComments.message`, `PageAnnotations.message` and
//! `IssueComments.message`. A value of a shape only Ruby prints exactly is `Fail::Unported`.

use serde_json::{Map, Value};

use crate::ruby_text::{chomp, kernel_integer, strip};

pub enum Fail {
    Refuse(String),
    Unported,
}

type R<T> = Result<T, Fail>;

/// `value.to_s` for scalars.
fn s(v: Option<&Value>) -> R<String> {
    match v {
        None | Some(Value::Null) => Ok(String::new()),
        Some(Value::String(t)) => Ok(t.clone()),
        Some(Value::Bool(b)) => Ok(b.to_string()),
        Some(Value::Number(n)) if n.is_i64() || n.is_u64() => Ok(n.to_string()),
        _ => Err(Fail::Unported),
    }
}

fn truthy(v: Option<&Value>) -> Option<&Value> {
    match v {
        None | Some(Value::Null) | Some(Value::Bool(false)) => None,
        other => other,
    }
}

fn int(v: Option<&Value>) -> R<Option<i64>> {
    match v {
        None => Ok(None),
        Some(v) => kernel_integer(v).map_err(|_| Fail::Unported),
    }
}

fn hex(t: &str, min: usize, max: usize) -> bool {
    (min..=max).contains(&t.len()) && t.chars().all(|c| c.is_ascii_hexdigit())
}

// ---- DiffComments

/// ":12", ":12-15" for a range on one side, ":old 12-new 15" across sides.
pub fn lines_label(raw: &Map<String, Value>) -> R<String> {
    let Some(line) = int(raw.get("line"))? else { return Ok(String::new()) };
    let Some(start) = int(raw.get("start_line"))? else { return Ok(format!(":{line}")) };
    let start_side = s(truthy(raw.get("start_side")).or(raw.get("side")))?;
    let side = s(raw.get("side"))?;
    if start_side == side {
        return Ok(format!(":{start}-{line}"));
    }
    Ok(format!(":{start_side} {start}-{side} {line}"))
}

fn pr_label(raw: &Map<String, Value>) -> R<String> {
    let Some(number) = int(raw.get("number"))? else { return Ok(String::new()) };
    let Some(repo) = truthy(raw.get("repo")) else { return Ok(String::new()) };
    let commit = s(raw.get("commit"))?;
    let commit = if hex(&commit, 7, 64) { format!(" @{}", &commit[..7]) } else { String::new() };
    let repo = s(Some(repo))?;
    let mut parts: Vec<&str> = repo.split('/').collect();
    while parts.last() == Some(&"") {
        parts.pop();
    }
    Ok(format!(" on {}#{number}{commit}", parts.last().copied().unwrap_or("")))
}

/// Every line quoted; `None` when there is no text.
pub fn quote(code: Option<&Value>) -> R<Option<String>> {
    let text = s(code)?;
    let text = chomp(&text);
    if strip(text).is_empty() {
        return Ok(None);
    }
    Ok(Some(text.split('\n').map(|l| format!("> {l}")).collect::<Vec<_>>().join("\n")))
}

fn on_commits(range: &str) -> Option<String> {
    let range = strip(range);
    if hex(range, 4, 64) {
        return Some(format!("on commit {range}"));
    }
    match range.split_once("..") {
        Some((a, b)) if hex(a, 4, 64) && hex(b, 4, 64) => Some(format!("on commits {range}")),
        _ => None,
    }
}

fn diff_entry(raw: &Map<String, Value>, header_branch: &str) -> R<Option<String>> {
    let comment = strip(&s(raw.get("comment"))?).to_string();
    let path = strip(&s(raw.get("path"))?).to_string();
    if comment.is_empty() || path.is_empty() {
        return Ok(None);
    }
    let mut where_ = format!("{path}{}{}", lines_label(raw)?, pr_label(raw)?);
    let mut notes: Vec<String> = vec![];
    let side_old = raw.get("side") == Some(&Value::String("old".into()));
    let start_old = truthy(raw.get("start_side")).is_none_or(|v| v == &Value::String("old".into()));
    if side_old && start_old {
        notes.push(if truthy(raw.get("start_line")).is_some() { "removed lines" } else { "removed line" }.into());
    }
    notes.extend(on_commits(&s(raw.get("range"))?));
    let own = strip(&s(raw.get("branch"))?).to_string();
    if !own.is_empty() && own != header_branch {
        notes.push(format!("on branch {own}"));
    }
    if raw.get("committed") == Some(&Value::Bool(true)) && strip(&s(raw.get("range"))?).is_empty() && truthy(raw.get("number")).is_none() {
        notes.push("as committed, not on disk".into());
    }
    if !notes.is_empty() {
        where_ += &format!(" ({})", notes.join(", "));
    }
    let mut parts = vec![where_];
    parts.extend(quote(raw.get("code"))?);
    parts.push(comment);
    Ok(Some(parts.join("\n")))
}

pub fn diff_comments(entries: &[&Map<String, Value>], branch: &str, base: &str) -> R<String> {
    let shown = strip(branch).to_string();
    let mut branches: Vec<String> = vec![];
    for e in entries {
        let own = strip(&s(e.get("branch"))?).to_string();
        let b = if own.is_empty() { shown.clone() } else { own };
        if !branches.contains(&b) {
            branches.push(b);
        }
    }
    let header_branch = if branches.len() == 1 { branches[0].clone() } else { String::new() };
    let mut comments = vec![];
    for e in entries {
        comments.extend(diff_entry(e, &header_branch)?);
    }
    if comments.is_empty() {
        return Err(Fail::Refuse("no comments to send".into()));
    }
    let mut on = if header_branch.is_empty() { "the diff".to_string() } else { format!("the diff of {header_branch}") };
    if !strip(base).is_empty() && header_branch == shown {
        on += &format!(" (vs {})", strip(base));
    }
    let header = format!("Review comments on {on} — {} comment{}:", comments.len(), if comments.len() == 1 { "" } else { "s" });
    let mut all = vec![header];
    all.extend(comments);
    Ok(all.join("\n\n"))
}

// ---- PageAnnotations

const HTML_LIMIT: usize = 600;
const TEXT_LIMIT: usize = 200;
const PAGE_LIMIT: usize = 300;
const CONTEXT_LIMIT: usize = 300;
const COMPONENT_LIMIT: usize = 80;
const CHAIN_LIMIT: usize = 300;
const REGIONS_LIMIT: usize = 200;
const OWNER_FILE_LIMIT: usize = 200;
const OWNERS_MAX: usize = 6;

fn squish(v: Option<&Value>) -> R<String> {
    let text = s(v)?;
    let mut out = String::new();
    let mut space = false;
    for c in text.chars() {
        if matches!(c, ' ' | '\t' | '\n' | '\u{b}' | '\u{c}' | '\r') {
            space = true;
        } else {
            if space {
                out.push(' ');
            }
            space = false;
            out.push(c);
        }
    }
    if space {
        out.push(' ');
    }
    Ok(strip(&out).to_string())
}

fn squish_str(t: &str) -> String {
    squish(Some(&Value::String(t.to_string()))).unwrap_or_default()
}

fn cut(value: &str, limit: usize) -> String {
    if value.chars().count() > limit {
        format!("{}…", value.chars().take(limit - 1).collect::<String>())
    } else {
        value.to_string()
    }
}

fn page_of(raw: &Map<String, Value>, url: &str) -> R<String> {
    let page = squish(raw.get("page"))?;
    Ok(if page.is_empty() { url.to_string() } else { page })
}

/// `URI.parse(url)`'s path and query; a URL Ruby refuses stays as it is.
fn path_of(url: &str) -> R<String> {
    let allowed = |c: char| c.is_ascii_alphanumeric() || "-._~:/?#[]@!$&'()*+,;=%".contains(c);
    if !url.chars().all(allowed) {
        return Ok(url.to_string());
    }
    if url.contains(['[', ']', '%', '@']) {
        return Err(Fail::Unported);
    }
    let Some((scheme, rest)) = url.split_once("://") else { return Err(Fail::Unported) };
    if scheme.is_empty() || !scheme.starts_with(|c: char| c.is_ascii_alphabetic()) || !scheme.chars().all(|c| c.is_ascii_alphanumeric() || "+.-".contains(c)) {
        return Err(Fail::Unported);
    }
    let rest = rest.split('#').next().unwrap_or("");
    let end = rest.find(['/', '?']).unwrap_or(rest.len());
    let (authority, tail) = rest.split_at(end);
    let host = authority.split(':').next().unwrap_or("");
    if host.is_empty() || !host.chars().all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-') || authority.matches(':').count() > 1 || authority.split(':').nth(1).is_some_and(|p| !p.chars().all(|c| c.is_ascii_digit())) {
        return Err(Fail::Unported);
    }
    let (path, query) = match tail.split_once('?') {
        Some((p, q)) => (p, Some(q)),
        None => (tail, None),
    };
    let path = if path.is_empty() { "/" } else { path };
    Ok(match query {
        Some(q) => format!("{path}?{q}"),
        None => path.to_string(),
    })
}

fn owner_line(owner: &Map<String, Value>) -> Option<String> {
    let text = |k: &str, limit: usize| owner.get(k).and_then(Value::as_str).map(|t| cut(&squish_str(t), limit)).unwrap_or_default();
    let mut name = text("name", COMPONENT_LIMIT);
    if name.is_empty() {
        return None;
    }
    let alias = text("as", COMPONENT_LIMIT);
    if !alias.is_empty() && alias != name {
        name += &format!(" (as <{alias}>)");
    }
    let mut file = text("file", OWNER_FILE_LIMIT);
    if let Some(line) = owner.get("line").filter(|l| l.is_i64() || l.is_u64()) {
        if !file.is_empty() && line.as_i64().is_none_or(|n| n > 0) {
            file += &format!(":{line}");
        }
    }
    if file.is_empty() {
        return Some(format!("  - {name}"));
    }
    let via = if owner.get("via") == Some(&Value::String("definition".into())) { "defined in" } else { "JSX in" };
    Some(format!("  - {name} · {via} {file}"))
}

enum Field {
    Text(String),
    Lines(Vec<String>),
}

fn component_fields(raw: &Map<String, Value>) -> R<Vec<(&'static str, Field)>> {
    if let Some(Value::Array(list)) = raw.get("owners") {
        let rendered: Vec<String> = list.iter().filter_map(|o| o.as_object()).filter_map(owner_line).take(OWNERS_MAX).collect();
        if !rendered.is_empty() {
            return Ok(vec![("rendered by", Field::Lines(rendered))]);
        }
    }
    let chain = match raw.get("components") {
        Some(Value::Array(list)) => {
            let names: Vec<String> = list.iter().filter_map(Value::as_str).map(|n| cut(&squish_str(n), COMPONENT_LIMIT)).filter(|n| !n.is_empty()).collect();
            cut(&names.join(" < "), CHAIN_LIMIT)
        }
        _ => String::new(),
    };
    if chain.is_empty() {
        Ok(vec![("component", Field::Text(squish(raw.get("component"))?))])
    } else {
        Ok(vec![("components", Field::Text(chain))])
    }
}

fn annotation_entry(raw: &Map<String, Value>, header_page: &str, url: &str) -> R<String> {
    let comment = strip(&s(raw.get("comment"))?).to_string();
    let page = page_of(raw, url)?;
    let mut fields: Vec<(&str, Field)> = vec![
        ("page", Field::Text(if page == header_page { String::new() } else { cut(&page, PAGE_LIMIT) })),
        ("selector", Field::Text(squish(raw.get("selector"))?)),
    ];
    for (name, field) in component_fields(raw)? {
        fields.push((name, field));
    }
    fields.push(("context", Field::Text(cut(&squish(raw.get("context"))?, CONTEXT_LIMIT))));
    fields.push(("regions", Field::Text(cut(&squish(raw.get("regions"))?, REGIONS_LIMIT))));
    fields.push(("text", Field::Text(cut(&squish(raw.get("text"))?, TEXT_LIMIT))));
    fields.push(("html", Field::Text(cut(&squish(raw.get("html"))?, HTML_LIMIT))));
    let mut lines = vec![comment];
    for (name, field) in fields {
        match field {
            Field::Text(t) if t.is_empty() => {}
            Field::Text(t) => lines.push(format!("{name}: {t}")),
            Field::Lines(l) => {
                lines.push(format!("{name}:"));
                lines.extend(l);
            }
        }
    }
    Ok(lines.join("\n"))
}

pub fn page_annotations(entries: &[&Map<String, Value>], url: Option<&Value>, title: Option<&Value>) -> R<String> {
    let mut annotations = vec![];
    for e in entries {
        if !strip(&s(e.get("comment"))?).is_empty() {
            annotations.push(*e);
        }
    }
    if annotations.is_empty() {
        return Err(Fail::Refuse("no annotations to send".into()));
    }
    let url = squish(url)?;
    let mut tally: Vec<(String, usize)> = vec![];
    for a in &annotations {
        let page = page_of(a, &url)?;
        match tally.iter_mut().find(|(p, _)| *p == page) {
            Some((_, n)) => *n += 1,
            None => tally.push((page, 1)),
        }
    }
    let mut best = &tally[0];
    for t in &tally {
        if t.1 > best.1 {
            best = t;
        }
    }
    let page = best.0.clone();
    let mut blocks = vec![];
    for a in &annotations {
        blocks.push(annotation_entry(a, &page, &url)?);
    }
    let named = if page == url {
        let title = squish(title)?;
        let name = if title.is_empty() { "a page".to_string() } else { format!("the page \"{title}\"") };
        if url.is_empty() { name } else { format!("{name} ({url})") }
    } else {
        let name = cut(&page, PAGE_LIMIT);
        if url.is_empty() { name } else { format!("{name} (sent from {})", cut(&path_of(&url)?, PAGE_LIMIT)) }
    };
    let header = format!("Annotations on {named} — {} element{}:", blocks.len(), if blocks.len() == 1 { "" } else { "s" });
    let mut all = vec![header];
    all.extend(blocks);
    Ok(all.join("\n\n"))
}

// ---- IssueComments

/// `issues`: file → (title, body) as the files read now.
pub fn issue_comments(entries: &[&Map<String, Value>], issues: &[(String, String, String)], broken: &[String], dir: &str) -> R<String> {
    let mut comments = vec![];
    for e in entries {
        if !strip(&s(e.get("comment"))?).is_empty() && !s(e.get("file"))?.is_empty() {
            comments.push(*e);
        }
    }
    if comments.is_empty() {
        return Err(Fail::Refuse("no issue comments to send".into()));
    }
    let on = if strip(dir).is_empty() { "the issue drafts".to_string() } else { format!("the issue drafts in {dir}") };
    let header = format!("Comments on {on} — {} comment{}:", comments.len(), if comments.len() == 1 { "" } else { "s" });
    let mut files: Vec<String> = vec![];
    for c in &comments {
        let f = s(c.get("file"))?;
        if !files.contains(&f) {
            files.push(f);
        }
    }
    files.sort();
    let mut sections = vec![header];
    for file in files {
        let mut on_file = vec![];
        for c in &comments {
            if s(c.get("file"))? == file {
                on_file.push(*c);
            }
        }
        let issue = issues.iter().rev().find(|(f, _, _)| *f == file);
        sections.push(issue_section(&file, &on_file, issue, broken.contains(&file))?);
    }
    Ok(sections.join("\n\n"))
}

fn issue_section(file: &str, comments: &[&Map<String, Value>], issue: Option<&(String, String, String)>, broken: bool) -> R<String> {
    let title = match issue {
        Some((_, t, _)) => t.clone(),
        None => {
            let mut last = String::new();
            for c in comments {
                let t = strip(&s(c.get("title"))?).to_string();
                if !t.is_empty() {
                    last = t;
                }
            }
            last
        }
    };
    let mut heading = format!("## {file}");
    if !strip(&title).is_empty() {
        heading += &format!(" — {title}");
    }
    if issue.is_none() {
        heading += if broken { " (doesn't parse right now)" } else { " (no longer in the issue drafts)" };
    }
    let body = issue.map(|(_, _, b)| b.as_str()).unwrap_or("");
    let mut ordered: Vec<(u8, usize, usize, &Map<String, Value>)> = vec![];
    for (index, c) in comments.iter().enumerate() {
        let block = strip(&s(c.get("block"))?).to_string();
        let at: Option<usize> = if block.is_empty() { Some(0) } else { body.find(&block).map(|b| body[..b].chars().count()) };
        let (rank, pos) = if block.is_empty() { (0, 0) } else { (at.map_or(1, |_| 0), at.unwrap_or(0)) };
        // the whole-issue comments (at -1) sort before every block
        let key = if block.is_empty() { (0u8, 0usize) } else { (rank + 1, pos) };
        ordered.push((key.0, key.1, index, *c));
    }
    ordered.sort_by_key(|(a, b, i, _)| (*a, *b, *i));
    let mut parts = vec![heading];
    for (_, _, _, c) in ordered {
        parts.push(issue_comment(c, issue.map(|_| body))?);
    }
    Ok(parts.join("\n\n"))
}

fn issue_comment(entry: &Map<String, Value>, body: Option<&str>) -> R<String> {
    let block = strip(&s(entry.get("block"))?).to_string();
    let text = strip(&s(entry.get("comment"))?).to_string();
    if block.is_empty() {
        return Ok(format!("On the whole issue:\n{text}"));
    }
    let quoted = block.split_inclusive('\n').map(|l| if strip(l).is_empty() { ">".to_string() } else { format!("> {}", chomp(l)) }).collect::<Vec<_>>().join("\n");
    let mut parts = vec![quoted];
    if body.is_some_and(|b| !b.contains(&block)) {
        parts.push("(this block has changed since; the text above is what the comment was made on)".into());
    }
    parts.push(text);
    Ok(parts.join("\n"))
}
