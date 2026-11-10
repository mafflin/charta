use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command as StdCommand;

use assert_cmd::Command;
use predicates::prelude::*;
use tempfile::TempDir;

/// An isolated HOME/XDG environment with a local-only notes repo.
struct Env {
    root: TempDir,
}

impl Env {
    fn new() -> Self {
        let env = Self::bare();
        env.write_config("autosync = \"off\"\n");
        git(&["init", "-q"], &env.notes());
        env
    }

    /// No config, no notes dir.
    fn bare() -> Self {
        let root = tempfile::tempdir().unwrap();
        fs::create_dir_all(root.path().join("home")).unwrap();
        fs::create_dir_all(root.path().join("work/myproject")).unwrap();
        Self { root }
    }

    fn path(&self, rel: &str) -> PathBuf {
        self.root.path().join(rel)
    }

    fn notes(&self) -> PathBuf {
        self.path("data/charta/notes")
    }

    fn write_config(&self, extra: &str) {
        let dir = self.path("config/charta");
        fs::create_dir_all(&dir).unwrap();
        let text = format!(
            "notes_dir = {:?}\n{extra}",
            self.notes().display().to_string()
        );
        fs::write(dir.join("settings.toml"), text).unwrap();
    }

    /// `charta` with an editor that writes `content` (or leaves the file alone if `None`).
    fn cmd(&self, content: Option<&str>) -> Command {
        let editor = match content {
            Some(text) => format!("sh -c 'printf %s \"{text}\" > \"$1\"' editor"),
            None => "true".to_string(),
        };
        let mut cmd = self.sandboxed(&charta_bin());
        cmd.env("EDITOR", editor);
        Command::from_std(cmd)
    }

    /// `program` running in this environment, with `charta` on `PATH`.
    fn sandboxed(&self, program: &Path) -> StdCommand {
        let bin_dir = charta_bin().parent().unwrap().to_path_buf();
        let path = std::env::join_paths(
            std::iter::once(bin_dir)
                .chain(std::env::split_paths(&std::env::var_os("PATH").unwrap())),
        )
        .unwrap();
        let mut cmd = StdCommand::new(program);
        cmd.current_dir(self.path("work/myproject"))
            .env_clear()
            .env("PATH", path)
            .env("HOME", self.path("home"))
            .env("XDG_CONFIG_HOME", self.path("config"))
            .env("XDG_DATA_HOME", self.path("data"))
            .envs(git_env());
        cmd
    }

    /// Name (without extension) and content of the only note in the notes dir.
    fn only_note(&self) -> (String, String) {
        let mut notes: Vec<_> = fs::read_dir(self.notes())
            .unwrap()
            .map(|e| e.unwrap().path())
            .filter(|p| p.extension().is_some_and(|e| e == "md"))
            .collect();
        assert_eq!(notes.len(), 1, "{notes:?}");
        let path = notes.pop().unwrap();
        let name = path.file_stem().unwrap().to_string_lossy().into_owned();
        (name, fs::read_to_string(path).unwrap())
    }

    fn log(&self) -> String {
        git_out(&["log", "--format=%s"], &self.notes())
    }
}

/// Asserts `name` is `myproject-YYYY-MM-DD-HHMM`, plus `-N` if `counter`.
fn assert_stamped(name: &str, counter: bool) {
    let parts: Vec<_> = name.split('-').collect();
    let lens: &[usize] = if counter {
        &[9, 4, 2, 2, 4, 1]
    } else {
        &[9, 4, 2, 2, 4]
    };
    let ok = parts[0] == "myproject"
        && parts.iter().map(|p| p.len()).eq(lens.iter().copied())
        && parts[1..]
            .iter()
            .all(|p| p.bytes().all(|b| b.is_ascii_digit()));
    assert!(ok, "unexpected default note name `{name}`");
}

fn charta_bin() -> PathBuf {
    assert_cmd::cargo::cargo_bin("charta")
}

fn git_env() -> Vec<(&'static str, &'static str)> {
    vec![
        ("GIT_CONFIG_GLOBAL", "/dev/null"),
        ("GIT_CONFIG_NOSYSTEM", "1"),
        ("GIT_AUTHOR_NAME", "Test"),
        ("GIT_AUTHOR_EMAIL", "test@example.com"),
        ("GIT_COMMITTER_NAME", "Test"),
        ("GIT_COMMITTER_EMAIL", "test@example.com"),
    ]
}

fn git(args: &[&str], dir: &Path) {
    fs::create_dir_all(dir).unwrap();
    let ok = StdCommand::new("git")
        .args(args)
        .current_dir(dir)
        .envs(git_env())
        .status()
        .unwrap();
    assert!(ok.success(), "git {args:?} failed");
}

