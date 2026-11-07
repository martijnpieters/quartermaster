use std::path::{Path, PathBuf};

use crate::data_root;
use crate::git::git;
use crate::project_config::ProjectConfig;

pub const NO_PROJECT: &str = "No project";

fn normalize(path: &str) -> PathBuf {
    let expanded = data_root::expand_path(path);
    std::fs::canonicalize(&expanded).unwrap_or(expanded)
}

fn within(path: &Path, root: &Path) -> bool {
    path == root || path.starts_with(root)
}

/// The configured project whose root holds `cwd` (or the worktree's main checkout), else "No project".
pub fn project_for(cwd: Option<&str>) -> String {
    let _ = data_root::ensure_setup();
    let config = ProjectConfig::current();
    let Some(cwd) = cwd else { return NO_PROJECT.to_string() };
    let path = normalize(cwd);
    let roots: Vec<(&str, PathBuf)> = config.roots().into_iter().map(|(n, r)| (n, normalize(r))).collect();
    if let Some((name, _)) = roots.iter().find(|(_, root)| within(&path, root)) {
        return name.to_string();
    }
    if let Some(out) = git(path.to_str(), &["rev-parse", "--git-common-dir"]) {
        let common = normalize(&path.join(out).to_string_lossy());
        if let Some((name, _)) = roots.iter().find(|(_, root)| within(&common, root)) {
            return name.to_string();
        }
    }
    NO_PROJECT.to_string()
}

/// The configured repo root of a project, if it has one.
pub fn root_for(project: Option<&str>) -> Option<String> {
    let _ = data_root::ensure_setup();
    let name = project?;
    let config = ProjectConfig::current();
    config.roots().into_iter().find(|(n, _)| *n == name).map(|(_, r)| r.to_string())
}
