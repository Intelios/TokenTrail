//! Stable identity for the working directory a harness recorded.
//!
//! A project used to be nothing but the path string in `usage_event.project`, so a
//! folder that was moved, renamed, cloned again or checked out as a linked git
//! worktree became a brand-new project that had to be merged by hand. This module
//! answers the questions needed to recognise it again: what is the same path
//! (normalisation), which repository does it belong to (`.git` directory, linked
//! worktree, submodule), and which of those facts survive a move (the remote URL
//! and, on the same volume, the `.git` directory's inode).

use std::path::{Path, PathBuf};

/// What `inspect` found out about one recorded folder.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Repo {
    /// The folder's normalised path — the key alias rows are stored under.
    pub alias_key: String,
    /// The repository the folder belongs to: its root, or the main worktree root
    /// when the folder is a linked worktree. `None` outside any repository.
    pub root: Option<String>,
    /// Normalised URL of `origin` (else the first remote), when there is one.
    pub remote: Option<String>,
    /// `dev:ino` of the repository's `.git`: survives `mv`, not a copy.
    pub inode: Option<String>,
    /// The repository was reached through a linked worktree (`commondir`).
    pub worktree: bool,
}

impl Repo {
    /// Identity facts that can name this repository again after it moves.
    /// Ordered strongest first: a shared remote is deliberate, an inode is
    /// circumstantial (it only proves the same directory did not move volumes).
    pub fn identities(&self) -> Vec<(&'static str, &str)> {
        let mut out = Vec::new();
        if let Some(remote) = self.remote.as_deref() {
            out.push(("remote", remote));
        }
        if let Some(inode) = self.inode.as_deref() {
            out.push(("inode", inode));
        }
        out
    }
}

/// `$HOME`, empty when unset. Only project ignore rules need it.
pub fn home_dir() -> PathBuf {
    std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default()
}

/// The key a recorded folder is stored under: `file://` URIs and percent escapes
/// resolved (Antigravity writes workspace URIs), trailing slashes dropped, and
/// the on-disk spelling used when the folder still exists, so a symlinked or
/// differently-cased path is the same project as the real one.
///
/// Synthetic names (`WackChatter · Lyra`) are not paths and pass through.
pub fn normalize_project_path(raw: &str) -> String {
    let mut s = raw.trim();
    let mut uri = false;
    if let Some(rest) = s.strip_prefix("file://") {
        // file:///Users/x and file://localhost/Users/x both name a local path.
        uri = true;
        s = match rest.find('/') {
            Some(0) => rest,
            Some(i) => &rest[i..],
            None => rest,
        };
    }
    // Escapes only mean anything in a path: a character name containing `%20`
    // must not be rewritten.
    let decoded = if uri || s.starts_with('/') {
        percent_decode(s)
    } else {
        s.to_string()
    };
    if !decoded.starts_with('/') {
        return decoded;
    }
    let trimmed = decoded.trim_end_matches('/');
    let trimmed = if trimmed.is_empty() { "/" } else { trimmed };
    match std::fs::canonicalize(trimmed) {
        Ok(p) => p.to_string_lossy().into_owned(),
        Err(_) => trimmed.to_string(),
    }
}

