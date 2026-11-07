use serde_yaml::Value;

use crate::data_root;
use crate::ruby_text::strip;
use crate::yaml_scalar::{array, get, string, text};

pub struct Ide {
    pub label: String,
    pub command: Vec<String>,
}

impl Ide {
    /// The command with the checkout's path in: an element that is exactly `PATH` becomes the path, `PATH`
    /// inside a longer element (a `sh -c` script) the shell-escaped path.
    pub fn argv(&self, path: &str) -> Vec<String> {
        self.command.iter().map(|arg| if arg == "PATH" { path.to_string() } else { arg.replace("PATH", &shell_escape(path)) }).collect()
    }
}

/// `Shellwords.escape`.
pub fn shell_escape(s: &str) -> String {
    if s.is_empty() {
        return "''".into();
    }
    let mut out = String::new();
    for c in s.chars() {
        match c {
            '\n' => out.push_str("'\n'"),
            c if c.is_ascii_alphanumeric() || "_-.,:+/@".contains(c) => out.push(c),
            c => {
                out.push('\\');
                out.push(c);
            }
        }
    }
    out
}

#[derive(Clone)]
pub struct DevServer {
    pub command: String,
    pub modes: Vec<String>,
    pub default_mode: String,
    pub status_url: Option<String>,
    pub state_file: Option<String>,
}

/// Benchmark runs (docs/areas/benchmarks.md): the Postgres template a run reads (or clones), the skill
/// folder the planner follows, and extra prompt lines.
#[derive(Clone)]
pub struct Benchmark {
    pub database: String,
    pub skill: String,
    pub notes: Option<String>,
}

impl Benchmark {
    pub fn read_only_url(&self) -> String {
        format!("postgres:///{}?options=-c%20default_transaction_read_only%3Don", self.database)
    }
}

pub struct ProjectSettings {
    pub name: String,
    pub root: String,
    pub dev_server: Option<DevServer>,
    pub ide: Option<Ide>,
    pub pr_prerequisites: bool,
    pub test_db: Option<String>,
    /// The PRs view's Review button runs this skill (default `/code-review`).
    pub review_skill: String,
    pub benchmark: Option<Benchmark>,
}

/// config.yml's project side; an unreadable or malformed file reads as no projects.
pub struct ProjectConfig {
    projects: Vec<ProjectSettings>,
    ide: Option<Ide>,
    default_name: String,
    starters: Vec<Starter>,
}

/// A loose-chat starter (config.yml `loose_starters:`): its prompt has the chat load something first.
#[derive(Clone)]
pub struct Starter {
    pub key: String,
    pub label: String,
    pub prompt: String,
}

fn parse_ide(command: Option<&Value>, label: Option<&Value>) -> Option<Ide> {
    let items = match command {
        Some(Value::Sequence(items)) if !items.is_empty() => items,
        _ => return None,
    };
    if !items.iter().all(|a| matches!(a, Value::String(_) | Value::Number(_))) {
        return None;
    }
    let argv: Vec<String> = items.iter().map(text).collect();
    if strip(&argv[0]).is_empty() || argv.iter().all(|a| !a.contains("PATH")) {
        return None;
    }
    Some(Ide { label: string(label).unwrap_or_else(|| "IDE".to_string()), command: argv })
}

fn expand(path: Option<String>) -> Option<String> {
    path.map(|p| data_root::expand_path(&p).to_string_lossy().into_owned())
}

fn parse_dev_server(value: Option<&Value>) -> Option<DevServer> {
    let (command, modes_value, default_value, status_url, state_file) = match value {
        Some(Value::String(_)) => (string(value), None, None, None, None),
        Some(Value::Mapping(m)) => (string(get(m, "command")), get(m, "modes"), get(m, "default_mode"), string(get(m, "status_url")), expand(string(get(m, "state_file")))),
        _ => return None,
    };
    let command = command?;
    let default = string(default_value);
    let mut modes: Vec<String> = array(modes_value).into_iter().map(text).filter(|m| !m.is_empty()).collect();
    if modes.is_empty() {
        modes = vec![default.clone().unwrap_or_else(|| "default".to_string())];
    }
    let default_mode = match &default {
        Some(d) if modes.contains(d) => d.clone(),
        _ => modes[0].clone(),
    };
    Some(DevServer { command, modes, default_mode, status_url, state_file })
}

