use std::env;
use std::path::Path;
use std::process::Command;

use anyhow::{Context, Result, bail};

/// Checked in order; the first non-empty one wins.
const EDITOR_VARS: [&str; 2] = ["VISUAL", "EDITOR"];
const FALLBACK_EDITOR: &str = "vi";

/// Opens `path` in `$VISUAL`, `$EDITOR`, or `vi`, and waits for it to exit.
pub fn open(path: &Path) -> Result<()> {
    let raw = EDITOR_VARS
        .iter()
        .find_map(|var| env::var(var).ok().filter(|v| !v.trim().is_empty()))
        .unwrap_or_else(|| FALLBACK_EDITOR.to_string());
    let parts =
        shell_words::split(&raw).with_context(|| format!("invalid editor command `{raw}`"))?;
    let Some((program, args)) = parts.split_first() else {
        bail!("invalid editor command `{raw}`");
    };
    let status = Command::new(program)
        .args(args)
        .arg(path)
        .status()
        .with_context(|| format!("failed to launch editor `{program}`"))?;
    if !status.success() {
        bail!("editor `{program}` exited with {status}");
    }
    Ok(())
}
