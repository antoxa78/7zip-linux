pub fn icon_for_file(name: &str, is_dir: bool) -> &'static str {
    if is_dir {
        return "folder-symbolic";
    }
    let ext = name.rsplit('.').next().unwrap_or("");
    match ext.to_lowercase().as_str() {
        // Archives
        "7z" | "zip" | "tar" | "gz" | "bz2" | "xz" | "rar" | "tgz" | "tbz2" | "txz" => "package-x-generic-symbolic",
        // Images
        "png" | "jpg" | "jpeg" | "gif" | "bmp" | "svg" | "webp" | "ico" => "image-x-generic-symbolic",
        // Video
        "mp4" | "mkv" | "avi" | "mov" | "wmv" | "flv" | "webm" => "video-x-generic-symbolic",
        // Audio
        "mp3" | "flac" | "ogg" | "wav" | "aac" | "wma" => "audio-x-generic-symbolic",
        // Documents
        "pdf" => "application-pdf-symbolic",
        "doc" | "docx" => "x-office-document-symbolic",
        "xls" | "xlsx" => "x-office-spreadsheet-symbolic",
        "ppt" | "pptx" => "x-office-presentation-symbolic",
        // Text
        "txt" | "md" | "rs" | "py" | "js" | "ts" | "c" | "cpp" | "h" | "hpp" | "java" | "go" | "toml" | "yaml" | "yml" | "json" | "xml" | "html" | "css" | "sh" | "bash" | "zsh" => "text-x-generic-symbolic",
        // Executables
        "exe" | "msi" | "deb" | "rpm" | "appimage" | "flatpak" => "application-x-executable-symbolic",
        // Default
        _ => "text-x-generic-symbolic",
    }
}
