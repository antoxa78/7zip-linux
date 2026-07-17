use std::path::Path;

pub async fn create_directory(path: &Path) -> Result<(), String> {
    std::fs::create_dir(path).map_err(|e| e.to_string())
}