fn parse_project(name: String, value: &Value) -> Option<ProjectSettings> {
    let (root, raw) = match value {
        Value::Mapping(m) => (string(get(m, "root")), Some(m)),
        other => (string(Some(other)), None),
    };
    let root = expand(root)?;
    let get_raw = |key: &str| raw.and_then(|m| get(m, key));
    Some(ProjectSettings {
        name,
        root,
        dev_server: parse_dev_server(get_raw("dev_server")),
        ide: parse_ide(get_raw("ide_command"), get_raw("ide_label")),
        pr_prerequisites: matches!(get_raw("pr_prerequisites"), Some(Value::Bool(true))),
        test_db: string(get_raw("test_db")),
        review_skill: string(get_raw("review_skill")).unwrap_or_else(|| "/code-review".to_string()),
        benchmark: parse_benchmark(get_raw("benchmark")),
    })
}

/// Both database and skill, or no benchmarks.
fn parse_benchmark(value: Option<&Value>) -> Option<Benchmark> {
    let Some(Value::Mapping(m)) = value else { return None };
    let database = string(get(m, "database"))?;
    let skill = expand(string(get(m, "skill")))?;
    if database.is_empty() || !database.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_') {
        return None;
    }
    Some(Benchmark { database, skill, notes: string(get(m, "notes")) })
}

impl ProjectConfig {
    pub fn current() -> ProjectConfig {
        let text = std::fs::read_to_string(data_root::config_file()).unwrap_or_default();
        Self::parse(&text)
    }

    pub fn parse(source: &str) -> ProjectConfig {
        let data = match serde_yaml::from_str::<Value>(source) {
            Ok(Value::Mapping(m)) => m,
            _ => return ProjectConfig { projects: vec![], ide: None, default_name: String::new(), starters: vec![] },
        };
        let projects = match data.get("projects") {
            Some(Value::Mapping(m)) => m.iter().filter_map(|(k, v)| parse_project(text(k), v)).collect(),
            _ => vec![],
        };
        let default_name = match get(&data, "default_project") {
            Some(Value::String(t)) => t.clone(),
            _ => String::new(),
        };
        let mut starters: Vec<Starter> = vec![];
        if let Some(Value::Mapping(m)) = get(&data, "loose_starters") {
            for (key, starter) in m {
                let Value::Mapping(fields) = starter else { continue };
                let Some(Value::String(prompt)) = get(fields, "prompt") else { continue };
                if crate::ruby_text::strip(prompt).is_empty() {
                    continue;
                }
                let key = text(key);
                let label = match get(fields, "label") {
                    Some(Value::String(l)) if !l.is_empty() => l.clone(),
                    _ => key.clone(),
                };
                let starter = Starter { key: key.clone(), label, prompt: crate::ruby_text::strip(prompt).to_string() };
                match starters.iter_mut().find(|s| s.key == key) {
                    Some(existing) => *existing = starter,
                    None => starters.push(starter),
                }
            }
        }
        ProjectConfig { projects, ide: parse_ide(get(&data, "ide_command"), get(&data, "ide_label")), default_name, starters }
    }

    /// key → starter, in config order.
    pub fn loose_starters(&self) -> &[Starter] {
        &self.starters
    }

    /// Where an issue number with no project goes: `default_project:` when it names a configured project,
    /// else the first one.
    pub fn default_project(&self) -> Option<String> {
        if self.projects.iter().any(|p| p.name == self.default_name) {
            return Some(self.default_name.clone());
        }
        self.projects.first().map(|p| p.name.clone())
    }

