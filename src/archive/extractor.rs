use std::path::Path;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::time::{sleep, Duration};

use crate::utils::NEED_PASSWORD;

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

/// Decompresses the outer layer of a `.tar.gz`-style archive into a private temp dir
/// and returns (temp dir, inner tar path). The caller must remove the temp dir.
async fn unpack_outer_layer(
    archive: &Path,
    password: Option<&str>,
) -> Result<(PathBuf, PathBuf), String> {
    let tmp = crate::utils::unique_temp_dir("tar")
        .map_err(|e| format!("Failed to create temp dir: {}", e))?;

    let mut phase1 = tokio::process::Command::new("7z");
    phase1.arg("e").arg(archive).arg(format!("-o{}", tmp.display())).arg("-y");
    if let Some(pw) = password {
        phase1.arg(format!("-p{}", pw));
    }
    let out1 = match phase1.stdin(std::process::Stdio::null()).output().await {
        Ok(o) => o,
        Err(e) => {
            let _ = std::fs::remove_dir_all(&tmp);
            return Err(format!("Failed to run 7z: {}", e));
        }
    };
    if !out1.status.success() {
        let stderr = String::from_utf8_lossy(&out1.stderr).to_string();
        let stdout = String::from_utf8_lossy(&out1.stdout).to_string();
        let _ = std::fs::remove_dir_all(&tmp);
        if needs_password(&stderr, &stdout) {
            return Err(NEED_PASSWORD.to_string());
        }
        return Err(stderr);
    }

    let inner = std::fs::read_dir(&tmp)
        .ok()
        .and_then(|rd| {
            rd.filter_map(|e| e.ok())
                .map(|e| e.path())
                .find(|p| p.is_file())
        });
    match inner {
        Some(inner) => Ok((tmp, inner)),
        None => {
            let _ = std::fs::remove_dir_all(&tmp);
            Err("Inner tar not found in compressed archive".to_string())
        }
    }
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
        let (tmp, inner_tar) = unpack_outer_layer(archive, options.password.as_deref()).await?;
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
    let stderr_task = super::proc::drain_stderr(&mut child);
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

    let status = child.wait().await
        .map_err(|e| format!("7z failed: {}", e))?;
    let stderr = super::proc::collect_stderr(stderr_task).await;

    if status.success() {
        if let Some(ref tx) = progress_tx {
            let _ = tx.send(100).await;
        }
        Ok(String::from_utf8_lossy(&stdout_buf).to_string())
    } else {
        let stdout_str = String::from_utf8_lossy(&stdout_buf).to_string();
        if needs_password(&stderr, &stdout_str) {
            Err(NEED_PASSWORD.to_string())
        } else if stderr.trim().is_empty() {
            Err(stdout_str)
        } else {
            Err(stderr)
        }
    }
}

/// True only when 7z's output points at a missing or wrong password.
/// (Plain "cannot open" errors mean "not an archive / corrupt", not "encrypted".)
pub fn needs_password(stderr: &str, stdout: &str) -> bool {
    let combined = format!("{} {}", stderr, stdout).to_lowercase();
    combined.contains("enter password")
        || combined.contains("wrong password")
        || combined.contains("encrypted archive")
        || combined.contains("nohdr-password")
}

/// Runs `7z x` for a single entry (file or folder) into `out_dir`, keeping full paths.
async fn run_extract_full_paths(
    archive: &Path,
    internal_path: &str,
    out_dir: &Path,
    password: Option<&str>,
) -> Result<(), String> {
    let mut cmd = tokio::process::Command::new("7z");
    cmd.arg("x")
        .arg(archive)
        .arg(internal_path)
        .arg(format!("-o{}", out_dir.display()))
        .arg("-y");
    if let Some(pw) = password {
        cmd.arg(format!("-p{}", pw));
    }
    let output = cmd
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
            Err(NEED_PASSWORD.to_string())
        } else if stderr.trim().is_empty() {
            Err(stdout)
        } else {
            Err(stderr)
        }
    }
}

/// Creates a hidden staging directory inside `dest_dir` (so the final move is a cheap
/// rename on the same filesystem), falling back to a private temp dir.
fn make_stage_dir(dest_dir: &Path) -> Result<PathBuf, String> {
    use std::sync::atomic::AtomicU64;
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let candidate = dest_dir.join(format!(".7zip-linux-extract-{}-{}", std::process::id(), n));
    if std::fs::create_dir(&candidate).is_ok() {
        return Ok(candidate);
    }
    crate::utils::unique_temp_dir("extract").map_err(|e| format!("Failed to create temp dir: {}", e))
}

/// Extracts one entry (a file *or a folder with all its contents*) from an archive and
/// places it at `dest_dir/<entry name>`, preserving the folder structure below it.
/// Existing folders are merged and existing files overwritten.
pub async fn extract_entry(
    archive: &Path,
    internal_path: &str,
    dest_dir: &Path,
    password: Option<&str>,
) -> Result<(), String> {
    let internal = internal_path.trim_matches('/').to_string();
    if internal.is_empty() {
        return Err("Internal path is empty".to_string());
    }
    std::fs::create_dir_all(dest_dir).map_err(|e| e.to_string())?;
    let name = internal.rsplit('/').next().unwrap_or(&internal).to_string();

    let archive_name = archive.file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("");

    let stage = make_stage_dir(dest_dir)?;

    let extracted = if is_tar_compressed(archive_name) {
        match unpack_outer_layer(archive, password).await {
            Ok((tmp, inner_tar)) => {
                let r = run_extract_full_paths(&inner_tar, &internal, &stage, password).await;
                let _ = std::fs::remove_dir_all(&tmp);
                r
            }
            Err(e) => Err(e),
        }
    } else {
        run_extract_full_paths(archive, &internal, &stage, password).await
    };

    let result = extracted.and_then(|_| {
        let item = stage.join(&internal);
        if std::fs::symlink_metadata(&item).is_err() {
            return Err(format!("\"{}\" was not found in the archive", internal));
        }
        crate::utils::fsops::move_merge(&item, &dest_dir.join(&name))
            .map_err(|e| format!("Failed to place extracted item: {}", e))
    });

    let _ = std::fs::remove_dir_all(&stage);
    result
}
