//! Syncing as the commands use it: [`git`]'s pull and push, plus committing
//! notes that were left uncommitted (e.g. by a crash) so they don't block the
//! pull.

use std::path::Path;

use anyhow::Result;

use crate::config::APP_NAME;
use crate::{git, notes};

pub const COMMITTED_LOCALLY: &str = "changes are committed locally";
pub const KEEPING_LOCAL: &str = "continuing with local notes";

/// Reports a failed sync step as a warning, with what it means for the notes.
pub fn warn_on_err(result: Result<()>, consequence: &str) {
    if let Err(err) = result {
        eprintln!("{APP_NAME}: warning: {err:#}, {consequence}");
    }
}

fn try_pull(dir: &Path) -> Result<()> {
    commit_stray_notes(dir);
    git::pull(dir)
}

/// Pulls before editing, with failures only as warnings: being offline must
/// never stop you from taking a note.
pub fn pull(dir: &Path) {
    warn_on_err(try_pull(dir), KEEPING_LOCAL);
}

/// Pushes local commits, with failures only as warnings; commits stay local
/// until the next successful push.
pub fn push(dir: &Path) {
    warn_on_err(git::push(dir), COMMITTED_LOCALLY);
}

/// Pulls, then pushes, with failures only as warnings.
pub fn sync_or_warn(dir: &Path) {
    warn_on_err(
        try_pull(dir).and_then(|()| git::push(dir)),
        COMMITTED_LOCALLY,
    );
}

/// Pulls, then pushes; for `charta sync`, so failures are errors.
pub fn sync(dir: &Path) -> Result<()> {
    try_pull(dir)?;
    git::push(dir)
}

/// Commits notes that changed but weren't committed, e.g. because charta
/// was killed while editing or the commit failed. Files that aren't notes
/// are left alone, so the pull still refuses to start over them. A note
/// that can't be committed is only a warning: the pull reports it if it's
/// in the way.
fn commit_stray_notes(dir: &Path) {
    let Ok(files) = git::changed_files(dir) else {
        return;
    };
    for rel in files {
        let Some(name) = notes::name_of(&rel) else {
            continue;
        };
        let path = dir.join(&rel);
        let committed = (|| {
            if path.is_file() && notes::is_blank(&path)? {
                notes::remove(dir, &path)?;
            }
            git::commit_note(dir, &rel, &name)
        })();
        if let Err(err) = committed {
            eprintln!("{APP_NAME}: warning: {err:#}, `{name}` stays uncommitted");
        }
    }
}