    /// One dev server serves one checkout at a time across projects: the first project that configures one.
    pub fn dev_server(&self) -> Option<&DevServer> {
        self.projects.iter().find_map(|p| p.dev_server.as_ref())
    }

    /// name → root, in config order.
    pub fn roots(&self) -> Vec<(&str, &str)> {
        self.projects.iter().map(|p| (p.name.as_str(), p.root.as_str())).collect()
    }

    pub fn settings(&self, project: Option<&str>) -> Option<&ProjectSettings> {
        let name = project?;
        self.projects.iter().find(|p| p.name == name)
    }

    /// `ProjectConfig#settings(project).review_skill`: a project's own, else the default.
    pub fn review_skill(&self, project: Option<&str>) -> String {
        self.settings(project).map(|s| s.review_skill.clone()).unwrap_or_else(|| "/code-review".to_string())
    }

    /// Every project name, in config order.
    pub fn project_names(&self) -> Vec<String> {
        self.projects.iter().map(|p| p.name.clone()).collect()
    }

    /// Every dev-server command config.yml configures, once each.
    pub fn dev_server_commands(&self) -> Vec<String> {
        let mut out: Vec<String> = vec![];
        for p in &self.projects {
            if let Some(d) = &p.dev_server {
                if !out.contains(&d.command) {
                    out.push(d.command.clone());
                }
            }
        }
        out
    }

    /// The project's own IDE, else the global one.
    pub fn ide(&self, project: Option<&str>) -> Option<&Ide> {
        self.settings(project).and_then(|s| s.ide.as_ref()).or(self.ide.as_ref())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_broken_config_reads_as_no_projects() {
        assert!(ProjectConfig::parse("projects: [unclosed").roots().is_empty());
        assert!(ProjectConfig::parse("- just\n- a list").roots().is_empty());
        assert!(ProjectConfig::parse("").roots().is_empty());
    }

    #[test]
    fn projects_keep_their_order_and_need_a_root() {
        let c = ProjectConfig::parse("projects:\n  b: /srv/b\n  a:\n    root: /srv/a\n  none:\n    dev_server: x\n");
        assert_eq!(c.roots(), vec![("b", "/srv/b"), ("a", "/srv/a")]);
    }

    #[test]
    fn dev_server_modes_and_default() {
        let c = ProjectConfig::parse("projects:\n  p:\n    root: /r\n    dev_server:\n      command: ds\n      modes: [dev, tunnel]\n      default_mode: nope\n  q:\n    root: /q\n    dev_server: plain\n");
        let p = c.settings(Some("p")).unwrap().dev_server.as_ref().unwrap();
        assert_eq!((p.command.as_str(), p.default_mode.as_str()), ("ds", "dev"));
        let q = c.settings(Some("q")).unwrap().dev_server.as_ref().unwrap();
        assert_eq!((q.command.as_str(), q.default_mode.as_str()), ("plain", "default"));
    }

    #[test]
    fn ide_needs_a_path_placeholder_and_falls_back_to_the_global_one() {
        let c = ProjectConfig::parse("ide_command: [open, PATH]\nide_label: Glob\nprojects:\n  p:\n    root: /r\n    ide_command: [code, PATH]\n    ide_label: Mine\n  q:\n    root: /q\n    ide_command: [code]\n");
        assert_eq!(c.ide(Some("p")).unwrap().label, "Mine");
        assert_eq!(c.ide(Some("q")).unwrap().label, "Glob");
        assert_eq!(c.ide(None).unwrap().label, "Glob");
        assert!(ProjectConfig::parse("ide_command: [code]").ide(None).is_none());
    }

    #[test]
    fn pr_prerequisites_must_be_literally_true() {
        let c = ProjectConfig::parse("projects:\n  p:\n    root: /r\n    pr_prerequisites: true\n  q:\n    root: /q\n    pr_prerequisites: 'yes'\n");
        assert!(c.settings(Some("p")).unwrap().pr_prerequisites);
        assert!(!c.settings(Some("q")).unwrap().pr_prerequisites);
    }
}
