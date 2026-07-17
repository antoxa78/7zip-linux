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
        Err(String::from_utf8_lossy(&output.stderr).to_string())
    }
}
