mod cli;
mod config;
mod editor;
mod git;
mod init;
mod notes;

use std::env;
use std::path::Path;

use anyhow::{Context, Result, anyhow, bail};
use clap::Parser;

use cli::{Cli, Command};
use config::{APP_NAME, Config};

fn main() {
    if let Err(err) = run() {
        eprintln!("{APP_NAME}: {err:#}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    match Cli::parse().command {
        None => {
            let (config, dir) = setup()?;
            let cwd = env::current_dir().context("cannot determine current directory")?;
            let name = notes::default_name(&dir, &cwd, chrono::Local::now().naive_local());
            edit(&config, &dir, &name)
        }
        Some(Command::New { name }) => {
            let name = notes::normalize(&name)?;
            let (config, dir) = setup()?;
            if notes::exists(&dir, &name) {
                bail!("note `{name}` already exists, use `{APP_NAME} open {name}`");
            }
            edit(&config, &dir, &name)
        }
        Some(Command::Open { name }) => {
            let name = notes::normalize(&name)?;
            let (config, dir) = setup()?;
            if !notes::exists(&dir, &name) {
                bail!("note `{name}` not found, use `{APP_NAME} new {name}` to create it");
            }
            edit(&config, &dir, &name)
        }
        Some(Command::Init) => {
            // A broken settings file must not block the command that fixes it.
            let existing = Config::load().unwrap_or_else(|err| {
                eprintln!("{APP_NAME}: warning: {err:#}, starting from defaults");
                None
            });
            init::run(existing).map(drop)
        }
        Some(Command::List) => {
            // No pull here: this runs on every tab completion.
            let config = Config::load()?
                .with_context(|| format!("{APP_NAME} is not set up, run `{APP_NAME} init`"))?;
            for name in notes::list(&config.notes_path()?)? {
                println!("{name}");
            }
            Ok(())
        }
    }
}

/// Loads the config (running `init` if there is none) and syncs if enabled.
fn setup() -> Result<(Config, std::path::PathBuf)> {
    let config = match Config::load()? {
        Some(config) => config,
        None => {
            eprintln!("{APP_NAME} is not set up yet, let's do that first.\n");
            init::run(None)?
        }
    };
    let dir = config.notes_path()?;
    if !git::is_repo(&dir) {
        bail!(
            "{} is not a git repository, run `{APP_NAME} init`",
            dir.display()
        );
    }
    if config.autosync {
        git::pull(&dir);
    }
    Ok((config, dir))
}

/// Opens a note in the editor, deletes it if left empty, then commits
/// (and pushes, with autosync) whatever changed.
fn edit(config: &Config, dir: &Path, name: &str) -> Result<()> {
    let path = notes::path(dir, name);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    if !path.exists() {
        std::fs::write(&path, "")?;
    }

    // Whatever the editor's exit status, the file on disk is the truth.
    let edited = editor::open(&path);

    if path.exists() && notes::is_blank(&path)? {
        notes::remove(dir, &path)?;
    }
    let rel = path.strip_prefix(dir)?;
    let committed = git::commit_note(dir, rel, name).map_err(|err| match &edited {
        Err(editor_err) => anyhow!("{editor_err:#}; also {err:#}"),
        Ok(()) => err,
    })?;
    if committed && !path.exists() {
        eprintln!("{APP_NAME}: deleted empty note `{name}`");
    }
    if config.autosync {
        git::push(dir);
    }
    edited
}
