use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Result, bail};
use chrono::NaiveDateTime;

pub const EXT: &str = "md";
const EXT_SEPARATOR: char = '.';
/// Used when the current directory has no usable name (e.g. `/`).
const FALLBACK_NAME: &str = "note";
const TIMESTAMP_FORMAT: &str = "%Y-%m-%d-%H%M";
/// Separates subdirectories in note names, independent of the OS.
const NAME_SEPARATOR: &str = "/";
const HIDDEN_PREFIX: char = '.';

/// Validates a user-supplied note name and strips an optional [`EXT`] suffix.
/// Names may contain [`NAME_SEPARATOR`] to place notes in subdirectories.
pub fn normalize(name: &str) -> Result<String> {
    let name = strip_ext(name.trim());
    // Empty parts catch absolute paths and leading, trailing, or doubled
    // separators; the hidden prefix catches `.`, `..`, and `.git`.
    let valid = name
        .split(NAME_SEPARATOR)
        .all(|part| !part.is_empty() && !part.starts_with(HIDDEN_PREFIX));
    if !valid {
        bail!("invalid note name `{name}`");
    }
    Ok(name.to_string())
}

fn strip_ext(name: &str) -> &str {
    name.strip_suffix(EXT)
        .and_then(|n| n.strip_suffix(EXT_SEPARATOR))
        .unwrap_or(name)
}

pub fn path(notes_dir: &Path, name: &str) -> PathBuf {
    notes_dir.join(format!("{name}{EXT_SEPARATOR}{EXT}"))
}

pub fn exists(notes_dir: &Path, name: &str) -> bool {
    path(notes_dir, name).exists()
}

/// Name for a note created without an explicit name: the current directory's
/// name, with a timestamp appended if that note already exists.
pub fn default_name(notes_dir: &Path, cwd: &Path, now: NaiveDateTime) -> String {
    let base = cwd
        .file_name()
        .map(|n| {
            let n = n.to_string_lossy();
            strip_ext(n.trim().trim_start_matches(HIDDEN_PREFIX)).to_string()
        })
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| FALLBACK_NAME.to_string());
    if !exists(notes_dir, &base) {
        return base;
    }
    let stamped = format!("{base}-{}", now.format(TIMESTAMP_FORMAT));
    if !exists(notes_dir, &stamped) {
        return stamped;
    }
    (2..)
        .map(|n| format!("{stamped}-{n}"))
        .find(|name| !exists(notes_dir, name))
        .expect("unbounded range")
}

/// All note names, sorted, relative to the notes dir and without extension.
/// Hidden files and directories (including `.git`) are skipped.
/// Every name round-trips through [`normalize`], so it can be opened.
pub fn list(notes_dir: &Path) -> Result<Vec<String>> {
    let mut names = Vec::new();
    collect(notes_dir, notes_dir, &mut names)?;
    names.sort();
    Ok(names)
}

fn collect(root: &Path, dir: &Path, names: &mut Vec<String>) -> Result<()> {
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        if entry
            .file_name()
            .to_string_lossy()
            .starts_with(HIDDEN_PREFIX)
        {
            continue;
        }
        let path = entry.path();
        if entry.file_type()?.is_dir() {
            collect(root, &path, names)?;
        } else if path.extension().is_some_and(|e| e == EXT) {
            let rel = path.strip_prefix(root)?.with_extension("");
            let parts: Vec<_> = rel
                .components()
                .map(|c| c.as_os_str().to_string_lossy())
                .collect();
            let name = parts.join(NAME_SEPARATOR);
            // `x.md.md` is listed as is: `normalize` strips only one extension.
            if strip_ext(&name) == name {
                names.push(name);
            } else {
                names.push(format!("{name}{EXT_SEPARATOR}{EXT}"));
            }
        }
    }
    Ok(())
}

/// A note counts as empty if it contains only whitespace.
pub fn is_blank(path: &Path) -> Result<bool> {
    Ok(fs::read(path)?.iter().all(u8::is_ascii_whitespace))
}

/// Deletes a note and any directories left empty by that, up to `notes_dir`.
pub fn remove(notes_dir: &Path, path: &Path) -> Result<()> {
    fs::remove_file(path)?;
    let mut dir = path.parent();
    while let Some(d) = dir {
        if d == notes_dir || fs::remove_dir(d).is_err() {
            break;
        }
        dir = d.parent();
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;

    fn now() -> NaiveDateTime {
        NaiveDate::from_ymd_opt(2026, 11, 10)
            .unwrap()
            .and_hms_opt(14, 32, 0)
            .unwrap()
    }

    fn touch(dir: &Path, name: &str) {
        let p = path(dir, name);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, "x").unwrap();
    }

    #[test]
    fn normalize_accepts_plain_and_nested_names() {
        assert_eq!(normalize("todo").unwrap(), "todo");
        assert_eq!(normalize("todo.md").unwrap(), "todo");
        assert_eq!(normalize("work/todo").unwrap(), "work/todo");
    }

    #[test]
    fn normalize_rejects_escaping_and_hidden_names() {
        for bad in [
            "", ".md", "../x", "/etc/x", "a/../b", ".hidden", "a/.git/x", "./x", "work/", "a//b",
        ] {
            assert!(normalize(bad).is_err(), "{bad} should be rejected");
        }
    }

    #[test]
    fn default_name_uses_dir_then_timestamp_then_counter() {
        let dir = tempfile::tempdir().unwrap();
        let cwd = Path::new("/home/me/api");
        assert_eq!(default_name(dir.path(), cwd, now()), "api");
        touch(dir.path(), "api");
        assert_eq!(default_name(dir.path(), cwd, now()), "api-2026-11-10-1432");
        touch(dir.path(), "api-2026-11-10-1432");
        assert_eq!(
            default_name(dir.path(), cwd, now()),
            "api-2026-11-10-1432-2"
        );
    }

    #[test]
    fn default_name_handles_dotdirs_and_root() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(
            default_name(dir.path(), Path::new("/home/me/.config"), now()),
            "config"
        );
        assert_eq!(default_name(dir.path(), Path::new("/"), now()), "note");
    }

    #[test]
    fn default_name_strips_extension_so_note_can_be_opened() {
        let dir = tempfile::tempdir().unwrap();
        let name = default_name(dir.path(), Path::new("/home/me/readme.md"), now());
        assert_eq!(name, "readme");
        assert_eq!(normalize(&format!("{name}.{EXT}")).unwrap(), name);
    }

    #[test]
    fn list_finds_nested_notes_and_skips_hidden() {
        let dir = tempfile::tempdir().unwrap();
        touch(dir.path(), "b");
        touch(dir.path(), "a/c");
        fs::create_dir_all(dir.path().join(crate::git::GIT_DIR)).unwrap();
        fs::write(dir.path().join(".git/x.md"), "").unwrap();
        fs::write(dir.path().join("readme.txt"), "").unwrap();
        assert_eq!(list(dir.path()).unwrap(), vec!["a/c", "b"]);
    }

    #[test]
    fn blank_detection() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("n.md");
        fs::write(&p, " \n\t\n").unwrap();
        assert!(is_blank(&p).unwrap());
        fs::write(&p, "\nhi\n").unwrap();
        assert!(!is_blank(&p).unwrap());
    }

    #[test]
    fn remove_cleans_up_empty_parents() {
        let dir = tempfile::tempdir().unwrap();
        touch(dir.path(), "a/b/c");
        remove(dir.path(), &path(dir.path(), "a/b/c")).unwrap();
        assert!(!dir.path().join("a").exists());
        assert!(dir.path().exists());
    }
}
