use std::path::Path;

#[derive(Clone)]
pub struct ArchiveEntry {
    pub name: String,
    pub is_dir: bool,
    pub size: u64,
    pub compressed_size: u64,
    pub method: String,
}

fn needs_password(stderr: &str, stdout: &str) -> bool {
    let combined = format!("{} {}", stderr, stdout).to_lowercase();
    combined.contains("enter password")
        || combined.contains("wrong password")
        || combined.contains("cannot open")
        || combined.contains("can not open")
        || combined.contains("encrypted = +")
        || combined.contains("nohdr-password")
}

pub async fn list_archive(path: &Path) -> Result<Vec<ArchiveEntry>, String> {
    list_archive_with_password(path, None).await
}

pub async fn list_archive_with_password(
    path: &Path,
    password: Option<&str>,
) -> Result<Vec<ArchiveEntry>, String> {
    let name = path.file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("")
        .to_lowercase();
    let is_single_file_compr = name.ends_with(".gz")
        || name.ends_with(".bz2") || name.ends_with(".xz")
        || name.ends_with(".lz4") || name.ends_with(".lzma")
        || name.ends_with(".zst") || name.ends_with(".z");

    if is_single_file_compr {
        let tmp = std::env::temp_dir().join("sevenzip-gui-list");
        let _ = std::fs::create_dir_all(&tmp);

        let mut cmd = tokio::process::Command::new("7z");
        cmd.arg("e").arg(path).arg(format!("-o{}", tmp.display())).arg("-y");
        if let Some(pw) = password {
            cmd.arg(format!("-p{}", pw));
        }

        let output = cmd.stdin(std::process::Stdio::null()).output().await
            .map_err(|e| format!("Failed to run 7z: {}", e))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            let stdout = String::from_utf8_lossy(&output.stdout);
            let _ = std::fs::remove_dir_all(&tmp);
            if needs_password(&stderr, &stdout) {
                return Err("__NEED_PASSWORD__".to_string());
            }
            return Err(stderr.to_string());
        }

        if let Ok(read_dir) = std::fs::read_dir(&tmp) {
            for entry in read_dir.flatten() {
                let inner = entry.path();
                if inner.is_file() && inner != *path {
                    let result = do_list_archive(&inner, password).await;
                    let _ = std::fs::remove_file(&inner);
                    let _ = std::fs::remove_dir_all(&tmp);
                    return result;
                }
            }
        }
        let _ = std::fs::remove_dir_all(&tmp);
        return Err("No inner archive found".to_string());
    }

    do_list_archive(path, password).await
}

async fn do_list_archive(
    path: &Path,
    password: Option<&str>,
) -> Result<Vec<ArchiveEntry>, String> {
    if password.is_none() {
        let encrypted = detect_encryption(path).await;
        if encrypted {
            return Err("__NEED_PASSWORD__".to_string());
        }
    }

    let mut cmd = tokio::process::Command::new("7z");
    cmd.arg("l").arg("-ba");
    if let Some(pw) = password {
        cmd.arg(format!("-p{}", pw));
    }
    cmd.arg(path);

    let output = cmd
        .stdin(std::process::Stdio::null())
        .output()
        .await
        .map_err(|e| format!("Failed to run 7z: {}", e))?;

    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    let stdout = String::from_utf8_lossy(&output.stdout);

    if !output.status.success() {
        if needs_password(&stderr, &stdout) {
            return Err("__NEED_PASSWORD__".to_string());
        }
        return Err(if stderr.is_empty() { stdout.to_string() } else { stderr });
    }

    parse_listing(&stdout)
}

