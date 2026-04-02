use anyhow::Result;
use std::path::Path;
use tracing::info;

/// Calculate total size of all MP4 files in a directory (non-recursive).
pub fn calculate_folder_size(dir: &str) -> u64 {
    let path = Path::new(dir);
    if !path.exists() {
        return 0;
    }

    std::fs::read_dir(path)
        .map(|entries| {
            entries
                .filter_map(|e| e.ok())
                .filter(|e| {
                    e.path()
                        .extension()
                        .and_then(|ext| ext.to_str())
                        .map(|ext| ext.eq_ignore_ascii_case("mp4"))
                        .unwrap_or(false)
                })
                .filter_map(|e| e.metadata().ok())
                .map(|m| m.len())
                .sum()
        })
        .unwrap_or(0)
}

/// Delete a video file and its parent directory entry if the directory is empty.
pub fn delete_video_file(path: &str) -> Result<()> {
    let p = Path::new(path);
    if p.exists() {
        std::fs::remove_file(p)?;
        info!("Deleted recording: {path}");
    }
    Ok(())
}

/// Returns true if storage is over the configured limit.
pub fn is_over_limit(output_dir: &str, max_gb: u32) -> bool {
    let current_bytes = calculate_folder_size(output_dir);
    let limit_bytes = (max_gb as u64) * 1024 * 1024 * 1024;
    current_bytes > limit_bytes
}

/// Returns current storage usage as (used_gb: f64, total_gb: f64).
pub fn storage_usage(output_dir: &str, max_gb: u32) -> (f64, f64) {
    let used_bytes = calculate_folder_size(output_dir);
    let used_gb = used_bytes as f64 / (1024.0 * 1024.0 * 1024.0);
    (used_gb, max_gb as f64)
}
