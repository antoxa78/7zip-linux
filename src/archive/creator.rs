use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::time::{sleep, Duration};

pub struct ArchiveOptions {
    pub format: String,
    pub level: u32,
    pub method: String,
    pub password: Option<String>,
    pub split_size: Option<String>,
    pub encrypt_file_names: bool,
}

impl Default for ArchiveOptions {
    fn default() -> Self {
        Self {
            format: "7z".into(),
            level: 5,
            method: "LZMA2".into(),
            password: None,
            split_size: None,
            encrypt_file_names: false,
        }
    }
}

pub async fn create_archive(
    output: &Path,
    files: &[&Path],
    options: &ArchiveOptions,
    progress_tx: Option<async_channel::Sender<u8>>,
    cancel: Option<Arc<AtomicBool>>,
    pause: Option<Arc<AtomicBool>>,
) -> Result<String, String> {
    let mx_level = match options.level {
        0 => 0,
        1 => 1,
        2 => 2,
        3 => 3,
        4 => 5,
        5 => 7,
        _ => 3,
    };
    let mut args = vec![
        "a".to_string(),
        format!("-t{}", options.format),
        format!("-mx={}", mx_level),
        "-bsp1".to_string(),
        output.to_string_lossy().to_string(),
    ];

    if !options.method.is_empty() {
        args.push(format!("-m0={}", options.method));
    }

    if let Some(ref password) = options.password {
        args.push(format!("-p{}", password));
        if options.encrypt_file_names {
            args.push("-mhe=on".to_string());
        }
    }

    if let Some(ref split) = options.split_size {
        args.push(format!("-v{}", split));
    }

    for file in files {
        args.push(file.to_string_lossy().to_string());
    }

    let mut child = tokio::process::Command::new("7z")
        .args(&args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| format!("Failed to run 7z: {}", e))?;

    let child_id = match child.id() {
        Some(id) => id,
        None => return Err("7z process exited immediately".to_string()),
    };
    let mut stdout = match child.stdout.take() {
        Some(s) => s,
        None => return Err("Failed to capture 7z stdout".to_string()),
    };
    use tokio::io::AsyncReadExt;
    let mut buf = vec![0u8; 4096];

    let cancel = cancel.unwrap_or_else(|| Arc::new(AtomicBool::new(false)));
    let pause = pause.unwrap_or_else(|| Arc::new(AtomicBool::new(false)));
    let mut was_paused = false;

    loop {
        if cancel.load(Ordering::Relaxed) {
            let _ = child.kill().await;
            return Err("Cancelled".to_string());
        }

        let is_paused = pause.load(Ordering::Relaxed);
        if is_paused && !was_paused {
            #[cfg(unix)]
            {
                let _ = tokio::process::Command::new("kill")
                    .arg("-STOP")
                    .arg(child_id.to_string())
                    .status()
                    .await;
            }
            was_paused = true;
        } else if !is_paused && was_paused {
            #[cfg(unix)]
            {
                let _ = tokio::process::Command::new("kill")
                    .arg("-CONT")
                    .arg(child_id.to_string())
                    .status()
                    .await;
            }
            was_paused = false;
        }

        tokio::select! {
            result = stdout.read(&mut buf) => {
                let n = match result {
                    Ok(0) => break,
                    Ok(n) => n,
                    Err(_) => break,
                };
                if let Some(ref tx) = progress_tx {
                    let text = String::from_utf8_lossy(&buf[..n]);
                    for segment in text.split('\r') {
                        let cleaned: String = segment.chars().map(|c| if c == '\x08' { ' ' } else { c }).collect();
                        for word in cleaned.split_whitespace() {
                            if let Some(stripped) = word.strip_suffix('%') {
                                if let Ok(pct) = stripped.parse::<u8>() {
                                    let _ = tx.try_send(pct);
                                }
                            }
                        }
                    }
                }
            }
            _ = sleep(Duration::from_millis(100)) => {}
        }
    }

    let output = child.wait_with_output().await
        .map_err(|e| format!("7z failed: {}", e))?;

    if output.status.success() {
        if let Some(ref tx) = progress_tx {
            let _ = tx.send(100).await;
        }
        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    } else {
        Err(String::from_utf8_lossy(&output.stderr).to_string())
    }
}

