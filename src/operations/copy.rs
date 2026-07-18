use std::path::Path;

pub async fn copy_file(source: &Path, dest: &Path, password: Option<&str>) -> Result<(), String> {
    if let Some((archive_path, internal_path)) =
        crate::archive::browse::parse_archive_path(source)
    {
        if internal_path.is_empty() {
            return Err("Cannot copy an archive root".to_string());
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
            password,
        )
        .await;
    }

    if source.is_dir() {
        std::fs::create_dir_all(dest).map_err(|e| e.to_string())?;
        for entry in std::fs::read_dir(source).map_err(|e| e.to_string())? {
            let entry = entry.map_err(|e| e.to_string())?;
            let src = entry.path();
            let dst = dest.join(entry.file_name());
            Box::pin(copy_file(&src, &dst, password)).await?;
        }
        Ok(())
    } else {
        std::fs::copy(source, dest).map_err(|e| e.to_string())?;
        Ok(())
    }
}
