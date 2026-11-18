use std::ffi::OsStr;
use std::io::{self, Write};
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::Duration;

use anyhow::{Context, Result, bail};

use crate::sync;

const GIT: &str = "git";
pub const GIT_DIR: &str = ".git";
pub const DEFAULT_REMOTE: &str = "origin";
const UPSTREAM_REF: &str = "@{upstream}";
const HEAD: &str = "HEAD";
/// What git says when another git process holds the index.
const INDEX_LOCKED: &str = "index.lock";
const LOCK_RETRIES: u32 = 20;
const LOCK_RETRY_DELAY: Duration = Duration::from_millis(100);

/// What happened to a note, as recorded in its commit message.
enum NoteChange {
    Create,
    Update,
    Delete,
}

impl NoteChange {
    /// Maps the two-letter code from `git status --porcelain`.
    fn from_porcelain(code: &str) -> Self {
        match code {
            "??" | "A " => Self::Create,
            c if c.contains('D') => Self::Delete,
            _ => Self::Update,
        }
    }

    fn as_str(&self) -> &'static str {
        match self {
            Self::Create => "create",
            Self::Update => "update",
            Self::Delete => "delete",
        }
    }
}

fn git(dir: &Path) -> Command {
    let mut cmd = Command::new(GIT);
    cmd.arg("-C").arg(dir);
    cmd
}

fn spawn_error() -> String {
    format!("failed to run {GIT} (is it installed?)")
}

fn run(cmd: &mut Command) -> Result<()> {
    let status = cmd.status().with_context(spawn_error)?;
    if !status.success() {
        bail!("`{}` failed", describe(cmd));
    }
    Ok(())
}

/// Like [`run`], but retries for a while if another git process (e.g. one
/// the user runs by hand) holds the index lock.
fn run_retrying(cmd: &mut Command) -> Result<()> {
    let mut attempt = 1;
    loop {
        let out = cmd.output().with_context(spawn_error)?;
        if out.status.success() {
            return Ok(());
        }
        let stderr = String::from_utf8_lossy(&out.stderr);
        if attempt < LOCK_RETRIES && stderr.contains(INDEX_LOCKED) {
            attempt += 1;
            thread::sleep(LOCK_RETRY_DELAY);
            continue;
        }
        // Hooks may report on either.
        let _ = io::stdout().write_all(&out.stdout);
        let _ = io::stderr().write_all(&out.stderr);
        bail!("`{}` failed", describe(cmd));
    }
}

/// Runs the command with stderr silenced and returns its stdout, minus the
/// trailing newline. Leading whitespace is kept (it's significant in
/// `git status --porcelain`).
fn output(cmd: &mut Command) -> Result<String> {
    let stdout = output_bytes(cmd)?;
    Ok(String::from_utf8_lossy(&stdout).trim_end().to_string())
}

/// Like [`output`], but returns stdout as it is, for paths that may not be
/// UTF-8.
fn output_bytes(cmd: &mut Command) -> Result<Vec<u8>> {
    let out = cmd
        .stderr(Stdio::null())
        .output()
        .with_context(spawn_error)?;
    if !out.status.success() {
        bail!("`{}` failed", describe(cmd));
    }
    Ok(out.stdout)
}

