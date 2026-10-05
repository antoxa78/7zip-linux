use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::time::{sleep, Duration};

fn read_only_format(path: &Path) -> Option<&'static str> {
    let name = path.file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("")
        .to_lowercase();
    if name.ends_with(".rar") {
        Some("RAR")
    } else if name.ends_with(".arj") {
        Some("ARJ")
    } else if name.ends_with(".cab") {
        Some("CAB")
    } else if name.ends_with(".chm") {
        Some("CHM")
    } else if name.ends_with(".cpio") {
        Some("CPIO")
    } else if name.ends_with(".deb") {
        Some("DEB")
    } else if name.ends_with(".dmg") {
        Some("DMG")
    } else if name.ends_with(".iso") {
        Some("ISO")
    } else if name.ends_with(".lzh") || name.ends_with(".lha") {
        Some("LZH")
    } else if name.ends_with(".rpm") {
        Some("RPM")
    } else if name.ends_with(".squashfs") {
        Some("SquashFS")
    } else if name.ends_with(".vhd") || name.ends_with(".vmdk") {
        Some("VHD/VMDK")
    } else if name.ends_with(".xar") {
        Some("XAR")
    } else {
        None
    }
}

pub fn is_read_only_archive(path: &Path) -> bool {
    read_only_format(path).is_some()
}

/// Maps the dialog presets (Store, Fastest, Fast, Normal, Maximum, Ultra)
/// to 7-Zip's -mx levels, matching 7-Zip on Windows.
pub fn mx_for_preset(preset: u32) -> u32 {
    match preset {
        0 => 0, // Store
        1 => 1, // Fastest
        2 => 3, // Fast
        3 => 5, // Normal
        4 => 7, // Maximum
        5 => 9, // Ultra
        _ => 5,
    }
}

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
    if options.format == "tar" || options.format.starts_with("tar.") {
        return create_tar_archive(output, files, options, progress_tx, cancel, pause).await;
    }

    let mx_level = mx_for_preset(options.level);
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
    } else if stderr.trim().is_empty() {
        Err(String::from_utf8_lossy(&stdout_buf).to_string())
    } else {
        Err(stderr)
    }
}

async fn create_tar_archive(
    output: &Path,
    files: &[&Path],
    options: &ArchiveOptions,
    progress_tx: Option<async_channel::Sender<u8>>,
    cancel: Option<Arc<AtomicBool>>,
    pause: Option<Arc<AtomicBool>>,
) -> Result<String, String> {
    if options.password.is_some() || options.encrypt_file_names {
        return Err("tar archives cannot be encrypted".to_string());
    }
    if options.split_size.is_some() {
        return Err("Splitting tar archives is not supported".to_string());
    }

    let mut args = Vec::new();
    match options.format.as_str() {
        "tar" => args.push("-cf".to_string()),
        "tar.gz" => args.push("-czf".to_string()),
        "tar.bz2" => args.push("-cjf".to_string()),
        "tar.xz" => args.push("-cJf".to_string()),
        "tar.zst" => {
            args.push("--zstd".to_string());
            args.push("-cf".to_string());
        }
        _ => return Err(format!("Unsupported tar format: {}", options.format)),
    }
    args.push(output.to_string_lossy().to_string());
    // Store each item relative to its own parent folder ("-C parent name"), the way
    // 7z does, instead of the full absolute path (home/user/...).
    for file in files {
        let parent = file.parent().filter(|p| !p.as_os_str().is_empty()).unwrap_or(Path::new("."));
        let name = match file.file_name() {
            Some(n) => n.to_string_lossy().to_string(),
            None => return Err(format!("Cannot archive \"{}\"", file.display())),
        };
        args.push("-C".to_string());
        args.push(parent.to_string_lossy().to_string());
        if name.starts_with('-') {
            // Keep names like "-foo" from being parsed as tar options.
            args.push(format!("--add-file={}", name));
        } else {
            args.push(name);
        }
    }

    let mut child = tokio::process::Command::new("tar")
        .args(&args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| format!("Failed to run tar: {}", e))?;
    let stderr_task = super::proc::drain_stderr(&mut child);

    let child_id = child
        .id()
        .ok_or_else(|| "tar process exited immediately".to_string())?;
    let cancel = cancel.unwrap_or_else(|| Arc::new(AtomicBool::new(false)));
    let pause = pause.unwrap_or_else(|| Arc::new(AtomicBool::new(false)));
    let mut was_paused = false;

    loop {
        if cancel.load(Ordering::Relaxed) {
            let _ = child.kill().await;
            let _ = child.wait().await;
            return Err("Cancelled".to_string());
        }

        let is_paused = pause.load(Ordering::Relaxed);
        if is_paused && !was_paused {
            #[cfg(unix)]
            {
                let _ = tokio::process::Command::new("kill")
                    .args(["-STOP", &child_id.to_string()])
                    .status()
                    .await;
            }
            was_paused = true;
        } else if !is_paused && was_paused {
            #[cfg(unix)]
            {
                let _ = tokio::process::Command::new("kill")
                    .args(["-CONT", &child_id.to_string()])
                    .status()
                    .await;
            }
            was_paused = false;
        }

        match child.try_wait().map_err(|e| format!("tar failed: {}", e))? {
            Some(status) => {
                if status.success() {
                    if let Some(ref tx) = progress_tx {
                        let _ = tx.send(100).await;
                    }
                    return Ok(String::new());
                }
                let stderr = super::proc::collect_stderr(stderr_task).await;
                return Err(if stderr.trim().is_empty() {
                    format!("tar failed with status {}", status)
                } else {
                    stderr
                });
            }
            None => sleep(Duration::from_millis(100)).await,
        }
    }
}