pub async fn add_to_archive(
    archive: &Path,
    files: &[&Path],
    password: Option<&str>,
) -> Result<String, String> {
    let mut args = vec![
        "a".to_string(),
        "-y".to_string(),
    ];
    if let Some(pw) = password {
        args.push(format!("-p{}", pw));
    }
    args.push(archive.to_string_lossy().to_string());
    for file in files {
        args.push(file.to_string_lossy().to_string());
    }

    let output = tokio::process::Command::new("7z")
        .args(&args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| format!("Failed to run 7z: {}", e))?
        .wait_with_output()
        .await
        .map_err(|e| format!("7z failed: {}", e))?;

    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        let combined = format!("{} {}", stdout, stderr);
        Err(if stderr.is_empty() { combined } else { stderr.to_string() })
    }
}

pub async fn add_directory_to_archive(
    archive: &Path,
    dir_name: &str,
    password: Option<&str>,
) -> Result<String, String> {
    let temp_base = std::env::temp_dir().join("sevenzip-gui-newdir");
    let _ = std::fs::create_dir_all(&temp_base);
    let new_dir = temp_base.join(dir_name);
    std::fs::create_dir(&new_dir).map_err(|e| format!("Failed to create temp dir: {}", e))?;

    let mut args = vec![
        "a".to_string(),
        "-y".to_string(),
    ];
    if let Some(pw) = password {
        args.push(format!("-p{}", pw));
    }
    args.push(archive.to_string_lossy().to_string());
    args.push(format!("{}/", dir_name));

    let result = tokio::process::Command::new("7z")
        .current_dir(&temp_base)
        .args(&args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| format!("Failed to run 7z: {}", e))?
        .wait_with_output()
        .await
        .map_err(|e| format!("7z failed: {}", e))?;

    let _ = std::fs::remove_dir_all(&new_dir);

    if result.status.success() {
        Ok(String::from_utf8_lossy(&result.stdout).to_string())
    } else {
        let stderr = String::from_utf8_lossy(&result.stderr);
        let stdout = String::from_utf8_lossy(&result.stdout);
        let combined = format!("{} {}", stdout, stderr);
        Err(if stderr.is_empty() { combined } else { stderr.to_string() })
    }
}

