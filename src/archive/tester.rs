use std::path::Path;

pub async fn test_archive(archive: &Path) -> Result<String, String> {
    let output = tokio::process::Command::new("7z")
        .arg("t")
        .arg(archive)
        .output()
        .await
        .map_err(|e| format!("Failed to run 7z: {}", e))?;

    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    } else {
        Err(String::from_utf8_lossy(&output.stderr).to_string())
    }
}
