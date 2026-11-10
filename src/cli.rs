use clap::{Parser, Subcommand};

use crate::config::APP_NAME;

/// A tiny, lightweight CLI for quick notes, kept in a git repo.
///
/// Without a command, creates a new note named after the current directory
/// and the current time.
#[derive(Parser)]
#[command(name = APP_NAME, version)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Command>,
}

#[derive(Subcommand)]
pub enum Command {
    /// Set up charta: notes repo, autosync, notes directory
    Init,
    /// Create a new note with the given name
    New { name: String },
    /// Open an existing note
    Open { name: String },
    /// List all notes
    List,
    /// Pull remote changes and push local commits
    Sync,
}
