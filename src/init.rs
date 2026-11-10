use std::env;
use std::fs;
use std::io::{self, BufRead, Write};

use anyhow::{Context, Result, bail};

use crate::config::{self, APP_NAME, Config};
use crate::git;

/// Answer to the remote prompt that switches back to local-only notes.
const LOCAL_ONLY_ANSWER: &str = "-";

/// Interactive setup. Existing settings are offered as defaults.
pub fn run(existing: Option<Config>) -> Result<Config> {
    let current = existing.unwrap_or_default();

    let notes_dir = absolutize(&prompt("Notes directory", &current.notes_dir)?)?;
    let repo = match &current.repo {
        Some(url) => prompt(
            &format!("Git remote URL (`{LOCAL_ONLY_ANSWER}` for local-only)"),
            url,
        )?,
        None => prompt("Git remote URL (leave empty for local-only)", "")?,
    };
    let repo = Some(repo).filter(|r| !r.is_empty() && r != LOCAL_ONLY_ANSWER);
    let autosync = match repo {
        Some(_) => confirm(
            "Autosync (pull before commands, push after edits)?",
            current.repo.is_none() || current.autosync,
        )?,
        None => false,
    };

    // Fail before changing anything: a half-connected repo is worse than none.
    if let Some(url) = &repo {
        git::check_remote(url)
            .context("check the URL and your connection, or answer `-` (local-only) for now")?;
    }

    let config = Config {
        repo,
        autosync,
        notes_dir,
    };
    prepare_repo(&config)?;
    let path = config.save()?;
    eprintln!("{APP_NAME}: settings saved to {}", path.display());
    Ok(config)
}

/// Makes a relative notes dir absolute, so it doesn't depend on where
/// charta is run from. `~` paths are kept as they are.
fn absolutize(dir: &str) -> Result<String> {
    if !config::expand_tilde(dir)?.is_relative() {
        return Ok(dir.to_string());
    }
    let cwd = env::current_dir().context("cannot determine current directory")?;
    Ok(cwd.join(dir).display().to_string())
}

/// Clones, creates, or reuses the notes repo. An existing repo is connected
/// to the remote; further syncing is left to the next command.
fn prepare_repo(config: &Config) -> Result<()> {
    let dir = config.notes_path()?;
    if git::is_repo(&dir) {
        if let Some(url) = &config.repo {
            git::connect_remote(&dir, url)?;
        }
        return Ok(());
    }
    if dir.exists() && fs::read_dir(&dir)?.next().is_some() {
        bail!("{} exists but is not a git repository", dir.display());
    }
    if let Some(parent) = dir.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    match &config.repo {
        Some(url) => {
            eprintln!("{APP_NAME}: cloning {url} into {}", dir.display());
            git::clone(url, &dir)
        }
        None => git::init(&dir),
    }
}

fn read_answer(question: &str) -> Result<String> {
    eprint!("{question} ");
    io::stderr().flush()?;
    let mut line = String::new();
    io::stdin().lock().read_line(&mut line)?;
    Ok(line.trim().to_string())
}

fn prompt(question: &str, default: &str) -> Result<String> {
    let label = if default.is_empty() {
        format!("{question}:")
    } else {
        format!("{question} [{default}]:")
    };
    let answer = read_answer(&label)?;
    Ok(if answer.is_empty() {
        default.to_string()
    } else {
        answer
    })
}

fn confirm(question: &str, default: bool) -> Result<bool> {
    let hint = if default { "[Y/n]" } else { "[y/N]" };
    loop {
        match read_answer(&format!("{question} {hint}"))?
            .to_lowercase()
            .as_str()
        {
            "" => return Ok(default),
            "y" | "yes" => return Ok(true),
            "n" | "no" => return Ok(false),
            _ => eprintln!("Please answer y or n."),
        }
    }
}
