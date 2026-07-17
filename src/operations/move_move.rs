use std::path::Path;

pub async fn move_file(source: &Path, dest: &Path) -> Result<(), String> {
    if let Some((archive_path, internal_path)) =
        crate::archive::browse::parse_archive_path(source)
    {
        if internal_path.is_empty() {
            return Err("Cannot move an archive root".to_string());
        }
        let dest_dir = if internal_path.ends_with('/') {
            dest.to_path_buf()
        } else {
            dest.parent().unwrap_or(Path::new(".")).to_path_buf()
        };
        return crate::archive::extractor::extract_entry(
            &archive_path,
            &internal_path,
            &dest_dir,
            None,
        )
        .await;
    }

    std::fs::rename(source, dest).map_err(|e| e.to_string())
}
