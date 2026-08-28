//! The repo registry: `~/.config/quivive/repos` (S1-S2 of `docs/spec.md`).
//!
//! Plain text, hand-edited, one repository path per line. There are no
//! subcommands to manage it (S2) and no validation here that an entry is a
//! real, live repository: a path that turns out not to be a git checkout, or
//! not to exist at all, is a per-repo `degraded` condition for the tick to
//! report (see `reader::read`), not a reason for the registry itself to
//! refuse to hand back the list. Refusing here would take the whole fleet
//! down for one bad line.

use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

/// Read the registry and return the repository paths it names, in the order
/// they first appear.
///
/// Reads `$XDG_CONFIG_HOME/quivive/repos`, falling back to
/// `~/.config/quivive/repos` when `XDG_CONFIG_HOME` is unset or empty — the
/// same fallback the XDG base-directory spec itself defines. A missing file
/// is an empty registry (S2), not an error: this is the resting state of a
/// machine that has not hand-edited the file yet, not a fault. Any other I/O
/// failure (permissions, a directory where the file should be, ...) is
/// surfaced as an error naming the path, because a registry that cannot be
/// read is not silently the same as one with nothing in it.
pub fn read() -> Result<Vec<PathBuf>> {
    let path = registry_path()?;
    parse(&path)
}

/// Where the registry file is, honouring `XDG_CONFIG_HOME`. Exposed
/// separately from [`read`] so callers (and tests) can name the path a
/// diagnostic is about without re-deriving it.
pub fn registry_path() -> Result<PathBuf> {
    let config_home = match std::env::var_os("XDG_CONFIG_HOME") {
        Some(dir) if !dir.is_empty() => PathBuf::from(dir),
        _ => home_dir()?.join(".config"),
    };
    Ok(config_home.join("quivive").join("repos"))
}

/// Parse a registry file already resolved to a path. Split from [`read`] so
/// tests can point it at a fixture without going through env vars at all.
fn parse(path: &Path) -> Result<Vec<PathBuf>> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        // S2, verbatim: a missing file is an empty registry, not an error.
        Err(e) if e.kind() == ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e).with_context(|| format!("reading registry {}", path.display())),
    };

    let home = home_dir().ok();
    let mut seen = std::collections::HashSet::new();
    let mut repos = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        // A blank line, or one whose first non-blank character is `#`, is
        // ignored. Deliberately NOT an inline `#` comment: a repository path
        // may legally contain a `#` (an unusual but valid path byte on every
        // platform quivive runs on), and a comment rule that eats the tail of
        // a real path is worse than having no inline-comment syntax at all.
        if line.is_empty() || line.starts_with('#') {
            continue;
        }

        let expanded = expand_tilde(line, home.as_deref());
        // Two lines naming the same repo must not tick it twice — S7's
        // per-agent state and S8's per-repo status are both keyed on the
        // repo path, so a duplicate entry would double-count that repo's
        // agents rather than just being redundant. Dedupe on the expanded
        // path, keep the first occurrence's order, so a hand-edited file
        // reads top-to-bottom the way its author wrote it.
        if seen.insert(expanded.clone()) {
            repos.push(expanded);
        }
    }
    Ok(repos)
}

/// Expand a single leading `~` to `$HOME`. Only a bare leading `~` or a
/// leading `~/...` is expanded (no `~user` — nothing in quivive needs another
/// user's home, and resolving one portably needs a crate outside the
/// dependency budget in `Cargo.toml`). A `~` anywhere else in the line is left
/// alone: it is not a home-directory reference there.
fn expand_tilde(line: &str, home: Option<&Path>) -> PathBuf {
    let Some(home) = home else {
        return PathBuf::from(line);
    };
    match line.strip_prefix('~') {
        Some("") => home.to_path_buf(),
        Some(rest) if rest.starts_with('/') => home.join(&rest[1..]),
        _ => PathBuf::from(line),
    }
}

/// `$HOME`, the one piece of environment every expansion and fallback in this
/// module needs. An error rather than a silent empty path: a registry read
/// with no `$HOME` to expand `~` against would otherwise produce paths that
/// quietly point at the filesystem root.
fn home_dir() -> Result<PathBuf> {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .filter(|p| !p.as_os_str().is_empty())
        .context("$HOME is not set; cannot locate the registry or expand `~`")
}

