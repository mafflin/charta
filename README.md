# charta

A tiny, lightweight CLI for quick notes, kept in a git repo.

```sh
charta                 # new note named <dir>-YYYY-MM-DD-HHMM
charta new <name>      # new note with a given name (may contain `/`)
charta open <name>     # open an existing note (tab-completed)
charta list            # list notes
charta sync            # pull remote changes, push local commits
charta init            # (re)configure
```

Notes are markdown files in `~/.local/share/charta/notes` (a git repo), opened
in `$VISUAL` / `$EDITOR` / `vi`. A note saved empty (or whitespace-only) is
deleted when the editor exits. Every change is committed.

`<dir>` is the current directory's name. A second note from the same directory
in the same minute gets a `-2` suffix, and so on.

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
autosync = "save"                      # full, save or off, see below
notes_dir = "~/.local/share/charta/notes"
```

`autosync` decides when charta syncs with the remote. Syncing means pulling
(rebasing local commits onto the remote) and pushing:

- `save` (default): syncs after a note was changed. The editor opens right away,
  but may show an outdated note if it was changed elsewhere.
- `full`: pulls before opening the editor, so you always edit the latest version,
  at the cost of waiting for the remote first; pushes after.
- `off`: never; run `charta sync` yourself.

Local-only notes (no `repo`) never sync, whatever `autosync` says.

Autosync failures (e.g. offline) are warnings; notes are always saved and committed locally.
`charta sync` reports them as errors instead. A pull never leaves the repo half-done: one
that conflicts is aborted, and none starts while the repo has uncommitted changes, so you
can resolve things by hand.

`charta init` checks that the remote is reachable before changing anything, and
fails otherwise. Re-running it offers the current settings as defaults. Answer `-`
at the remote prompt to switch back to local-only notes.
