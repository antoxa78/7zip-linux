use std::path::Path;

/// Copies `source` to `dest` (the full destination path, including the item's name).
/// `source` may be a virtual path inside an archive; `password` is that archive's password.
pub async fn copy_file(source: &Path, dest: &Path, password: Option<&str>) -> Result<(), String> {
    if let Some((archive_path, internal_path)) =
        crate::archive::browse::parse_archive_path(source)
    {
        if internal_path.is_empty() {
            return Err("Cannot copy an archive root".to_string());
        }
        // extract_entry places the entry (file or whole folder) at dest_dir/<name>.
        let dest_dir = dest.parent().unwrap_or(Path::new(".")).to_path_buf();
        return crate::archive::extractor::extract_entry(
            &archive_path,
            &internal_path,
            &dest_dir,
            password,
        )
        .await;
    }

    if source.is_dir() && crate::utils::fsops::is_same_or_inside(dest, source) {
        return Err(format!(
            "Cannot copy \"{}\" into itself",
            source.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default()
        ));
    }
    if source == dest {
        return Err("Source and destination are the same file".to_string());
    }
    crate::utils::fsops::copy_recursive(source, dest).map_err(|e| e.to_string())
}
