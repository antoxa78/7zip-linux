//! Small filesystem helpers shared by paste, drag & drop and extraction.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// Recursively copies `src` to `dst`. Directories are merged into an existing
/// directory; files overwrite existing files. Symlinks are copied as links.
pub fn copy_recursive(src: &Path, dst: &Path) -> io::Result<()> {
    let meta = fs::symlink_metadata(src)?;
    if meta.file_type().is_symlink() {
        let target = fs::read_link(src)?;
        if fs::symlink_metadata(dst).is_ok() {
            remove_path(dst)?;
        }
        #[cfg(unix)]
        std::os::unix::fs::symlink(target, dst)?;
        return Ok(());
    }
    if meta.is_dir() {
        fs::create_dir_all(dst)?;
        for entry in fs::read_dir(src)? {
            let entry = entry?;
            copy_recursive(&entry.path(), &dst.join(entry.file_name()))?;
        }
        Ok(())
    } else {
        fs::copy(src, dst).map(|_| ())
    }
}

/// Removes a file, symlink or whole directory tree.
pub fn remove_path(path: &Path) -> io::Result<()> {
    let meta = fs::symlink_metadata(path)?;
    if meta.is_dir() {
        fs::remove_dir_all(path)
    } else {
        fs::remove_file(path)
    }
}

/// Moves `src` to `dst`, merging directories and overwriting files.
/// Falls back to copy + delete when a rename is not possible (e.g. across devices).
/// Refuses to replace a directory with a file or vice versa.
pub fn move_merge(src: &Path, dst: &Path) -> io::Result<()> {
    let src_meta = fs::symlink_metadata(src)?;
    if let Ok(dst_meta) = fs::symlink_metadata(dst) {
        if src_meta.is_dir() && dst_meta.is_dir() {
            for entry in fs::read_dir(src)? {
                let entry = entry?;
                move_merge(&entry.path(), &dst.join(entry.file_name()))?;
            }
            return fs::remove_dir(src);
        }
        if src_meta.is_dir() != dst_meta.is_dir() {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                format!(
                    "\"{}\" already exists and is a {}",
                    dst.display(),
                    if dst_meta.is_dir() { "folder" } else { "file" }
                ),
            ));
        }
    }
    match fs::rename(src, dst) {
        Ok(()) => Ok(()),
        Err(_) => {
            copy_recursive(src, dst)?;
            remove_path(src)
        }
    }
}

/// True when `path` is `base` itself or lies somewhere inside it.
pub fn is_same_or_inside(path: &Path, base: &Path) -> bool {
    let canon = |p: &Path| fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
    canon(path).starts_with(canon(base))
}

/// Returns `dir/name (copy).ext`, `dir/name (copy 2).ext`, ... — the first that does not exist.
pub fn unique_copy_name(dir: &Path, name: &str, is_dir: bool) -> PathBuf {
    let (stem, ext) = if is_dir {
        (name.to_string(), String::new())
    } else {
        match name.rfind('.') {
            Some(i) if i > 0 => (name[..i].to_string(), name[i..].to_string()),
            _ => (name.to_string(), String::new()),
        }
    };
    let mut n = 1u32;
    loop {
        let candidate = if n == 1 {
            format!("{} (copy){}", stem, ext)
        } else {
            format!("{} (copy {}){}", stem, n, ext)
        };
        let path = dir.join(&candidate);
        if fs::symlink_metadata(&path).is_err() {
            return path;
        }
        n += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("fsops-test-{}-{}", tag, std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn copy_name_keeps_extension() {
        let d = tmp("copyname");
        fs::write(d.join("a.txt"), "x").unwrap();
        assert_eq!(unique_copy_name(&d, "a.txt", false), d.join("a (copy).txt"));
        fs::write(d.join("a (copy).txt"), "x").unwrap();
        assert_eq!(unique_copy_name(&d, "a.txt", false), d.join("a (copy 2).txt"));
        assert_eq!(unique_copy_name(&d, ".bashrc", false), d.join(".bashrc (copy)"));
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn move_merge_merges_directories() {
        let d = tmp("merge");
        fs::create_dir_all(d.join("src/sub")).unwrap();
        fs::write(d.join("src/sub/new.txt"), "new").unwrap();
        fs::write(d.join("src/same.txt"), "updated").unwrap();
        fs::create_dir_all(d.join("dst/sub")).unwrap();
        fs::write(d.join("dst/sub/old.txt"), "old").unwrap();
        fs::write(d.join("dst/same.txt"), "stale").unwrap();
        move_merge(&d.join("src"), &d.join("dst")).unwrap();
        assert!(!d.join("src").exists());
        assert_eq!(fs::read_to_string(d.join("dst/sub/old.txt")).unwrap(), "old");
        assert_eq!(fs::read_to_string(d.join("dst/sub/new.txt")).unwrap(), "new");
        assert_eq!(fs::read_to_string(d.join("dst/same.txt")).unwrap(), "updated");
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn inside_detection() {
        let d = tmp("inside");
        fs::create_dir_all(d.join("a/b")).unwrap();
        assert!(is_same_or_inside(&d.join("a/b"), &d.join("a")));
        assert!(is_same_or_inside(&d.join("a"), &d.join("a")));
        assert!(!is_same_or_inside(&d.join("a"), &d.join("a/b")));
        let _ = fs::remove_dir_all(&d);
    }
}
