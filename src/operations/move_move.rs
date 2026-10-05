use std::path::Path;

/// Moves `source` to `dest` (full destination path). Works across filesystems.
pub async fn move_file(source: &Path, dest: &Path, password: Option<&str>) -> Result<(), String> {
    if let Some((archive_path, internal_path)) =
        crate::archive::browse::parse_archive_path(source)
    {
        if internal_path.is_empty() {
            return Err("Cannot move an archive root".to_string());
        }
        let dest_dir = dest.parent().unwrap_or(Path::new(".")).to_path_buf();
        crate::archive::extractor::extract_entry(
            &archive_path,
            &internal_path,
            &dest_dir,
            password,
        )
        .await?;
        crate::archive::creator::delete_entry_from_archive(
            &archive_path,
            &internal_path,
            password,
        )
        .await?;
        return Ok(());
    }

    if source == dest {
        return Ok(());
    }
    if source.is_dir() && crate::utils::fsops::is_same_or_inside(dest, source) {
        return Err(format!(
            "Cannot move \"{}\" into itself",
            source.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default()
        ));
    }
    crate::utils::fsops::move_merge(source, dest).map_err(|e| e.to_string())
}