fn git_out(args: &[&str], dir: &Path) -> String {
    let out = StdCommand::new("git")
        .args(args)
        .current_dir(dir)
        .envs(git_env())
        .output()
        .unwrap();
    String::from_utf8(out.stdout).unwrap()
}

#[test]
fn no_args_creates_note_named_after_cwd_and_time_and_commits() {
    let env = Env::new();
    env.cmd(Some("hello")).assert().success();
    let (name, content) = env.only_note();
    assert_stamped(&name, false);
    assert_eq!(content, "hello");
    assert_eq!(env.log(), format!("{name}: create\n"));
}

#[test]
fn second_note_from_same_dir_gets_distinct_name() {
    let env = Env::new();
    env.cmd(Some("one")).assert().success();
    env.cmd(Some("two")).assert().success();
    let names = String::from_utf8(env.cmd(None).arg("list").output().unwrap().stdout).unwrap();
    let names: Vec<_> = names.lines().collect();
    assert_eq!(names.len(), 2);
    assert_stamped(names[0], false);
    // Same minute: the second note gets a counter. Otherwise: a new timestamp.
    assert_stamped(names[1], names[1].starts_with(&format!("{}-", names[0])));
    assert_ne!(names[0], names[1]);
}

#[test]
fn empty_new_note_is_discarded() {
    let env = Env::new();
    env.cmd(None).args(["new", "scratch"]).assert().success();
    assert!(!env.notes().join("scratch.md").exists());
    assert_eq!(env.log(), "");
}

#[test]
fn whitespace_only_note_is_discarded() {
    let env = Env::new();
    env.cmd(Some("  \n\t"))
        .args(["new", "scratch"])
        .assert()
        .success();
    assert!(!env.notes().join("scratch.md").exists());
}

