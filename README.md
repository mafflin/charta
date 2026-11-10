# charta

Quick notes from the terminal, kept in a git repo.

```sh
charta                 # new note named after the current directory
charta new <name>      # new note with a given name (may contain `/`)
charta open <name>     # open an existing note (tab-completed)
charta list            # list notes
charta init            # (re)configure
```

Notes are markdown files in `~/.local/share/charta/notes` (a git repo), opened
in `$VISUAL` / `$EDITOR` / `vi`. A note saved empty (or whitespace-only) is
deleted when the editor exits. Every change is committed.

Running `charta` in a directory that already has a note creates
`<dir>-YYYY-MM-DD-HHMM.md` instead.

## Install

With a recent Rust toolchain, from a clone:

```sh
cargo install --locked --path .                   # into ~/.cargo/bin
stow -d completions -t ~/.completions bash        # bash completion
stow -d completions -t ~/.zsh/completions zsh     # zsh completion
charta init
```

stow needs the target directories to exist. For zsh, add `fpath=(~/.zsh/completions $fpath)` before
`compinit`. The completions complete commands and, after `open`, note names.

## Settings

`~/.config/charta/settings.toml` (respects `$XDG_CONFIG_HOME`):

```toml
repo = "git@github.com:you/notes.git"  # omit for local-only notes
autosync = true                        # git pull before commands, git push after edits
notes_dir = "~/.local/share/charta/notes"
```

Sync failures (e.g. offline) are warnings; notes are always saved and committed locally.
A pull that conflicts is aborted, leaving the repo as it was, so you can resolve it by hand.

`charta init` checks that the remote is reachable before changing anything, and
fails otherwise. Re-running it offers the current settings as defaults. Answer `-`
at the remote prompt to switch back to local-only notes.
