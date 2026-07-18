use std::path::Path;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::time::{sleep, Duration};

fn is_tar_compressed(name: &str) -> bool {
    let lower = name.to_lowercase();
    lower.contains(".tar.") && (lower.ends_with(".gz") || lower.ends_with(".bz2")
        || lower.ends_with(".xz") || lower.ends_with(".zst")
        || lower.ends_with(".lz4") || lower.ends_with(".lzma")
        || lower.ends_with(".z"))
}

pub struct ExtractOptions {
    pub output_dir: PathBuf,
    pub full_paths: bool,
    pub overwrite: OverwriteMode,
    pub password: Option<String>,
}

#[derive(Default)]
pub enum OverwriteMode {
    #[default]
    Overwrite,
    SkipExisting,
    AutoRename,
}

pub async fn extract_archive(
    archive: &Path,
    options: &ExtractOptions,
    progress_tx: Option<async_channel::Sender<u8>>,
    cancel: Option<Arc<AtomicBool>>,
    pause: Option<Arc<AtomicBool>>,
) -> Result<String, String> {
    let archive_name = archive.file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("");

    if is_tar_compressed(archive_name) {
        let tmp = std::env::temp_dir().join("sevenzip-gui-list");
        let _ = std::fs::create_dir_all(&tmp);
        let _ = std::fs::remove_dir_all(&tmp);

        let mut phase1 = tokio::process::Command::new("7z");
        phase1.arg("e").arg(archive).arg(format!("-o{}", tmp.display())).arg("-y");
        if let Some(ref pw) = options.password {
            phase1.arg(format!("-p{}", pw));
        }
        let out1 = phase1.stdin(std::process::Stdio::null()).output().await
            .map_err(|e| format!("Failed to run 7z: {}", e))?;
        if !out1.status.success() {
            let stderr = String::from_utf8_lossy(&out1.stderr).to_string();
            let stdout = String::from_utf8_lossy(&out1.stdout).to_string();
            let _ = std::fs::remove_dir_all(&tmp);
            if needs_password(&stderr, &stdout) {
                return Err("__NEED_PASSWORD__".to_string());
            }
            return Err(stderr);
        }

        let inner_tar = std::fs::read_dir(&tmp)
            .map_err(|e| e.to_string())?
            .filter_map(|e| e.ok())
            .find(|e| e.path().is_file() && e.path() != *archive)
            .map(|e| e.path())
            .ok_or_else(|| {
                let _ = std::fs::remove_dir_all(&tmp);
                "Inner tar not found in compressed archive".to_string()
            })?;

        let result = extract_archive_inner(&inner_tar, options, progress_tx, cancel, pause).await;
        let _ = std::fs::remove_dir_all(&tmp);
        result
    } else {
        extract_archive_inner(archive, options, progress_tx, cancel, pause).await
    }
}

async fn extract_archive_inner(
    archive: &Path,
    options: &ExtractOptions,
    progress_tx: Option<async_channel::Sender<u8>>,
    cancel: Option<Arc<AtomicBool>>,
    pause: Option<Arc<AtomicBool>>,
) -> Result<String, String> {
    let cmd = if options.full_paths { "x" } else { "e" };
    let mut args = vec![
        cmd.to_string(),
        archive.to_string_lossy().to_string(),
        format!("-o{}", options.output_dir.display()),
        "-bsp1".to_string(),
    ];

    match &options.overwrite {
        OverwriteMode::Overwrite => args.push("-aoa".to_string()),
        OverwriteMode::SkipExisting => args.push("-aos".to_string()),
        OverwriteMode::AutoRename => args.push("-aou".to_string()),
    }

    if let Some(ref password) = &options.password {
        args.push(format!("-p{}", password));
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
    let mut stdout_buf = Vec::new();

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
            _ = sleep(Duration::from_millis(100)) => {}
        }
    }

    let output = child.wait_with_output().await
        .map_err(|e| format!("7z failed: {}", e))?;

    if output.status.success() {
        if let Some(ref tx) = progress_tx {
            let _ = tx.send(100).await;
        }
        Ok(String::from_utf8_lossy(&stdout_buf).to_string())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        let stdout_str = String::from_utf8_lossy(&stdout_buf).to_string();
        if needs_password(&stderr, &stdout_str) {
            Err("__NEED_PASSWORD__".to_string())
        } else {
            Err(stderr)
        }
    }
}

