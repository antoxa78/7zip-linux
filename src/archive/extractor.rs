use std::path::Path;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::time::{sleep, Duration};

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
                let _ = std::process::Command::new("kill")
                    .arg("-STOP")
                    .arg(child_id.to_string())
                    .status();
            }
            was_paused = true;
        } else if !is_paused && was_paused {
            #[cfg(unix)]
            {
                let _ = std::process::Command::new("kill")
                    .arg("-CONT")
                    .arg(child_id.to_string())
                    .status();
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
            let _ = tx.try_send(100);
        }
        Ok(String::from_utf8_lossy(&output.stdout).to_string())
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
    std::fs::create_dir_all(dest_dir).map_err(|e| e.to_string())?;

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