pub async fn add_to_archive(
    archive: &Path,
    files: &[&Path],
    password: Option<&str>,
) -> Result<String, String> {
    if let Some(fmt) = read_only_format(archive) {
        return Err(format!(
            "{} archives are read-only and cannot be modified.\nExtract the files, make changes, and repack the archive instead.",
            fmt
        ));
    }
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

/// Creates an empty folder `dir_name` inside the archive, under `internal_prefix`
/// (the archive folder currently being browsed; empty for the archive root).
pub async fn add_directory_to_archive(
    archive: &Path,
    internal_prefix: &str,
    dir_name: &str,
    password: Option<&str>,
) -> Result<String, String> {
    if let Some(fmt) = read_only_format(archive) {
        return Err(format!(
            "{} archives are read-only and cannot be modified.\nExtract the files, make changes, and repack the archive instead.",
            fmt
        ));
    }
    if dir_name.is_empty() || dir_name.contains('/') || dir_name == "." || dir_name == ".." {
        return Err(format!("\"{}\" is not a valid folder name", dir_name));
    }
    let prefix = internal_prefix.trim_matches('/');
    let rel = if prefix.is_empty() {
        dir_name.to_string()
    } else {
        format!("{}/{}", prefix, dir_name)
    };

    let temp_base = crate::utils::unique_temp_dir("newdir")
        .map_err(|e| format!("Failed to create temp dir: {}", e))?;
    if let Err(e) = std::fs::create_dir_all(temp_base.join(&rel)) {
        let _ = std::fs::remove_dir_all(&temp_base);
        return Err(format!("Failed to create temp dir: {}", e));
    }

    let mut args = vec![
        "a".to_string(),
        "-y".to_string(),
    ];
    if let Some(pw) = password {
        args.push(format!("-p{}", pw));
    }
    args.push(archive.to_string_lossy().to_string());
    args.push(format!("{}/", rel));

    let result = tokio::process::Command::new("7z")
        .current_dir(&temp_base)
        .args(&args)
        .stdin(std::process::Stdio::null())
        .output()
        .await;
    let _ = std::fs::remove_dir_all(&temp_base);
    let result = result.map_err(|e| format!("Failed to run 7z: {}", e))?;

    if result.status.success() {
        Ok(String::from_utf8_lossy(&result.stdout).to_string())
    } else {
        let stderr = String::from_utf8_lossy(&result.stderr);
        let stdout = String::from_utf8_lossy(&result.stdout);
        let combined = format!("{} {}", stdout, stderr);
        Err(if stderr.trim().is_empty() { combined } else { stderr.to_string() })
    }
}

pub async fn add_files_into_archive_path(
    archive: &Path,
    files: &[&Path],
    internal_prefix: &str,
    password: Option<&str>,
    progress_tx: Option<async_channel::Sender<u8>>,
) -> Result<(), String> {
    if let Some(fmt) = read_only_format(archive) {
        return Err(format!(
            "{} archives are read-only and cannot be modified.\nExtract the files, make changes, and repack the archive instead.",
            fmt
        ));
    }
    let internal_prefix = internal_prefix.trim_matches('/');
    let temp_base = crate::utils::unique_temp_dir("add")
        .map_err(|e| format!("Failed to create temp dir: {}", e))?;
    let result = add_files_staged(archive, files, internal_prefix, password, progress_tx, &temp_base).await;
    let _ = std::fs::remove_dir_all(&temp_base);
    result
}

async fn add_files_staged(
    archive: &Path,
    files: &[&Path],
    internal_prefix: &str,
    password: Option<&str>,
    progress_tx: Option<async_channel::Sender<u8>>,
    temp_base: &Path,
) -> Result<(), String> {
    let target_dir = if internal_prefix.is_empty() {
        temp_base.to_path_buf()
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
        return Err("No valid files to add".to_string());
    }

    let mut args = vec!["a".to_string(), "-y".to_string(), "-bsp1".to_string()];
    if let Some(pw) = password {
        args.push(format!("-p{}", pw));
    }
    args.push(archive.to_string_lossy().to_string());
    args.extend(relative_paths);

    let mut child = tokio::process::Command::new("7z")
        .current_dir(temp_base)
        .args(&args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| format!("Failed to run 7z: {}", e))?;

    let stderr_task = super::proc::drain_stderr(&mut child);
    let mut stdout = child.stdout.take()
        .ok_or_else(|| "Failed to capture 7z stdout".to_string())?;
    use tokio::io::AsyncReadExt;
    let mut buf = vec![0u8; 4096];
    let mut stdout_buf = Vec::new();

    loop {
        let n = match stdout.read(&mut buf).await {
            Ok(n) => n,
            Err(_) => break,
        };
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

    let status = child.wait().await
        .map_err(|e| format!("7z failed: {}", e))?;
    let stderr = super::proc::collect_stderr(stderr_task).await;

    if status.success() {
        if let Some(ref tx) = progress_tx {
            let _ = tx.send(100).await;
        }
        Ok(())
    } else {
        let stdout_str = String::from_utf8_lossy(&stdout_buf);
        let combined = format!("{} {}", stdout_str, stderr);
        Err(if stderr.trim().is_empty() { combined } else { stderr })
    }
}

/// Moves an entry (file or folder) to a new full path inside the same archive.
pub async fn move_entry_in_archive(
    archive: &Path,
    old_path: &str,
    new_path: &str,
    password: Option<&str>,
) -> Result<(), String> {
    if let Some(fmt) = read_only_format(archive) {
        return Err(format!(
            "{} archives are read-only and cannot be modified.\nExtract the files, make changes, and repack the archive instead.",
            fmt
        ));
    }
    let mut args = vec!["rn".to_string(), "-y".to_string()];
    if let Some(pw) = password {
        args.push(format!("-p{}", pw));
    }
    args.push(archive.to_string_lossy().to_string());
    args.push(old_path.trim_matches('/').to_string());
    args.push(new_path.trim_matches('/').to_string());

    let output = tokio::process::Command::new("7z")
        .args(&args)
        .stdin(std::process::Stdio::null())
        .output()
        .await
        .map_err(|e| format!("Failed to run 7z: {}", e))?;
    if output.status.success() {
        Ok(())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        let combined = format!("{} {}", stdout, stderr);
        Err(if stderr.trim().is_empty() { combined } else { stderr.to_string() })
    }
}

pub async fn delete_entry_from_archive(
    archive: &Path,
    internal_path: &str,
    password: Option<&str>,
) -> Result<(), String> {
    if let Some(fmt) = read_only_format(archive) {
        return Err(format!(
            "{} archives are read-only and cannot be modified.\nExtract the files, make changes, and repack the archive instead.",
            fmt
        ));
    }
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
    if let Some(fmt) = read_only_format(archive) {
        return Err(format!(
            "{} archives are read-only and cannot be modified.\nExtract the files, make changes, and repack the archive instead.",
            fmt
        ));
    }
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