#[test]
fn new_rejects_existing_note() {
    let env = Env::new();
    env.cmd(Some("x")).args(["new", "todo"]).assert().success();
    env.cmd(Some("y"))
        .args(["new", "todo"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("already exists"));
    assert_eq!(
        fs::read_to_string(env.notes().join("todo.md")).unwrap(),
        "x"
    );
}

#[test]
fn open_edits_and_commits_update() {
    let env = Env::new();
    env.cmd(Some("v1")).args(["new", "todo"]).assert().success();
    env.cmd(Some("v2"))
        .args(["open", "todo.md"])
        .assert()
        .success();
    assert_eq!(
        fs::read_to_string(env.notes().join("todo.md")).unwrap(),
        "v2"
    );
    assert_eq!(env.log(), "todo: update\ntodo: create\n");
}

#[test]
fn open_unchanged_note_makes_no_commit() {
    let env = Env::new();
    env.cmd(Some("v1")).args(["new", "todo"]).assert().success();
    env.cmd(Some("v1"))
        .args(["open", "todo"])
        .assert()
        .success();
    assert_eq!(env.log(), "todo: create\n");
}

#[test]
fn emptying_existing_note_deletes_it() {
    let env = Env::new();
    env.cmd(Some("v1"))
        .args(["new", "work/todo"])
        .assert()
        .success();
    env.cmd(Some(""))
        .args(["open", "work/todo"])
        .assert()
        .success();
    assert!(!env.notes().join("work").exists());
    assert_eq!(env.log(), "work/todo: delete\nwork/todo: create\n");
}

#[test]
fn open_missing_note_fails() {
    let env = Env::new();
    env.cmd(None)
        .args(["open", "nope"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("not found"));
}

#[test]
fn rejects_names_escaping_notes_dir() {
    let env = Env::new();
    env.cmd(Some("x"))
        .args(["new", "../evil"])
        .assert()
        .failure();
    assert!(!env.path("data/charta/evil.md").exists());
}

#[test]
fn init_local_only_via_prompts() {
    let env = Env::bare();
    let notes = env.notes().display().to_string();
    env.cmd(None)
        .arg("init")
        .write_stdin(format!("{notes}\n\n"))
        .assert()
        .success();
    assert!(env.notes().join(".git").exists());
    let config = fs::read_to_string(env.path("config/charta/settings.toml")).unwrap();
    assert!(config.contains("autosync = \"off\""), "{config}");
    assert!(!config.contains("repo"), "{config}");
}

#[test]
fn no_args_without_config_runs_init_first() {
    let env = Env::bare();
    let notes = env.notes().display().to_string();
    env.cmd(Some("hi"))
        .write_stdin(format!("{notes}\n\n"))
        .assert()
        .success();
    let (name, content) = env.only_note();
    assert_stamped(&name, false);
    assert_eq!(content, "hi");
}

#[test]
fn autosync_clones_pushes_and_pulls() {
    let env = Env::bare();
    let remote = env.path("remote.git");
    git(
        &["init", "-q", "--bare", remote.to_str().unwrap()],
        env.root.path(),
    );
    let notes = env.notes().display().to_string();
    let answers = format!("{notes}\n{}\nfull\n", remote.display());
    env.cmd(None)
        .arg("init")
        .write_stdin(answers)
        .assert()
        .success();

    // First note is pushed and sets the upstream.
    env.cmd(Some("from charta"))
        .args(["new", "a"])
        .assert()
        .success();
    assert_eq!(
        git_out(&["log", "--format=%s", "HEAD"], &remote),
        "a: create\n"
    );

    // A change pushed from elsewhere is pulled before the next command.
    let other = env.path("other");
    git(
        &[
            "clone",
            "-q",
            remote.to_str().unwrap(),
            other.to_str().unwrap(),
        ],
        env.root.path(),
    );
    fs::write(other.join("b.md"), "from elsewhere").unwrap();
    git(&["add", "b.md"], &other);
    git(&["commit", "-q", "-m", "b"], &other);
    git(&["push", "-q"], &other);

    env.cmd(Some("edited"))
        .args(["open", "b"])
        .assert()
        .success();
    assert_eq!(
        git_out(&["log", "--format=%s", "HEAD"], &remote),
        "b: update\nb\na: create\n"
    );
}

#[test]
fn autosync_offline_still_saves_locally() {
    let env = Env::new();
    env.write_config("repo = \"/nonexistent/remote.git\"\nautosync = \"full\"\n");
    git(
        &["remote", "add", "origin", "/nonexistent/remote.git"],
        &env.notes(),
    );
    env.cmd(Some("offline"))
        .args(["new", "x"])
        .assert()
        .success()
        .stderr(predicate::str::contains("warning"));
    assert_eq!(env.log(), "x: create\n");
}

#[test]
fn bash_completion_completes_commands_and_note_names() {
    let env = Env::new();
    env.cmd(Some("x")).args(["new", "todo"]).assert().success();
    env.cmd(Some("x"))
        .args(["new", "work/standup"])
        .assert()
        .success();
    let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("completions/bash/_charta");
    let complete = |words: &str| {
        let line = format!(
            ". {:?}; COMP_WORDS=({words}); COMP_CWORD=$((${{#COMP_WORDS[@]}} - 1)); \
             _charta; printf '%s\\n' \"${{COMPREPLY[@]}}\"",
            script.display().to_string()
        );
        let out = env
            .sandboxed(Path::new("bash"))
            .args(["-c", &line])
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8(out.stdout).unwrap()
    };
    assert_eq!(complete("charta ''"), "init\nnew\nopen\nlist\nsync\nhelp\n");
    assert_eq!(complete("charta o"), "open\n");
    assert_eq!(complete("charta open ''"), "todo\nwork/standup\n");
    assert_eq!(complete("charta open w"), "work/standup\n");
    assert_eq!(complete("charta new ''"), "\n");
}

/// Loads the zsh completion from `$1` with stubs of `_describe` and `compadd`
/// that print the candidates, then completes the last of the remaining words.
const ZSH_COMPLETE: &str = r#"
fpath=("$1" $fpath)
shift
autoload -Uz _charta
_describe() {
  while [[ $1 == -? ]]; do shift 2; done
  shift
  local name
  for name; do print -rl -- "${(@)${(@P)name}%%:*}"; done
}
compadd() {
  [[ $1 == -a ]] && print -rl -- "${(@P)2}"
}
words=("$@")
CURRENT=$#
_charta
exit 0
"#;

#[test]
fn zsh_completion_completes_commands_and_note_names() {
    if StdCommand::new("zsh").arg("--version").output().is_err() {
        eprintln!("skipped: zsh is not installed");
        return;
    }
    let env = Env::new();
    env.cmd(Some("x")).args(["new", "todo"]).assert().success();
    env.cmd(Some("x"))
        .args(["new", "work/standup"])
        .assert()
        .success();
    let fpath = Path::new(env!("CARGO_MANIFEST_DIR")).join("completions/zsh");
    let complete = |words: &[&str]| {
        let out = env
            .sandboxed(Path::new("zsh"))
            .args(["-f", "-c", ZSH_COMPLETE, "zsh"])
            .arg(&fpath)
            .args(words)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8(out.stdout).unwrap()
    };
    assert_eq!(
        complete(&["charta", ""]),
        "init\nnew\nopen\nlist\nsync\nhelp\n"
    );
    assert_eq!(complete(&["charta", "open", ""]), "todo\nwork/standup\n");
    assert_eq!(complete(&["charta", "new", ""]), "");
}

fn bare_remote(env: &Env) -> PathBuf {
    let remote = env.path("remote.git");
    git(
        &["init", "-q", "--bare", remote.to_str().unwrap()],
        env.root.path(),
    );
    remote
}

/// Commits a note to `remote` from a separate clone, like another machine would.
fn push_from_elsewhere(env: &Env, remote: &Path, file: &str, content: &str) {
    let other = env.path("other");
    if !other.exists() {
        let (remote, other) = (remote.to_str().unwrap(), other.to_str().unwrap());
        git(&["clone", "-q", remote, other], env.root.path());
    }
    fs::write(other.join(file), content).unwrap();
    git(&["add", file], &other);
    git(&["commit", "-q", "-m", file], &other);
    git(&["push", "-q", "origin", "HEAD"], &other);
}

/// Runs `charta init` keeping the notes dir and setting `remote` with the
/// default autosync (`save`).
fn init_with_remote(env: &Env, remote: &Path) {
    env.cmd(None)
        .arg("init")
        .write_stdin(format!("\n{}\n\n", remote.display()))
        .assert()
        .success();
}

#[test]
fn pull_conflict_aborts_rebase_and_keeps_local_note() {
    let env = Env::bare();
    let remote = bare_remote(&env);
    env.cmd(None)
        .arg("init")
        .write_stdin(format!(
            "{}\n{}\n\n",
            env.notes().display(),
            remote.display()
        ))
        .assert()
        .success();
    env.cmd(Some("base")).args(["new", "a"]).assert().success();
    push_from_elsewhere(&env, &remote, "a.md", "theirs");
    // An unpushed local edit that conflicts with the remote one.
    fs::write(env.notes().join("a.md"), "mine").unwrap();
    git(&["commit", "-q", "-am", "local"], &env.notes());

    env.cmd(Some("mine, edited"))
        .args(["open", "a"])
        .assert()
        .success()
        .stderr(predicate::str::contains("resolve manually"));

    let git_dir = env.notes().join(".git");
    assert!(!git_dir.join("rebase-merge").exists() && !git_dir.join("rebase-apply").exists());
    assert_eq!(
        fs::read_to_string(env.notes().join("a.md")).unwrap(),
        "mine, edited"
    );
    assert!(env.log().starts_with("a: update\nlocal\n"), "{}", env.log());
}

#[test]
fn init_dash_switches_back_to_local_only() {
    let env = Env::new();
    env.write_config("repo = \"git@example.com:me/notes.git\"\nautosync = \"full\"\n");
    env.cmd(None)
        .arg("init")
        .write_stdin("\n-\n")
        .assert()
        .success();
    let config = fs::read_to_string(env.path("config/charta/settings.toml")).unwrap();
    assert!(!config.contains("repo"), "{config}");
    assert!(config.contains("autosync = \"off\""), "{config}");
}

#[test]
fn init_updates_changed_remote_url() {
    let env = Env::new();
    git(
        &["remote", "add", "origin", "/old/remote.git"],
        &env.notes(),
    );
    let remote = bare_remote(&env);
    init_with_remote(&env, &remote);
    assert_eq!(
        git_out(&["remote", "get-url", "origin"], &env.notes()).trim(),
        remote.to_str().unwrap()
    );
}

#[test]
fn init_connects_existing_notes_to_remote_with_history() {
    let env = Env::new();
    env.cmd(Some("local"))
        .args(["new", "mine"])
        .assert()
        .success();
    let remote = bare_remote(&env);
    push_from_elsewhere(&env, &remote, "theirs.md", "remote");

    init_with_remote(&env, &remote);
    assert_eq!(
        fs::read_to_string(env.notes().join("theirs.md")).unwrap(),
        "remote"
    );

    // Local history is rebased onto the remote and pushed with the next edit.
    env.cmd(Some("local, edited"))
        .args(["open", "mine"])
        .assert()
        .success()
        .stderr(predicate::str::is_empty());
    assert_eq!(
        git_out(&["log", "--format=%s", "HEAD"], &remote),
        "mine: update\nmine: create\ntheirs.md\n"
    );
}

#[test]
fn init_connects_empty_local_repo_to_remote_with_history() {
    let env = Env::new();
    let remote = bare_remote(&env);
    push_from_elsewhere(&env, &remote, "theirs.md", "remote");

    init_with_remote(&env, &remote);
    env.cmd(None)
        .arg("list")
        .assert()
        .success()
        .stdout("theirs\n");
}

#[test]
fn init_makes_relative_notes_dir_absolute() {
    let env = Env::bare();
    env.cmd(None)
        .arg("init")
        .write_stdin("rel-notes\n\n")
        .assert()
        .success();
    let expected = env.path("work/myproject/rel-notes");
    assert!(expected.join(".git").exists());
    let config = fs::read_to_string(env.path("config/charta/settings.toml")).unwrap();
    assert!(
        config.contains(&format!("{:?}", expected.display().to_string())),
        "{config}"
    );
}

#[test]
fn autosync_skips_push_when_nothing_to_push() {
    let env = Env::bare();
    let remote = bare_remote(&env);
    env.cmd(None)
        .arg("init")
        .write_stdin(format!(
            "{}\n{}\n\n",
            env.notes().display(),
            remote.display()
        ))
        .assert()
        .success();
    env.cmd(Some("v1")).args(["new", "a"]).assert().success();
    // Remote becomes unreachable; opening without changes must not try to push.
    fs::rename(&remote, env.path("gone.git")).unwrap();
    env.cmd(Some("v1"))
        .args(["open", "a"])
        .assert()
        .success()
        .stderr(predicate::str::contains("committed locally").not());
}

fn settings(env: &Env) -> String {
    fs::read_to_string(env.path("config/charta/settings.toml")).unwrap()
}

#[test]
fn init_recovers_from_broken_settings() {
    let env = Env::new();
    env.write_config("autosync = yes\n");
    env.cmd(None)
        .arg("init")
        .write_stdin("\n\n")
        .assert()
        .success()
        .stderr(predicate::str::contains("starting from defaults"));
    assert!(
        settings(&env).contains("autosync = \"off\""),
        "{}",
        settings(&env)
    );
}

#[test]
fn init_with_unreachable_remote_changes_nothing() {
    let env = Env::new();
    let before = settings(&env);
    env.cmd(None)
        .arg("init")
        .write_stdin("\n/nonexistent/remote.git\n\n")
        .assert()
        .failure()
        .stderr(predicate::str::contains("cannot reach"));
    assert_eq!(settings(&env), before);
    assert_eq!(git_out(&["remote"], &env.notes()), "");
}

#[test]
fn first_run_with_unreachable_remote_creates_nothing() {
    let env = Env::bare();
    env.cmd(Some("x"))
        .write_stdin(format!(
            "{}\n/nonexistent/remote.git\n\n",
            env.notes().display()
        ))
        .assert()
        .failure();
    assert!(!env.path("config/charta/settings.toml").exists());
    assert!(!env.notes().exists());
}

#[test]
fn init_renames_local_branch_to_remote_default() {
    let env = Env::new(); // local repo on `master`
    env.cmd(Some("local"))
        .args(["new", "mine"])
        .assert()
        .success();
    let remote = env.path("remote.git");
    git(
        &[
            "init",
            "-q",
            "--bare",
            "-b",
            "main",
            remote.to_str().unwrap(),
        ],
        env.root.path(),
    );
    let other = env.path("other");
    git(&["init", "-q", "-b", "main"], &other);
    fs::write(other.join("theirs.md"), "remote").unwrap();
    git(&["add", "theirs.md"], &other);
    git(&["commit", "-q", "-m", "theirs"], &other);
    git(&["push", "-q", remote.to_str().unwrap(), "main"], &other);

    init_with_remote(&env, &remote);
    assert_eq!(
        git_out(&["branch", "--show-current"], &env.notes()).trim(),
        "main"
    );
    assert!(env.notes().join("theirs.md").exists());

    env.cmd(Some("edited"))
        .args(["open", "mine"])
        .assert()
        .success();
    assert_eq!(
        git_out(&["branch", "--format=%(refname:short)"], &remote),
        "main\n"
    );
    assert!(git_out(&["log", "--format=%s", "main"], &remote).starts_with("mine: update\n"));
}

#[test]
fn starts_tracking_when_empty_remote_gets_history_from_elsewhere() {
    let env = Env::new();
    env.cmd(Some("local"))
        .args(["new", "mine"])
        .assert()
        .success();
    let remote = bare_remote(&env);
    init_with_remote(&env, &remote); // remote still empty: no upstream yet
    push_from_elsewhere(&env, &remote, "theirs.md", "remote");

    env.cmd(Some("edited"))
        .args(["open", "mine"])
        .assert()
        .success()
        .stderr(predicate::str::is_empty());
    assert!(env.notes().join("theirs.md").exists());
    assert_eq!(
        git_out(&["branch", "--format=%(refname:short)"], &remote),
        "master\n"
    );
    assert_eq!(
        git_out(&["log", "--format=%s", "master"], &remote),
        "mine: update\nmine: create\ntheirs.md\n"
    );
}

#[test]
fn init_with_new_remote_url_drops_stale_upstream() {
    let env = Env::bare();
    let old = bare_remote(&env);
    env.cmd(None)
        .arg("init")
        .write_stdin(format!("{}\n{}\n\n", env.notes().display(), old.display()))
        .assert()
        .success();
    env.cmd(Some("v1")).args(["new", "a"]).assert().success();

    let new = env.path("new.git");
    git(
        &["init", "-q", "--bare", new.to_str().unwrap()],
        env.root.path(),
    );
    init_with_remote(&env, &new);
    // The next edit pushes everything to the new, empty remote.
    env.cmd(Some("v2"))
        .args(["open", "a"])
        .assert()
        .success()
        .stderr(predicate::str::is_empty());
    assert_eq!(
        git_out(&["log", "--format=%s", "HEAD"], &new),
        "a: update\na: create\n"
    );
}

#[test]
fn emptying_staged_but_uncommitted_note_succeeds() {
    let env = Env::new();
    fs::write(env.notes().join("a.md"), "staged").unwrap();
    git(&["add", "a.md"], &env.notes());
    env.cmd(Some("")).args(["open", "a"]).assert().success();
    assert!(!env.notes().join("a.md").exists());
    assert_eq!(env.log(), "");
}

#[test]
fn invalid_name_fails_before_setup() {
    let env = Env::bare();
    env.cmd(Some("x"))
        .args(["new", "../x"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("invalid note name"));
    assert!(!env.path("config/charta/settings.toml").exists());
}

#[test]
fn double_extension_note_round_trips_through_list() {
    let env = Env::new();
    env.cmd(Some("v1"))
        .args(["new", "x.md.md"])
        .assert()
        .success();
    env.cmd(None)
        .arg("list")
        .assert()
        .success()
        .stdout("x.md.md\n");
    env.cmd(Some("v2"))
        .args(["open", "x.md.md"])
        .assert()
        .success();
    assert_eq!(
        fs::read_to_string(env.notes().join("x.md.md")).unwrap(),
        "v2"
    );
}

#[test]
fn reports_editor_error_when_commit_also_fails() {
    use std::os::unix::fs::PermissionsExt;
    let env = Env::new();
    let hook = env.notes().join(".git/hooks/pre-commit");
    fs::create_dir_all(hook.parent().unwrap()).unwrap();
    fs::write(&hook, "#!/bin/sh\nexit 1\n").unwrap();
    fs::set_permissions(&hook, fs::Permissions::from_mode(0o755)).unwrap();
    fs::write(env.notes().join("a.md"), "uncommitted").unwrap();

    env.cmd(None)
        .args(["open", "a"])
        .env("EDITOR", "/nonexistent/editor")
        .assert()
        .failure()
        .stderr(
            predicate::str::contains("failed to launch editor")
                .and(predicate::str::contains("committing it failed")),
        );
}

/// Runs `charta init` with the notes dir, `remote`, and autosync off.
fn init_without_autosync(env: &Env, remote: &Path) {
    env.cmd(None)
        .arg("init")
        .write_stdin(format!(
            "{}\n{}\noff\n",
            env.notes().display(),
            remote.display()
        ))
        .assert()
        .success();
}

fn commit_count(repo: &Path) -> String {
    git_out(&["rev-list", "--all", "--count"], repo)
}

#[test]
fn sync_pulls_and_pushes_without_autosync() {
    let env = Env::bare();
    let remote = bare_remote(&env);
    init_without_autosync(&env, &remote);
    assert!(settings(&env).contains("autosync = \"off\""));

    // Without autosync, nothing reaches the remote on its own.
    env.cmd(Some("mine")).args(["new", "a"]).assert().success();
    assert_eq!(commit_count(&remote), "0\n");

    // The remote got history elsewhere; sync starts tracking it, then pushes.
    push_from_elsewhere(&env, &remote, "b.md", "theirs");
    env.cmd(None).arg("sync").assert().success();
    assert_eq!(
        fs::read_to_string(env.notes().join("b.md")).unwrap(),
        "theirs"
    );
    assert_eq!(
        git_out(&["log", "--format=%s", "HEAD"], &remote),
        "a: create\nb.md\n"
    );

    // Once tracking, sync pushes new local commits.
    env.cmd(Some("more")).args(["new", "c"]).assert().success();
    env.cmd(None).arg("sync").assert().success();
    assert_eq!(
        git_out(&["log", "--format=%s", "-1", "HEAD"], &remote),
        "c: create\n"
    );
}

#[test]
fn sync_fails_when_offline_and_keeps_local_commits() {
    let env = Env::bare();
    let remote = bare_remote(&env);
    init_without_autosync(&env, &remote);
    env.cmd(Some("v1")).args(["new", "a"]).assert().success();
    env.cmd(None).arg("sync").assert().success();

    fs::rename(&remote, env.path("gone.git")).unwrap();
    env.cmd(Some("v2")).args(["open", "a"]).assert().success();
    // A plain network failure is no conflict to resolve.
    env.cmd(None)
        .arg("sync")
        .assert()
        .failure()
        .stderr(predicate::str::contains("resolve manually").not());
    assert_eq!(env.log(), "a: update\na: create\n");
}

#[test]
fn sync_without_setup_fails_instead_of_prompting() {
    let env = Env::bare();
    env.cmd(None)
        .arg("sync")
        .write_stdin("\n\n")
        .assert()
        .failure()
        .stderr(predicate::str::contains("not set up"));
    assert!(!env.path("config/charta/settings.toml").exists());
}

#[test]
fn save_autosync_retries_failed_push_without_a_change() {
    let env = Env::bare();
    let remote = bare_remote(&env);
    init_with_remote(&env, &remote);
    env.cmd(Some("v1")).args(["new", "a"]).assert().success();

    let gone = env.path("gone.git");
    fs::rename(&remote, &gone).unwrap();
    env.cmd(Some("v2"))
        .args(["open", "a"])
        .assert()
        .success()
        .stderr(predicate::str::contains("committed locally"));
    fs::rename(&gone, &remote).unwrap();

    env.cmd(Some("v2")).args(["open", "a"]).assert().success();
    assert_eq!(
        git_out(&["log", "--format=%s", "HEAD"], &remote),
        "a: update\na: create\n"
    );
}

#[test]
fn sync_conflict_fails_and_aborts_rebase() {
    let env = Env::bare();
    let remote = bare_remote(&env);
    init_without_autosync(&env, &remote);
    env.cmd(Some("base")).args(["new", "a"]).assert().success();
    env.cmd(None).arg("sync").assert().success();
    push_from_elsewhere(&env, &remote, "a.md", "theirs");
    env.cmd(Some("mine")).args(["open", "a"]).assert().success();

    env.cmd(None)
        .arg("sync")
        .assert()
        .failure()
        .stderr(predicate::str::contains("resolve manually"));

    let git_dir = env.notes().join(".git");
    assert!(!git_dir.join("rebase-merge").exists() && !git_dir.join("rebase-apply").exists());
    assert_eq!(
        fs::read_to_string(env.notes().join("a.md")).unwrap(),
        "mine"
    );
    assert_eq!(commit_count(&remote), "2\n");
}

#[test]
fn sync_fails_for_local_only_notes_despite_leftover_remote() {
    let env = Env::new();
    let remote = bare_remote(&env);
    git(
        &["remote", "add", "origin", remote.to_str().unwrap()],
        &env.notes(),
    );
    env.cmd(None)
        .arg("sync")
        .assert()
        .failure()
        .stderr(predicate::str::contains("local-only"));
}

#[test]
fn local_only_notes_never_autosync_despite_leftover_remote() {
    let env = Env::new();
    let remote = bare_remote(&env);
    git(
        &["remote", "add", "origin", remote.to_str().unwrap()],
        &env.notes(),
    );
    for mode in ["full", "save"] {
        env.write_config(&format!("autosync = \"{mode}\"\n"));
        env.cmd(Some(mode))
            .args(["new", mode])
            .assert()
            .success()
            .stderr(predicate::str::is_empty());
    }
    assert_eq!(commit_count(&remote), "0\n");
}

#[test]
fn sync_fails_when_repo_has_no_remote() {
    let env = Env::new();
    env.write_config("repo = \"/somewhere/notes.git\"\n");
    env.cmd(None)
        .arg("sync")
        .assert()
        .failure()
        .stderr(predicate::str::contains("has no remote"));
}

#[test]
fn sync_brings_remote_history_into_clone_without_commits() {
    let env = Env::bare();
    let remote = bare_remote(&env);
    init_without_autosync(&env, &remote); // clones the still empty remote
    push_from_elsewhere(&env, &remote, "theirs.md", "remote");

    env.cmd(None).arg("sync").assert().success();
    assert_eq!(
        fs::read_to_string(env.notes().join("theirs.md")).unwrap(),
        "remote"
    );
    assert_eq!(env.log(), "theirs.md\n");
}

#[test]
fn sync_refuses_to_pull_over_uncommitted_changes() {
    let env = Env::bare();
    let remote = bare_remote(&env);
    init_without_autosync(&env, &remote);
    env.cmd(Some("v1")).args(["new", "a"]).assert().success();
    env.cmd(None).arg("sync").assert().success();
    push_from_elsewhere(&env, &remote, "b.md", "theirs");
    // Edited outside charta, never committed.
    fs::write(env.notes().join("a.md"), "uncommitted").unwrap();

    env.cmd(None)
        .arg("sync")
        .assert()
        .failure()
        .stderr(predicate::str::contains("uncommitted changes"));
    assert_eq!(
        fs::read_to_string(env.notes().join("a.md")).unwrap(),
        "uncommitted"
    );
    assert!(!env.notes().join("b.md").exists());
    assert_eq!(env.log(), "a: create\n");
}

#[test]
fn full_autosync_pushes_after_a_change_without_pulling_again() {
    let env = Env::bare();
    let remote = bare_remote(&env);
    env.cmd(None)
        .arg("init")
        .write_stdin(format!(
            "{}\n{}\nfull\n",
            env.notes().display(),
            remote.display()
        ))
        .assert()
        .success();
    // The editor pushes a note elsewhere while charta waits for it, so a
    // pull after saving would bring it in.
    let other = env.path("other");
    let editor = format!(
        "sh -c 'printf mine > \"$1\" && cd {other:?} && printf theirs > b.md && \
         git add b.md && git commit -qm b && git push -q origin HEAD' editor",
    );
    let other_str = other.to_str().unwrap();
    git(
        &["clone", "-q", remote.to_str().unwrap(), other_str],
        env.root.path(),
    );
    env.cmd(None)
        .env("EDITOR", editor)
        .args(["new", "a"])
        .assert()
        .success()
        .stderr(predicate::str::contains("committed locally"));
    // The push was rejected (the remote moved on), and nothing was pulled.
    assert!(!env.notes().join("b.md").exists());
    assert_eq!(env.log(), "a: create\n");
}

#[test]
fn save_autosync_syncs_only_after_a_change() {
    let env = Env::bare();
    let remote = bare_remote(&env);
    env.cmd(None)
        .arg("init")
        .write_stdin(format!(
            "{}\n{}\n\n",
            env.notes().display(),
            remote.display()
        ))
        .assert()
        .success();
    assert!(settings(&env).contains("autosync = \"save\""));

    env.cmd(Some("v1")).args(["new", "a"]).assert().success();
    assert_eq!(
        git_out(&["log", "--format=%s", "HEAD"], &remote),
        "a: create\n"
    );

    // No change, no network: the remote's new note isn't pulled.
    push_from_elsewhere(&env, &remote, "b.md", "theirs");
    env.cmd(Some("v1")).args(["open", "a"]).assert().success();
    assert!(!env.notes().join("b.md").exists());

    // A change is rebased onto the remote and pushed.
    env.cmd(Some("v2")).args(["open", "a"]).assert().success();
    assert!(env.notes().join("b.md").exists());
    assert_eq!(
        git_out(&["log", "--format=%s", "HEAD"], &remote),
        "a: update\nb.md\na: create\n"
    );
}

#[test]
fn save_autosync_offline_warns_once_and_saves_locally() {
    let env = Env::new();
    env.write_config("repo = \"/nonexistent/remote.git\"\nautosync = \"save\"\n");
    git(
        &["remote", "add", "origin", "/nonexistent/remote.git"],
        &env.notes(),
    );
    let out = env
        .cmd(Some("offline"))
        .args(["new", "x"])
        .assert()
        .success()
        .get_output()
        .stderr
        .clone();
    let stderr = String::from_utf8(out).unwrap();
    assert_eq!(stderr.matches("warning").count(), 1, "{stderr}");
    assert_eq!(env.log(), "x: create\n");
}

#[test]
fn init_reprompts_on_unknown_autosync_mode() {
    let env = Env::bare();
    let remote = bare_remote(&env);
    env.cmd(None)
        .arg("init")
        .write_stdin(format!(
            "{}\n{}\nFull\nfull\n",
            env.notes().display(),
            remote.display()
        ))
        .assert()
        .success()
        .stderr(predicate::str::contains("Please answer full, save, off."));
    assert!(settings(&env).contains("autosync = \"full\""));
}