fn needs_password(stderr: &str, stdout: &str) -> bool {
    let combined = format!("{} {}", stderr, stdout).to_lowercase();
    combined.contains("enter password")
        || combined.contains("wrong password")
        || combined.contains("cannot open")
        || combined.contains("can not open")
        || combined.contains("encrypted")
}

pub async fn extract_entry(
    archive: &Path,
    internal_path: &str,
    dest_dir: &Path,
    password: Option<&str>,
) -> Result<(), String> {
    if internal_path.is_empty() {
        return Err("Internal path is empty".to_string());
    }
    std::fs::create_dir_all(dest_dir).map_err(|e| e.to_string())?;

    let archive_name = archive.file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("");

    if is_tar_compressed(archive_name) {
        let tmp = std::env::temp_dir().join("sevenzip-gui-list");
        let _ = std::fs::create_dir_all(&tmp);
        let _ = std::fs::remove_dir_all(&tmp);

        let mut phase1 = tokio::process::Command::new("7z");
        phase1.arg("e").arg(archive).arg(format!("-o{}", tmp.display())).arg("-y");
        if let Some(pw) = password {
            phase1.arg(format!("-p{}", pw));
        }
        let out1 = phase1.stdin(std::process::Stdio::null()).output().await
            .map_err(|e| format!("Failed to run 7z: {}", e))?;
        if !out1.status.success() {
            let stderr = String::from_utf8_lossy(&out1.stderr).to_string();
            let stdout = String::from_utf8_lossy(&out1.stdout).to_string();
            let _ = std::fs::remove_dir_all(&tmp);
            if needs_password(&stderr, &stdout) {
                return Err("__NEED_PASSWORD__".to_string());
            }
            return Err(stderr);
        }

        let inner_tar = std::fs::read_dir(&tmp)
            .map_err(|e| e.to_string())?
            .filter_map(|e| e.ok())
            .find(|e| e.path().is_file() && e.path() != *archive)
            .map(|e| e.path())
            .ok_or_else(|| {
                let _ = std::fs::remove_dir_all(&tmp);
                "Inner tar not found in compressed archive".to_string()
            })?;

        let is_dir = internal_path.ends_with('/');
        let cmd = if is_dir { "x" } else { "e" };
        let mut phase2 = tokio::process::Command::new("7z");
        phase2.arg(cmd).arg(&inner_tar).arg(internal_path)
            .arg(format!("-o{}", dest_dir.display())).arg("-y");
        if let Some(pw) = password {
            phase2.arg(format!("-p{}", pw));
        }
        let out2 = phase2.stdin(std::process::Stdio::null()).output().await
            .map_err(|e| format!("Failed to run 7z: {}", e))?;
        let _ = std::fs::remove_dir_all(&tmp);

        if out2.status.success() {
            Ok(())
        } else {
            let stderr = String::from_utf8_lossy(&out2.stderr).to_string();
            let stdout = String::from_utf8_lossy(&out2.stdout).to_string();
            if needs_password(&stderr, &stdout) {
                Err("__NEED_PASSWORD__".to_string())
            } else {
                Err(stderr)
            }
        }
    } else {
        let is_dir = internal_path.ends_with('/');
        let cmd = if is_dir { "x" } else { "e" };

        let mut args = vec![
            cmd.to_string(),
            archive.to_string_lossy().to_string(),
            internal_path.to_string(),
            format!("-o{}", dest_dir.display()),
            "-y".to_string(),
        ];

        if let Some(pw) = password {
            args.push(format!("-p{}", pw));
        }

        let output = tokio::process::Command::new("7z")
            .args(&args)
            .stdin(std::process::Stdio::null())
            .output()
            .await
            .map_err(|e| format!("Failed to run 7z: {}", e))?;

        if output.status.success() {
            Ok(())
        } else {
            let stderr = String::from_utf8_lossy(&output.stderr).to_string();
            let stdout = String::from_utf8_lossy(&output.stdout).to_string();
            if needs_password(&stderr, &stdout) {
                Err("__NEED_PASSWORD__".to_string())
            } else {
                Err(stderr)
            }
        }
    }
}