fn percent_decode(s: &str) -> String {
    if !s.contains('%') {
        return s.to_string();
    }
    let bytes = s.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let (Some(hi), Some(lo)) = (hex(bytes[i + 1]), hex(bytes[i + 2])) {
                out.push(hi * 16 + lo);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn hex(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

/// `host/owner/repo`, lowercase, protocol and `.git` gone. `git@host:owner/repo`
/// and `https://host/owner/repo` name the same repository.
pub fn normalize_remote(url: &str) -> Option<String> {
    let url = url.trim();
    if url.is_empty() {
        return None;
    }
    let (host, path) = match url.split_once("://") {
        Some((_, rest)) => {
            let (authority, path) = rest.split_once('/')?;
            let host = authority.rsplit('@').next().unwrap_or(authority);
            let host = host.split(':').next().unwrap_or(host);
            (host.to_string(), path.to_string())
        }
        None => {
            // scp-style `git@host:owner/repo`, and bare `host:owner/repo`.
            let (authority, path) = url.split_once(':')?;
            let host = authority.rsplit('@').next().unwrap_or(authority);
            if host.contains('/') {
                return None;
            }
            (host.to_string(), path.to_string())
        }
    };
    let path = path.trim_matches('/');
    let path = path.strip_suffix(".git").unwrap_or(path);
    if host.is_empty() || path.is_empty() {
        return None;
    }
    Some(format!("{}/{}", host.to_lowercase(), path.to_lowercase()))
}

/// Everything needed to place `raw` in a project: its alias key, the repository
/// above it and the identity of that repository.
pub fn inspect(raw: &str) -> Repo {
    let alias_key = normalize_project_path(raw);
    let mut repo = Repo {
        alias_key: alias_key.clone(),
        ..Default::default()
    };
    if !alias_key.starts_with('/') {
        return repo;
    }
    let mut dir = Path::new(&alias_key);
    loop {
        let dot_git = dir.join(".git");
        if let Ok(meta) = std::fs::metadata(&dot_git) {
            if meta.is_dir() {
                repo.root = Some(dir.to_string_lossy().into_owned());
                repo.remote = remote_of(&dot_git);
                repo.inode = inode_key(&dot_git);
                return repo;
            }
            if let Some(gitdir) = gitdir_file(&dot_git) {
                // A linked worktree shares the main checkout's config and `.git`;
                // a submodule has its own gitdir and no `commondir`, and stays a
                // project of its own so vendored checkouts are not swallowed.
                if let Some(main) = worktree_main(&gitdir) {
                    if let Some(main_root) = main.parent().filter(|r| r.is_dir()) {
                        repo.root = Some(main_root.to_string_lossy().into_owned());
                        repo.remote = remote_of(&main);
                        repo.inode = inode_key(&main);
                        repo.worktree = true;
                        return repo;
                    }
                }
                repo.root = Some(dir.to_string_lossy().into_owned());
                repo.remote = remote_of(&gitdir);
                repo.inode = inode_key(&gitdir);
                return repo;
            }
            // A `.git` we cannot read: treat the folder as the root, as before.
            repo.root = Some(dir.to_string_lossy().into_owned());
            return repo;
        }
        match dir.parent() {
            Some(parent) => dir = parent,
            None => return repo,
        }
    }
}

/// The project a recorded folder belongs to: the nearest enclosing repository
/// (main worktree for a linked worktree), else the folder itself.
///
/// Missing directories are walked through rather than rejected, so a deleted deep
/// folder still lands on the repository above it. The store's ladder reads `inspect`
/// directly; this is the filesystem half of it, kept for the tests that pin the rule.
#[cfg(test)]
pub fn resolve_project_root(path: &str) -> String {
    let repo = inspect(path);
    repo.root.unwrap_or(repo.alias_key)
}

/// `gitdir: <path>` from a `.git` file, resolved against the folder holding it.
fn gitdir_file(dot_git: &Path) -> Option<PathBuf> {
    let text = std::fs::read_to_string(dot_git).ok()?;
    let line = text
        .lines()
        .find(|l| l.trim_start().starts_with("gitdir:"))?
        .trim();
    let target = line.strip_prefix("gitdir:")?.trim();
    if target.is_empty() {
        return None;
    }
    let path = Path::new(target);
    let resolved = match (dot_git.parent(), path.is_absolute()) {
        (_, true) => path.to_path_buf(),
        (Some(parent), false) => parent.join(path),
        (None, false) => return None,
    };
    Some(resolved)
}

/// The main checkout's `.git` for a linked worktree: worktree gitdirs carry a
/// `commondir` file, submodule gitdirs do not.
fn worktree_main(gitdir: &Path) -> Option<PathBuf> {
    let text = std::fs::read_to_string(gitdir.join("commondir")).ok()?;
    let target = text.trim();
    if target.is_empty() {
        return None;
    }
    let path = Path::new(target);
    let main = if path.is_absolute() {
        path.to_path_buf()
    } else {
        gitdir.join(path)
    };
    let main = lexical(&main);
    (main.file_name()? == ".git").then_some(main)
}

/// Resolve `.` and `..` without touching the filesystem: `commondir` is a relative
/// path and its target may not exist (a deleted main checkout).
fn lexical(path: &Path) -> PathBuf {
    use std::path::Component;
    let mut out = PathBuf::new();
    for part in path.components() {
        match part {
            Component::ParentDir => {
                out.pop();
            }
            Component::CurDir => {}
            other => out.push(other.as_os_str()),
        }
    }
    out
}

/// Normalised `origin` URL (else the first configured remote) from a git config.
fn remote_of(gitdir: &Path) -> Option<String> {
    let config = std::fs::read_to_string(gitdir.join("config")).ok()?;
    remote_url(&config).and_then(|url| normalize_remote(&url))
}

fn remote_url(config: &str) -> Option<String> {
    let mut section = String::new();
    let mut origin: Option<String> = None;
    let mut first: Option<String> = None;
    for line in config.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
            continue;
        }
        if line.starts_with('[') {
            section = line.to_string();
            continue;
        }
        if !section.starts_with("[remote ") {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        if key.trim() != "url" {
            continue;
        }
        let value = value.trim().trim_matches('"').to_string();
        if value.is_empty() {
            continue;
        }
        if section.contains("\"origin\"") {
            origin = Some(value);
        } else if first.is_none() {
            first = Some(value);
        }
    }
    origin.or(first)
}

/// `dev:ino` of a directory. macOS/Linux only: elsewhere the identity is simply
/// absent and projects fall back to the path and the remote.
fn inode_key(dir: &Path) -> Option<String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let meta = std::fs::metadata(dir).ok()?;
        Some(format!("{}:{}", meta.dev(), meta.ino()))
    }
    #[cfg(not(unix))]
    {
        let _ = dir;
        None
    }
}