async fn detect_encryption(path: &Path) -> bool {
    eprintln!("[PW] detect_encryption: {}", path.display());
    let output = tokio::process::Command::new("7z")
        .args(["l", "-slt"])
        .arg(path)
        .stdin(std::process::Stdio::null())
        .output()
        .await;
    match output {
        Ok(o) => {
            let combined = String::from_utf8_lossy(&o.stdout).to_string();
            let stderr = String::from_utf8_lossy(&o.stderr).to_string();
            let all = format!("{} {}", combined, stderr).to_lowercase();
            let enc = all.contains("encrypted = +");
            eprintln!("[PW] detect_encryption={} (exit={})", enc, o.status);
            enc
        }
        Err(e) => {
            eprintln!("[PW] detect_encryption failed: {}", e);
            false
        }
    }
}

fn parse_listing(stdout: &str) -> Result<Vec<ArchiveEntry>, String> {
    let mut entries = Vec::new();

    for line in stdout.lines() {
        let parts: Vec<&str> = line.split_whitespace().collect();
        // The attributes column ("d....", "....a", ...) is always present and is the
        // reliable anchor to locate a row, since headless formats (gzip, bzip2, xz...)
        // omit the date and time columns in the "7z l -ba" output.
        for (i, token) in parts.iter().enumerate() {
            if token.len() != 5 || !token.chars().all(|c| c == '.' || "DRHSAN".contains(c)) {
                continue;
            }
            if i + 2 >= parts.len() {
                continue;
            }
            let size = match parts[i + 1].parse::<u64>() {
                Ok(size) => size,
                Err(_) => continue,
            };
            let comp = parts[i + 2].parse::<u64>().unwrap_or(0);

            let name = parts[i + 3..].join(" ");
            if name.is_empty() {
                continue;
            }

            let is_dir = token.contains('D');
            let method = if is_dir {
                String::from("DIR")
            } else {
                String::from("--")
            };

            entries.push(ArchiveEntry {
                name,
                is_dir,
                size,
                compressed_size: comp,
                method,
            });
            break;
        }
    }

    Ok(entries)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_zip_listing_with_date_columns() {
        let stdout = "\
Date      Time    Attr         Size   Compressed  Name
------------------- ----- ------------ ------------  ------------------------
2026-08-05 15:12:02 D....            0            0  src
2026-08-05 11:17:47 .....          191          130  src/main.tsx
------------------- ----- ------------ ------------  ------------------------
                                                       2 files
";
        let entries = parse_listing(stdout).unwrap();
        assert_eq!(entries.len(), 2);
        assert!(entries[0].is_dir);
        assert_eq!(entries[0].name, "src");
        assert!(!entries[1].is_dir);
        assert_eq!(entries[1].name, "src/main.tsx");
        assert_eq!(entries[1].size, 191);
        assert_eq!(entries[1].compressed_size, 130);
    }

    #[test]
    fn parses_headless_listing_without_date_columns() {
        let stdout = "\
Type = gzip
Headers Size = 10

   Date      Time    Attr         Size   Compressed  Name
------------------- ----- ------------ ------------  ------------------------
                    .....       450560       136792  workspace.tar
------------------- ----- ------------ ------------  ------------------------
                                450560       136792  1 files
";
        let entries = parse_listing(stdout).unwrap();
        assert_eq!(entries.len(), 1);
        assert!(!entries[0].is_dir);
        assert_eq!(entries[0].name, "workspace.tar");
        assert_eq!(entries[0].size, 450560);
        assert_eq!(entries[0].compressed_size, 136792);
    }

    #[test]
    fn parses_bare_listing_output() {
        let stdout = "\
2026-08-05 15:12:02 D....            0            0  tmp/opencode/ws/src
2026-08-05 11:17:47 .....          191          130  tmp/opencode/ws/src/main.tsx
2026-09-19 07:59:59 .....        16368         3703  tmp/opencode/ws/src/App.tsx
";
        let entries = parse_listing(stdout).unwrap();
        assert_eq!(entries.len(), 3);
        assert!(entries[0].is_dir);
        assert_eq!(entries[2].name, "tmp/opencode/ws/src/App.tsx");
        assert_eq!(entries[2].size, 16368);
    }
}