#[cfg(test)]
mod tests {
    use std::fs;
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;

    use tempfile::TempDir;

    use super::*;

    /// Points `XDG_CONFIG_HOME` (and, when needed, `HOME`) at a fresh tempdir
    /// for the life of the guard, then restores whatever was there before —
    /// so tests never read or perturb the real `$HOME`. `cargo test`'s
    /// default harness runs every test in a parallel thread of the SAME
    /// process, and env vars are process-global — two tests setting
    /// `XDG_CONFIG_HOME` at once is a real race, caught empirically running
    /// this file's tests together rather than one at a time. A process-wide
    /// mutex, held for the guard's lifetime, serializes every test that
    /// touches the environment so each one's view of it is uninterrupted.
    struct EnvGuard {
        _lock: std::sync::MutexGuard<'static, ()>,
        saved: Vec<(&'static str, Option<std::ffi::OsString>)>,
    }

    impl EnvGuard {
        fn set(vars: &[(&'static str, &Path)]) -> Self {
            static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
            // A prior test panicking while it held the lock must not poison
            // every test after it — recover the guard rather than propagate.
            let lock = LOCK.lock().unwrap_or_else(|e| e.into_inner());
            let saved = vars
                .iter()
                .map(|(key, value)| {
                    let prior = std::env::var_os(key);
                    unsafe { std::env::set_var(key, value) };
                    (*key, prior)
                })
                .collect();
            Self { _lock: lock, saved }
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            for (key, prior) in self.saved.drain(..) {
                match prior {
                    Some(v) => unsafe { std::env::set_var(key, v) },
                    None => unsafe { std::env::remove_var(key) },
                }
            }
        }
    }

    fn write_registry(dir: &TempDir, contents: &str) -> PathBuf {
        let quivive_dir = dir.path().join("quivive");
        fs::create_dir_all(&quivive_dir).unwrap();
        let path = quivive_dir.join("repos");
        fs::write(&path, contents).unwrap();
        path
    }

    #[test]
    fn missing_file_is_an_empty_registry_not_an_error() {
        let xdg = TempDir::new().unwrap();
        let _g = EnvGuard::set(&[("XDG_CONFIG_HOME", xdg.path())]);
        // Note: no file written under xdg at all.
        assert_eq!(read().unwrap(), Vec::<PathBuf>::new());
    }

    #[test]
    fn blank_lines_and_comment_lines_are_ignored() {
        let xdg = TempDir::new().unwrap();
        write_registry(
            &xdg,
            "\n  \n# a comment\n/repo/one\n   # indented comment\n/repo/two\n",
        );
        let _g = EnvGuard::set(&[("XDG_CONFIG_HOME", xdg.path())]);
        assert_eq!(
            read().unwrap(),
            vec![PathBuf::from("/repo/one"), PathBuf::from("/repo/two")]
        );
    }

    #[test]
    fn inline_hash_is_not_a_comment_because_paths_can_contain_one() {
        let xdg = TempDir::new().unwrap();
        write_registry(&xdg, "/repo/weird#name\n");
        let _g = EnvGuard::set(&[("XDG_CONFIG_HOME", xdg.path())]);
        assert_eq!(read().unwrap(), vec![PathBuf::from("/repo/weird#name")]);
    }

    #[test]
    fn tilde_expands_against_home() {
        let xdg = TempDir::new().unwrap();
        let home = TempDir::new().unwrap();
        write_registry(&xdg, "~\n~/code/repo\n");
        let _g = EnvGuard::set(&[("XDG_CONFIG_HOME", xdg.path()), ("HOME", home.path())]);
        assert_eq!(
            read().unwrap(),
            vec![home.path().to_path_buf(), home.path().join("code/repo")]
        );
    }

    #[test]
    fn tilde_not_at_line_start_is_left_alone() {
        let xdg = TempDir::new().unwrap();
        let home = TempDir::new().unwrap();
        write_registry(&xdg, "/repo/a~b\n");
        let _g = EnvGuard::set(&[("XDG_CONFIG_HOME", xdg.path()), ("HOME", home.path())]);
        assert_eq!(read().unwrap(), vec![PathBuf::from("/repo/a~b")]);
    }

    #[test]
    fn xdg_config_home_overrides_the_dotconfig_fallback() {
        let xdg = TempDir::new().unwrap();
        let home = TempDir::new().unwrap();
        // A file under ~/.config/quivive/repos that must NOT be read, to prove
        // the override actually took effect rather than both paths agreeing
        // by coincidence.
        let dotconfig = home.path().join(".config").join("quivive");
        fs::create_dir_all(&dotconfig).unwrap();
        fs::write(dotconfig.join("repos"), "/should/not/be/read\n").unwrap();

        write_registry(&xdg, "/from/xdg\n");
        let _g = EnvGuard::set(&[("XDG_CONFIG_HOME", xdg.path()), ("HOME", home.path())]);
        assert_eq!(read().unwrap(), vec![PathBuf::from("/from/xdg")]);
    }

    #[test]
    fn empty_xdg_config_home_falls_back_to_dotconfig() {
        let home = TempDir::new().unwrap();
        let dotconfig = home.path().join(".config").join("quivive");
        fs::create_dir_all(&dotconfig).unwrap();
        fs::write(dotconfig.join("repos"), "/from/dotconfig\n").unwrap();

        // An empty value is unset in every shell quivive runs under; treat it
        // the same as absent rather than as "config home is the empty path".
        let _g = EnvGuard::set(&[("XDG_CONFIG_HOME", Path::new("")), ("HOME", home.path())]);
        assert_eq!(read().unwrap(), vec![PathBuf::from("/from/dotconfig")]);
    }

    #[test]
    fn duplicate_lines_are_deduped_keeping_first_occurrence_order() {
        let xdg = TempDir::new().unwrap();
        write_registry(&xdg, "/repo/a\n/repo/b\n/repo/a\n/repo/c\n/repo/b\n");
        let _g = EnvGuard::set(&[("XDG_CONFIG_HOME", xdg.path())]);
        assert_eq!(
            read().unwrap(),
            vec![
                PathBuf::from("/repo/a"),
                PathBuf::from("/repo/b"),
                PathBuf::from("/repo/c"),
            ]
        );
    }

    #[test]
    fn duplicate_after_tilde_expansion_still_dedupes() {
        let xdg = TempDir::new().unwrap();
        let home = TempDir::new().unwrap();
        // The third line names the same repo as the first, but spelled out in
        // full rather than with `~` — expansion must happen before the
        // dedupe check, or this would (wrongly) keep both.
        let expanded = home.path().join("code/repo").display().to_string();
        write_registry(&xdg, &format!("~/code/repo\n/repo/other\n{expanded}\n"));
        let _g = EnvGuard::set(&[("XDG_CONFIG_HOME", xdg.path()), ("HOME", home.path())]);
        assert_eq!(
            read().unwrap(),
            vec![home.path().join("code/repo"), PathBuf::from("/repo/other")]
        );
    }

    #[cfg(unix)]
    #[test]
    fn unreadable_file_is_an_error_naming_the_path() {
        let xdg = TempDir::new().unwrap();
        let path = write_registry(&xdg, "/repo/one\n");
        let mut perms = fs::metadata(&path).unwrap().permissions();
        perms.set_mode(0o000);
        fs::set_permissions(&path, perms).unwrap();

        let _g = EnvGuard::set(&[("XDG_CONFIG_HOME", xdg.path())]);
        let result = read();
        // Root (routine in a CI container) ignores permission bits entirely
        // and reads the file anyway, so a 0o000 file is not a usable fixture
        // for "unreadable" there — the case this test exists to cover simply
        // cannot be produced, so restore permissions and skip rather than
        // false-fail on a check the environment cannot perform.
        let mut restore = fs::metadata(&path).unwrap().permissions();
        restore.set_mode(0o644);
        let _ = fs::set_permissions(&path, restore);
        let Err(err) = result else {
            return;
        };
        assert!(
            err.to_string().contains(&path.display().to_string()),
            "error should name the path: {err}"
        );
    }
}