/// Does `path` fall under any ignore rule? A rule ending in `/` covers that path
/// and everything below it; a rule without one matches exactly, so `~/dev` cannot
/// swallow every repository under it. `~` expands at match time.
pub fn ignore_matches(rules: &[String], path: &str, home: &Path) -> bool {
    let path = path.trim_end_matches('/');
    if path.is_empty() {
        return false;
    }
    rules.iter().any(|rule| {
        let rule = rule.trim();
        if rule.is_empty() {
            return false;
        }
        let expanded = expand_rule(rule, home);
        let subtree = expanded.ends_with('/');
        let base = expanded.trim_end_matches('/');
        if base.is_empty() || base == "/" {
            return false;
        }
        if subtree {
            path == base || path.starts_with(&format!("{base}/"))
        } else {
            path == base
        }
    })
}

fn expand_rule(rule: &str, home: &Path) -> String {
    if rule == "~" {
        return home.display().to_string();
    }
    match rule.strip_prefix("~/") {
        Some(rest) => home.join(rest).display().to_string(),
        None => rule.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("tt-pid-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::canonicalize(&dir).unwrap()
    }

    fn fake_repo(root: &Path, remote: Option<&str>) {
        std::fs::create_dir_all(root.join(".git")).unwrap();
        let config = match remote {
            Some(url) => format!("[core]\n\trepositoryformatversion = 0\n[remote \"origin\"]\n\turl = {url}\n"),
            None => "[core]\n\trepositoryformatversion = 0\n".to_string(),
        };
        std::fs::write(root.join(".git/config"), config).unwrap();
    }

    #[test]
    fn normalizes_file_uris_escapes_and_trailing_slashes() {
        assert_eq!(
            normalize_project_path("file:///Users/x/Prompts%20for%20ST"),
            "/Users/x/Prompts for ST"
        );
        assert_eq!(
            normalize_project_path("/Users/x/Prompts for ST///"),
            "/Users/x/Prompts for ST"
        );
        // synthetic, non-path project names are never rewritten
        assert_eq!(normalize_project_path("WackChatter · Lyra"), "WackChatter · Lyra");
        assert_eq!(normalize_project_path("WackChatter · 100% Lyra"), "WackChatter · 100% Lyra");
        // a stray escape that is not a path stays literal
        assert_eq!(normalize_project_path("100%20done"), "100%20done");
    }

    #[test]
    fn normalizes_remotes_across_protocols() {
        for url in [
            "https://github.com/Intelios/X.git",
            "git@github.com:Intelios/X.git",
            "ssh://git@github.com/Intelios/X",
            "https://user:pw@github.com/Intelios/X/",
        ] {
            assert_eq!(
                normalize_remote(url).as_deref(),
                Some("github.com/intelios/x"),
                "{url}"
            );
        }
        assert_eq!(normalize_remote(""), None);
    }

    #[test]
    fn finds_the_repository_and_its_identity() {
        let root = tmp("repo");
        fake_repo(&root, Some("https://github.com/Intelios/Probe.git"));
        let deep = root.join("crates/core/src");
        std::fs::create_dir_all(&deep).unwrap();

        let repo = inspect(&deep.to_string_lossy());
        assert_eq!(repo.root.as_deref(), Some(root.to_string_lossy().as_ref()));
        assert_eq!(repo.remote.as_deref(), Some("github.com/intelios/probe"));
        #[cfg(unix)]
        assert!(repo.inode.is_some());
        assert!(!repo.worktree);

        // deleted deep folders still land on the repository above them
        assert_eq!(
            resolve_project_root(&root.join("gone/long/ago").to_string_lossy()),
            root.to_string_lossy()
        );

        // outside any repository: the folder itself
        let loose = tmp("loose");
        let scripts = loose.join("notes/scripts");
        std::fs::create_dir_all(&scripts).unwrap();
        assert_eq!(
            resolve_project_root(&scripts.to_string_lossy()),
            scripts.to_string_lossy()
        );
    }

    #[test]
    fn linked_worktrees_resolve_to_the_main_checkout() {
        let main = tmp("wt-main");
        fake_repo(&main, Some("git@github.com:Intelios/Probe.git"));
        let worktree = tmp("wt-task");
        // What `git worktree add` writes: a `.git` file plus a commondir.
        let gitdir = main.join(".git/worktrees/task");
        std::fs::create_dir_all(&gitdir).unwrap();
        std::fs::write(gitdir.join("commondir"), "../..\n").unwrap();
        std::fs::write(
            worktree.join(".git"),
            format!("gitdir: {}\n", gitdir.display()),
        )
        .unwrap();

        let repo = inspect(&worktree.to_string_lossy());
        assert!(repo.worktree);
        assert_eq!(repo.root.as_deref(), Some(main.to_string_lossy().as_ref()));
        assert_eq!(repo.remote.as_deref(), Some("github.com/intelios/probe"));
        assert_eq!(
            resolve_project_root(&worktree.join("src").to_string_lossy()),
            main.to_string_lossy()
        );
    }

    #[test]
    fn submodules_stay_their_own_project() {
        let root = tmp("sub-parent");
        fake_repo(&root, Some("https://github.com/Intelios/Parent.git"));
        let sub = root.join("vendor/inner");
        std::fs::create_dir_all(&sub).unwrap();
        // What `git submodule add` writes: a `.git` file, no commondir.
        let gitdir = root.join(".git/modules/inner");
        std::fs::create_dir_all(&gitdir).unwrap();
        std::fs::write(gitdir.join("config"), "[core]\n\tbare = false\n").unwrap();
        std::fs::write(sub.join(".git"), "gitdir: ../../.git/modules/inner\n").unwrap();

        let repo = inspect(&sub.to_string_lossy());
        assert!(!repo.worktree);
        assert_eq!(repo.root.as_deref(), Some(sub.to_string_lossy().as_ref()));
        assert_eq!(resolve_project_root(&sub.to_string_lossy()), sub.to_string_lossy());
    }

    #[test]
    fn a_worktree_without_its_main_checkout_is_its_own_project() {
        let worktree = tmp("orphan-wt");
        let gitdir = tmp("orphan-gitdir").join("worktrees/task");
        std::fs::create_dir_all(&gitdir).unwrap();
        std::fs::write(gitdir.join("commondir"), "../../missing-git\n").unwrap();
        std::fs::write(
            worktree.join(".git"),
            format!("gitdir: {}\n", gitdir.display()),
        )
        .unwrap();

        let repo = inspect(&worktree.to_string_lossy());
        assert!(!repo.worktree);
        assert_eq!(repo.root.as_deref(), Some(worktree.to_string_lossy().as_ref()));
    }

    #[test]
    fn ignore_rules_match_subtrees_and_exact_paths() {
        let home = Path::new("/Users/someone");
        let rules = vec![
            "~".to_string(),
            "~/Documents/Codex/".to_string(),
            "/private/tmp/".to_string(),
        ];
        assert!(ignore_matches(&rules, "/Users/someone", home));
        // an exact rule does not swallow children
        assert!(!ignore_matches(&rules, "/Users/someone/dev/foo", home));
        assert!(ignore_matches(&rules, "/Users/someone/Documents/Codex/2026-10-05/hi", home));
        assert!(ignore_matches(&rules, "/private/tmp", home));
        assert!(ignore_matches(&rules, "/private/tmp/wackcode-run", home));
        assert!(!ignore_matches(&rules, "/Users/someone/Documents/GitHub/TokenTrail", home));
        assert!(!ignore_matches(&rules, "/", home));
    }
}