pub async fn add_files_into_archive_path(
    archive: &Path,
    files: &[&Path],
    internal_prefix: &str,
    password: Option<&str>,
    progress_tx: Option<async_channel::Sender<u8>>,
) -> Result<(), String> {
    let temp_base = std::env::temp_dir().join("sevenzip-gui-internal");
    let _ = std::fs::create_dir_all(&temp_base);
    let target_dir = if internal_prefix.is_empty() {
        temp_base.clone()
    } else {
        temp_base.join(internal_prefix)
    };
    std::fs::create_dir_all(&target_dir)
        .map_err(|e| format!("Failed to create temp dir: {}", e))?;

    let mut relative_paths: Vec<String> = Vec::new();
    for file in files {
        if let Some(name) = file.file_name() {
            let dest = target_dir.join(name);
            if file.is_dir() {
                copy_dir_recursive(file, &dest)
                    .map_err(|e| format!("Failed to copy directory: {}", e))?;
            } else {
                std::fs::copy(file, &dest)
                    .map_err(|e| format!("Failed to copy file: {}", e))?;
            }
            let rel = if internal_prefix.is_empty() {
                name.to_string_lossy().to_string()
            } else {
                format!("{}/{}", internal_prefix, name.to_string_lossy())
            };
            relative_paths.push(rel);
        }
    }

    if relative_paths.is_empty() {
        let _ = std::fs::remove_dir_all(&temp_base);
        return Err("No valid files to add".to_string());
    }

    let mut args = vec!["a".to_string(), "-y".to_string(), "-bsp1".to_string()];
    if let Some(pw) = password {
        args.push(format!("-p{}", pw));
    }
    args.push(archive.to_string_lossy().to_string());
    args.extend(relative_paths);

    let mut child = tokio::process::Command::new("7z")
        .current_dir(&temp_base)
        .args(&args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| format!("Failed to run 7z: {}", e))?;

    let mut stdout = child.stdout.take()
        .ok_or_else(|| "Failed to capture 7z stdout".to_string())?;
    use tokio::io::AsyncReadExt;
    let mut buf = vec![0u8; 4096];
    let mut stdout_buf = Vec::new();

    loop {
        let n = stdout.read(&mut buf).await
            .map_err(|_| "Failed to read 7z stdout".to_string())?;
        if n == 0 {
            break;
        }
        stdout_buf.extend_from_slice(&buf[..n]);
        if let Some(ref tx) = progress_tx {
            let text = String::from_utf8_lossy(&buf[..n]);
            for segment in text.split('\r') {
                let cleaned: String = segment.chars().map(|c| if c == '\x08' { ' ' } else { c }).collect();
                for word in cleaned.split_whitespace() {
                    if let Some(stripped) = word.strip_suffix('%') {
                        if let Ok(pct) = stripped.parse::<u8>() {
                            let _ = tx.try_send(pct);
                        }
                    }
                }
            }
        }
    }

    let output = child.wait_with_output().await
        .map_err(|e| format!("7z failed: {}", e))?;

    let _ = std::fs::remove_dir_all(&temp_base);

    if output.status.success() {
        if let Some(ref tx) = progress_tx {
            let _ = tx.send(100).await;
        }
        Ok(())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout_str = String::from_utf8_lossy(&stdout_buf);
        let combined = format!("{} {}", stdout_str, stderr);
        Err(if stderr.is_empty() { combined } else { stderr.to_string() })
    }
}

pub async fn delete_entry_from_archive(
    archive: &Path,
    internal_path: &str,
    password: Option<&str>,
) -> Result<(), String> {
    let mut args = vec![
        "d".to_string(),
        "-y".to_string(),
    ];
    if let Some(pw) = password {
        args.push(format!("-p{}", pw));
    }
    args.push(archive.to_string_lossy().to_string());
    args.push(internal_path.to_string());

    let output = tokio::process::Command::new("7z")
        .args(&args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| format!("Failed to run 7z: {}", e))?
        .wait_with_output()
        .await
        .map_err(|e| format!("7z failed: {}", e))?;

    if output.status.success() {
        Ok(())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        let combined = format!("{} {}", stdout, stderr);
        Err(if stderr.is_empty() { combined } else { stderr.to_string() })
    }
}

pub async fn rename_entry_in_archive(
    archive: &Path,
    old_path: &str,
    new_name: &str,
    password: Option<&str>,
) -> Result<(), String> {
    let new_path = if let Some(slash) = old_path.rfind('/') {
        format!("{}/{}", &old_path[..slash + 1], new_name)
    } else {
        new_name.to_string()
    };

    let mut args = vec![
        "rn".to_string(),
        "-y".to_string(),
    ];
    if let Some(pw) = password {
        args.push(format!("-p{}", pw));
    }
    args.push(archive.to_string_lossy().to_string());
    args.push(old_path.to_string());
    args.push(new_path);

    let output = tokio::process::Command::new("7z")
        .args(&args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| format!("Failed to run 7z: {}", e))?
        .wait_with_output()
        .await
        .map_err(|e| format!("7z failed: {}", e))?;

    if output.status.success() {
        Ok(())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        let combined = format!("{} {}", stdout, stderr);
        Err(if stderr.is_empty() { combined } else { stderr.to_string() })
    }
}

fn copy_dir_recursive(src: &Path, dest: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dest)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let src_path = entry.path();
        let dest_path = dest.join(entry.file_name());
        if src_path.is_dir() {
            copy_dir_recursive(&src_path, &dest_path)?;
        } else {
            std::fs::copy(&src_path, &dest_path)?;
        }
    }
    Ok(())
}
