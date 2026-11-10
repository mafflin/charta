use std::path::Path;
use std::process::{Command, Stdio};

use anyhow::{Context, Result, bail};

use crate::config::APP_NAME;

const GIT: &str = "git";
pub const GIT_DIR: &str = ".git";
pub const DEFAULT_REMOTE: &str = "origin";
const UPSTREAM_REF: &str = "@{upstream}";
const HEAD: &str = "HEAD";

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

/// Runs the command with stderr silenced and returns its stdout, minus the
/// trailing newline. Leading whitespace is kept (it's significant in
/// `git status --porcelain`).
fn output(cmd: &mut Command) -> Result<String> {
    let out = cmd
        .stderr(Stdio::null())
        .output()
        .with_context(spawn_error)?;
    if !out.status.success() {
        bail!("`{}` failed", describe(cmd));
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim_end().to_string())
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

fn warn(err: anyhow::Error, consequence: &str) {
    eprintln!("{APP_NAME}: warning: {err:#}, {consequence}");
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
    track(dir)
}

/// If there's no upstream yet and the (already fetched) remote has history,
/// tracks it: the remote branch of the same name if there is one, otherwise
/// the remote's default branch, renaming the local branch to match. Then
/// integrates it. Does nothing for an empty remote; the first push sets the
/// upstream instead.
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
    run(git(dir).args(["branch", "-q", "--set-upstream-to", &remote_branch]))?;
    pull(dir);
    Ok(())
}

/// The remote's default branch as `origin/<name>`, if the remote has one.
fn remote_default_branch(dir: &Path) -> Option<String> {
    succeeds(git(dir).args(["remote", "set-head", DEFAULT_REMOTE, "--auto"]));
    let default_ref = format!("refs/remotes/{DEFAULT_REMOTE}/{HEAD}");
    output(git(dir).args(["symbolic-ref", "--short", &default_ref])).ok()
}

/// Pulls if the current branch tracks a remote. If there's a remote but no
/// upstream yet (it was empty at `init`), checks whether it has history now.
/// Failures are only warnings: being offline must never stop you from taking
/// a note. A conflicting rebase is aborted so the repo is never left
/// mid-rebase.
pub fn pull(dir: &Path) {
    if !has_upstream(dir) {
        if remote_url(dir).is_some()
            && let Err(err) = fetch(dir).and_then(|()| track(dir))
        {
            warn(err, "continuing with local notes");
        }
        return;
    }
    if let Err(err) = run(git(dir).args(["pull", "-q", "--rebase", "--autostash"])) {
        succeeds(git(dir).args(["rebase", "--abort"]));
        let consequence = format!(
            "continuing with local notes (resolve manually in {})",
            dir.display()
        );
        warn(err, &consequence);
    }
}

/// Pushes local commits the upstream doesn't have, setting the upstream to
/// [`DEFAULT_REMOTE`] on first push. Failures are only warnings; commits stay
/// local until the next successful push.
pub fn push(dir: &Path) {
    let result = if has_upstream(dir) {
        if !is_ahead(dir) {
            return;
        }
        run(git(dir).args(["push", "-q"]))
    } else if remote_url(dir).is_some() && has_commits(dir) {
        run(git(dir).args(["push", "-q", "-u", DEFAULT_REMOTE, HEAD]))
    } else {
        return;
    };
    if let Err(err) = result {
        warn(err, "changes are committed locally");
    }
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
    run(git(dir).args(["add", "-A", "--"]).arg(rel))?;
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
    run(git(dir)
        .args(["commit", "-q", "-m", &message, "--"])
        .arg(rel))
    .context("note is saved, but committing it failed")?;
    Ok(true)
}