fn succeeds(cmd: &mut Command) -> bool {
    cmd.stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

fn describe(cmd: &Command) -> String {
    let args: Vec<_> = cmd.get_args().map(|a| a.to_string_lossy()).collect();
    format!("{GIT} {}", args.join(" "))
}

pub fn is_repo(dir: &Path) -> bool {
    dir.join(GIT_DIR).exists()
}

/// Checks that `url` can be reached, without touching any repo.
pub fn check_remote(url: &str) -> Result<()> {
    run(Command::new(GIT)
        .args(["ls-remote", "-q", url])
        .stdout(Stdio::null()))
    .with_context(|| format!("cannot reach {url}"))
}

pub fn clone(url: &str, dir: &Path) -> Result<()> {
    run(Command::new(GIT).args(["clone", "-q", url]).arg(dir))
}

pub fn init(dir: &Path) -> Result<()> {
    run(Command::new(GIT).args(["init", "-q"]).arg(dir))
}

fn remote_url(dir: &Path) -> Option<String> {
    output(git(dir).args(["remote", "get-url", DEFAULT_REMOTE])).ok()
}

pub fn has_remote(dir: &Path) -> bool {
    remote_url(dir).is_some()
}

fn has_commits(dir: &Path) -> bool {
    ref_exists(dir, HEAD)
}

fn has_upstream(dir: &Path) -> bool {
    ref_exists(dir, UPSTREAM_REF)
}

fn ref_exists(dir: &Path, reference: &str) -> bool {
    succeeds(git(dir).args(["rev-parse", "--verify", "-q", reference]))
}

/// Whether there are local commits the upstream doesn't have yet.
fn is_ahead(dir: &Path) -> bool {
    let range = format!("{UPSTREAM_REF}..{HEAD}");
    output(git(dir).args(["rev-list", "--count", &range])).is_ok_and(|count| count != "0")
}

fn fetch(dir: &Path) -> Result<()> {
    run(git(dir).args(["fetch", "-q", "--prune", DEFAULT_REMOTE]))
}

/// Points [`DEFAULT_REMOTE`] at `url` in an existing repo, fetches it, and
/// starts tracking it if it already has history. Call [`check_remote`] first.
pub fn connect_remote(dir: &Path, url: &str) -> Result<()> {
    match remote_url(dir) {
        Some(current) if current == url => {}
        Some(_) => run(git(dir).args(["remote", "set-url", DEFAULT_REMOTE, url]))?,
        None => run(git(dir).args(["remote", "add", DEFAULT_REMOTE, url]))?,
    }
    // Pruning drops refs of a previous remote, so they don't count as upstream.
    fetch(dir)?;
    track(dir)?;
    sync::warn_on_err(integrate(dir), sync::KEEPING_LOCAL);
    Ok(())
}

/// If there's no upstream yet and the (already fetched) remote has history,
/// tracks it: the remote branch of the same name if there is one, otherwise
/// the remote's default branch, renaming the local branch to match. Does
/// nothing for an empty remote; the first push sets the upstream instead.
fn track(dir: &Path) -> Result<()> {
    if has_upstream(dir) {
        return Ok(());
    }
    if !has_commits(dir) {
        // Fresh repo without notes: just check out the remote's default branch.
        if let Some(remote_branch) = remote_default_branch(dir) {
            run(git(dir).args(["checkout", "-q", "--track", &remote_branch]))?;
        }
        return Ok(());
    }
    let local = output(git(dir).args(["symbolic-ref", "--short", HEAD]))?;
    let same_name = format!("{DEFAULT_REMOTE}/{local}");
    let remote_branch = if ref_exists(dir, &same_name) {
        same_name
    } else {
        let Some(remote_branch) = remote_default_branch(dir) else {
            return Ok(());
        };
        let name = remote_branch
            .strip_prefix(&format!("{DEFAULT_REMOTE}/"))
            .unwrap_or(&remote_branch);
        run(git(dir).args(["branch", "-m", name]))?;
        remote_branch
    };
    run(git(dir).args(["branch", "-q", "--set-upstream-to", &remote_branch]))
}

/// The remote's default branch as `origin/<name>`, if the remote has one.
fn remote_default_branch(dir: &Path) -> Option<String> {
    succeeds(git(dir).args(["remote", "set-head", DEFAULT_REMOTE, "--auto"]));
    let default_ref = format!("refs/remotes/{DEFAULT_REMOTE}/{HEAD}");
    output(git(dir).args(["symbolic-ref", "--short", &default_ref])).ok()
}

/// Fetches the remote, starts tracking it if it has history now (it may have
/// been empty at `init`), and rebases local commits onto it.
pub fn pull(dir: &Path) -> Result<()> {
    if !has_remote(dir) {
        return Ok(());
    }
    fetch(dir)?;
    track(dir)?;
    integrate(dir)
}

/// Brings the already fetched upstream into the current branch, without
/// touching the network. Refuses to start with uncommitted changes, and
/// aborts a conflicting rebase: either way the repo is left as it was.
fn integrate(dir: &Path) -> Result<()> {
    if !has_upstream(dir) {
        return Ok(());
    }
    if !has_commits(dir) {
        // Nothing to rebase yet (and `rebase` can't start from no commits).
        return run(git(dir).args(["merge", "-q", "--ff-only", UPSTREAM_REF]));
    }
    if has_uncommitted_changes(dir)? {
        bail!(
            "uncommitted changes in {}, commit or discard them first",
            dir.display()
        );
    }
    if let Err(err) = run(git(dir).args(["rebase", "-q", UPSTREAM_REF])) {
        // Only succeeds if the rebase got as far as a conflict.
        if succeeds(git(dir).args(["rebase", "--abort"])) {
            bail!(
                "{err:#}: local and remote changes conflict (resolve manually in {})",
                dir.display()
            );
        }
        return Err(err);
    }
    Ok(())
}

/// Whether tracked files differ from the last commit. Untracked files don't
/// count: a rebase leaves them alone.
fn has_uncommitted_changes(dir: &Path) -> Result<bool> {
    let status = output(git(dir).args(["status", "--porcelain", "--untracked-files=no"]))?;
    Ok(!status.is_empty())
}

/// Pushes local commits the upstream doesn't have, setting the upstream to
/// [`DEFAULT_REMOTE`] on first push.
pub fn push(dir: &Path) -> Result<()> {
    if has_upstream(dir) {
        if is_ahead(dir) {
            run(git(dir).args(["push", "-q"]))?;
        }
    } else if has_remote(dir) && has_commits(dir) {
        run(git(dir).args(["push", "-q", "-u", DEFAULT_REMOTE, HEAD]))?;
    }
    Ok(())
}

/// Files that differ from the last commit, untracked ones included,
/// relative to `dir`. Renames and copies the user staged are left out.
pub fn changed_files(dir: &Path) -> Result<Vec<PathBuf>> {
    // `-z`: unquoted paths, NUL-terminated, kept as bytes (they may not be UTF-8).
    let status =
        output_bytes(git(dir).args(["status", "--porcelain", "-z", "--untracked-files=all"]))?;
    let mut files = Vec::new();
    let mut entries = status.split(|&b| b == 0).filter(|e| !e.is_empty());
    while let Some(entry) = entries.next() {
        let (Some(code), Some(path)) = (entry.get(..2), entry.get(3..)) else {
            continue;
        };
        if code.iter().any(|c| matches!(c, b'R' | b'C')) {
            // The next entry is the source path.
            entries.next();
            continue;
        }
        files.push(PathBuf::from(OsStr::from_bytes(path)));
    }
    Ok(files)
}

/// Commits the current state of a single note, if it changed.
/// Returns whether a commit was made.
pub fn commit_note(dir: &Path, rel: &Path, name: &str) -> Result<bool> {
    let status = output(
        git(dir)
            .args(["status", "--porcelain", "--untracked-files=all", "--"])
            .arg(rel),
    )?;
    let Some(code) = status.lines().next().and_then(|l| l.get(..2)) else {
        return Ok(false);
    };
    let change = NoteChange::from_porcelain(code);
    run_retrying(git(dir).args(["add", "-A", "--"]).arg(rel))?;
    // Staging can leave nothing to commit, e.g. a note that was added to the
    // index but never committed, then emptied.
    if succeeds(
        git(dir)
            .args(["diff", "--cached", "--quiet", "--"])
            .arg(rel),
    ) {
        return Ok(false);
    }
    let message = format!("{name}: {}", change.as_str());
    run_retrying(
        git(dir)
            .args(["commit", "-q", "-m", &message, "--"])
            .arg(rel),
    )
    .context("note is saved, but committing it failed")?;
    Ok(true)
}
