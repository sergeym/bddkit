//! Where plugins come from. Provisioning is machine state and never appears in
//! the test config: the test config is committed and describes the system
//! under test, while a `.so` path describes one machine.

use anyhow::{Context, Result};
use serde::Deserialize;
use std::path::{Path, PathBuf};

/// One installed plugin. `bddkit plugin install` (P2) also writes `version`,
/// `source`, `sha256`, `target`, `groups` and `abi`; P1 neither reads nor
/// declares them, because serde ignores unknown keys unless a struct asks for
/// `deny_unknown_fields` — and declaring a field just to discard it would
/// suggest the host validates something it does not. The manifest inside the
/// binary is authoritative here, not the lock file's cached copy of it.
#[derive(Debug, Clone, Deserialize)]
pub struct LockEntry {
    pub name: String,
    /// Absolute, or relative to the lock file's own directory (`.bddkit/`).
    /// `~` is NOT expanded — a `~/…` path reaches `dlopen` verbatim and fails
    /// with a confusing "no such file", so say so where the author will read it.
    pub path: PathBuf,
}

#[derive(Debug, Deserialize)]
struct LockFile {
    #[serde(default)]
    plugin: Vec<LockEntry>,
}

/// The project lock overrides the user lock entry by entry, keyed by name, so
/// a repository can pin the version its CI uses without the developer losing
/// the plugins they installed globally. Duplicate names within a single file
/// are NOT deduplicated: two entries named the same in the user file both
/// survive, while two in the project file collapse to the last one through
/// this same override loop.
pub fn load_from(project: &Path, user: Option<&Path>) -> Result<Vec<LockEntry>> {
    let mut entries: Vec<LockEntry> = Vec::new();
    if let Some(user) = user {
        entries.extend(read_file(user)?);
    }
    for entry in read_file(project)? {
        match entries.iter_mut().find(|e| e.name == entry.name) {
            Some(existing) => *existing = entry,
            None => entries.push(entry),
        }
    }
    Ok(entries)
}

/// The standard locations: `<config_dir>/.bddkit/plugins.yaml` overriding
/// `~/.config/bddkit/plugins.yaml`.
///
/// Anchored to the config file's directory, not the process working directory,
/// for the same reason `config::load` anchors the `.env` layers there: the lock
/// belongs to the suite, and `bddkit run --config suites/cfg.yaml` run from the
/// parent directory must find the same plugins as a run from inside `suites/`.
/// An unset `HOME` simply means no user lock, which is indistinguishable from a
/// `HOME` that has no lock file in it.
pub fn load_default(config_dir: &Path) -> Result<Vec<LockEntry>> {
    let project = config_dir.join(".bddkit/plugins.yaml");
    let user = std::env::var_os("HOME")
        .map(|home| PathBuf::from(home).join(".config/bddkit/plugins.yaml"));
    load_from(&project, user.as_deref())
}

fn read_file(path: &Path) -> Result<Vec<LockEntry>> {
    let raw = match std::fs::read_to_string(path) {
        Ok(raw) => raw,
        // No lock file means no plugins, which is the normal case.
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e).with_context(|| format!("failed to read {}", path.display())),
    };
    let parsed: LockFile = serde_yaml_ng::from_str(&raw)
        .with_context(|| format!("failed to parse {}", path.display()))?;
    let dir = path.parent().unwrap_or_else(|| Path::new("."));
    Ok(parsed
        .plugin
        .into_iter()
        .map(|mut entry| {
            if entry.path.is_relative() {
                entry.path = dir.join(&entry.path);
            }
            entry
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(dir: &std::path::Path, body: &str) -> std::path::PathBuf {
        std::fs::create_dir_all(dir.join(".bddkit")).expect("mkdir");
        let path = dir.join(".bddkit/plugins.yaml");
        std::fs::write(&path, body).expect("write lock");
        path
    }

    fn temp(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("bddkit-lock-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("mkdir");
        dir
    }

    #[test]
    fn parses_the_minimal_entry() {
        let dir = temp("minimal");
        write(&dir, "plugin:\n  - name: widget\n    path: /opt/libwidget.so\n");
        let entries = load_from(&dir.join(".bddkit/plugins.yaml"), None).expect("loads");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].name, "widget");
        assert_eq!(entries[0].path, std::path::PathBuf::from("/opt/libwidget.so"));
    }

    #[test]
    fn a_lock_file_carrying_p2_provisioning_fields_still_parses() {
        // sha256/target/source/version are written by `plugin install` in P2.
        // P1 declares none of them and must not reject a file that has them.
        let dir = temp("extra-fields");
        write(
            &dir,
            concat!(
                "plugin:\n  - name: widget\n    path: /opt/libwidget.so\n    version: 1.2.0\n",
                "    source: https://github.com/example/bddkit-widget\n    sha256: e3b0c442\n",
                "    target: x86_64-unknown-linux-gnu\n",
            ),
        );
        let entries = load_from(&dir.join(".bddkit/plugins.yaml"), None).expect("loads");
        assert_eq!(entries.len(), 1);
    }

    #[test]
    fn a_project_entry_overrides_a_user_entry_of_the_same_name() {
        let user = temp("user");
        let project = temp("project");
        write(&user, "plugin:\n  - name: widget\n    path: /user/libwidget.so\n  - name: mail\n    path: /user/libmail.so\n");
        write(&project, "plugin:\n  - name: widget\n    path: /project/libwidget.so\n");
        let entries = load_from(
            &project.join(".bddkit/plugins.yaml"),
            Some(&user.join(".bddkit/plugins.yaml")),
        )
        .expect("loads");
        let widget = entries.iter().find(|e| e.name == "widget").expect("widget present");
        assert_eq!(widget.path, std::path::PathBuf::from("/project/libwidget.so"));
        assert!(
            entries.iter().any(|e| e.name == "mail"),
            "a user entry the project does not override survives"
        );
    }

    #[test]
    fn a_missing_lock_file_is_not_an_error() {
        // Most runs have no plugins at all; a missing file means "none".
        let dir = temp("absent");
        let entries = load_from(&dir.join(".bddkit/plugins.yaml"), None).expect("loads");
        assert!(entries.is_empty());
    }

    #[test]
    fn a_malformed_lock_file_is_an_error() {
        let dir = temp("malformed");
        write(&dir, "plugin:\n  - name: widget\n");
        let error = load_from(&dir.join(".bddkit/plugins.yaml"), None)
            .expect_err("path is required");
        assert!(format!("{error:#}").contains("plugins.yaml"), "{error:#}");
    }

    #[test]
    fn a_lock_file_with_no_plugin_key_yields_no_entries() {
        let dir = temp("empty");
        write(&dir, "{}\n");
        let entries = load_from(&dir.join(".bddkit/plugins.yaml"), None).expect("loads");
        assert!(entries.is_empty());
    }

    #[test]
    fn a_relative_path_resolves_against_the_lock_file_directory() {
        // A committed project lock referring to ./vendor/libwidget.so must work
        // regardless of the working directory the run was started from.
        let dir = temp("relative");
        write(&dir, "plugin:\n  - name: widget\n    path: vendor/libwidget.so\n");
        let entries = load_from(&dir.join(".bddkit/plugins.yaml"), None).expect("loads");
        assert_eq!(entries[0].path, dir.join(".bddkit/vendor/libwidget.so"));
    }
}
