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
    cmd.arg("l").arg("-slt");
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
    let mut current: Option<ArchiveEntry> = None;
    // Only parse entry blocks after the "----------" separator that follows the
    // archive metadata header; the block before it describes the archive itself.
    let mut started = false;

    for line in stdout.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if line == "----------" {
            started = true;
            continue;
        }
        if !started {
            continue;
        }
        if let Some(rest) = line.strip_prefix("Path = ") {
            if let Some(entry) = current.take() {
                entries.push(entry);
            }
            current = Some(ArchiveEntry {
                name: rest.trim().to_string(),
                is_dir: false,
                size: 0,
                compressed_size: 0,
                method: String::new(),
            });
        } else if let Some(entry) = current.as_mut() {
            if let Some(v) = line.strip_prefix("Size = ") {
                entry.size = v.trim().parse().unwrap_or(0);
            } else if let Some(v) = line.strip_prefix("Packed Size = ") {
                entry.compressed_size = v.trim().parse().unwrap_or(0);
            } else if let Some(v) = line.strip_prefix("Attributes = ") {
                entry.is_dir = v.trim().contains('D');
            }
        }
    }

    if let Some(entry) = current.take() {
        entries.push(entry);
    }

    Ok(entries)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_listing_with_nested_files_and_dirs() {
        let stdout = "\
----------
Path = src
Size = 0
Packed Size = 0
Modified = 2026-08-05 15:12:02.0000000
Attributes = D
CRC = 

Path = src/main.tsx
Size = 191
Packed Size = 130
Modified = 2026-08-05 11:17:47.0000000
Attributes = A
CRC = 

Path = src/App.tsx
Size = 16368
Packed Size = 3703
Modified = 2026-09-19 07:59:59.0000000
Attributes = A
CRC = 
";
        let entries = parse_listing(stdout).unwrap();
        assert_eq!(entries.len(), 3);
        assert!(entries[0].is_dir);
        assert_eq!(entries[0].name, "src");
        assert!(!entries[1].is_dir);
        assert_eq!(entries[1].name, "src/main.tsx");
        assert_eq!(entries[1].size, 191);
        assert_eq!(entries[1].compressed_size, 130);
        assert_eq!(entries[2].size, 16368);
    }

    #[test]
    fn parses_headless_listing_without_attributes() {
        let stdout = "\
Path = t.tar.gz
Type = gzip
Headers Size = 10

----------
Path = workspace.tar
Size = 450560
Packed Size = 136792
Modified = 
Host OS = Unix
CRC = 

";
        let entries = parse_listing(stdout).unwrap();
        assert_eq!(entries.len(), 1);
        assert!(!entries[0].is_dir);
        assert_eq!(entries[0].name, "workspace.tar");
        assert_eq!(entries[0].size, 450560);
        assert_eq!(entries[0].compressed_size, 136792);
    }

    #[test]
    fn parses_solid_archive_with_missing_packed_sizes() {
        let stdout = "\
Path = /tmp/simpledlna-1.0.7z
Type = 7z
Physical Size = 1342690
Headers Size = 557
Method = LZMA:22 BCJ2
Solid = +
Blocks = 2

----------
Path = sdlna.exe
Size = 160256
Packed Size = 1341649
Modified = 2014-10-21 00:15:02.0000000
Attributes = A
CRC = 73FE04AE
Encrypted = -
Method = BCJ2 LZMA:22 LZMA:20:lc0:lp2 LZMA:20:lc0:lp2
Block = 1

Path = SimpleDLNA.exe
Size = 415744
Packed Size = 
Modified = 2014-10-21 00:15:02.0000000
Attributes = A
CRC = 8DFC0065
Encrypted = -
Method = BCJ2 LZMA:22 LZMA:20:lc0:lp2 LZMA:20:lc0:lp2
Block = 1

Path = x86/SQLite.Interop.dll
Size = 891392
Packed Size = 
Modified = 2014-10-16 06:58:52.0000000
Attributes = A
CRC = 7D99EECC
Encrypted = -
Method = BCJ2 LZMA:22 LZMA:20:lc0:lp2 LZMA:20:lc0:lp2
Block = 1

Path = x64/SQLite.Interop.dll
Size = 1136128
Packed Size = 
Modified = 2014-10-16 06:58:52.0000000
Attributes = A
CRC = 7D99EECC
Encrypted = -
Method = BCJ2 LZMA:22 LZMA:20:lc0:lp2 LZMA:20:lc0:lp2
Block = 1

Path = x86
Size = 0
Packed Size = 0
Modified = 2014-10-21 00:16:29.0000000
Attributes = D
CRC = 
Encrypted = -
Method = 
Block = 

Path = x64
Size = 0
Packed Size = 0
Modified = 2014-10-21 00:16:29.0000000
Attributes = D
CRC = 
Encrypted = -
Method = 
Block = 
";
        let entries = parse_listing(stdout).unwrap();
        assert_eq!(entries.len(), 6);
        assert_eq!(entries[2].name, "x86/SQLite.Interop.dll");
        assert_eq!(entries[2].size, 891392);
        assert!(!entries[2].is_dir);
        assert_eq!(entries[3].name, "x64/SQLite.Interop.dll");
        assert_eq!(entries[3].size, 1136128);
        assert!(!entries[3].is_dir);
        assert!(entries[4].is_dir);
        assert_eq!(entries[4].name, "x86");
        assert!(entries[5].is_dir);
        assert_eq!(entries[5].name, "x64");
    }

    #[test]
    fn ignores_archive_metadata_header() {
        let stdout = "\
Path = /tmp/test.zip
Type = zip
Physical Size = 1000

----------
Path = README.md
Size = 500
Packed Size = 300
Modified = 2026-08-05 11:17:47.0000000
Attributes = A
CRC = 

";
        let entries = parse_listing(stdout).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].name, "README.md");
    }
}
