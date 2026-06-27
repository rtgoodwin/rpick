//! Application configuration types

/// Runtime configuration for the rpick application
#[derive(Debug, Clone)]
pub struct Config {
    /// Working directory (defaults to cwd)
    pub working_directory: String,
    /// Minimum duration filter in seconds
    pub min_duration_secs: f64,
    /// Cache file path
    pub cache_path: String,
    /// Deleted hashes file path
    pub deleted_hashes_path: String,
}

impl Default for Config {
    fn default() -> Self {
        let home = dirs_data_dir();
        Self {
            working_directory: std::env::current_dir()
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_else(|_| ".".to_string()),
            min_duration_secs: 0.0,
            cache_path: home.join(".rpick_video_cache.json").to_string_lossy().to_string(),
            deleted_hashes_path: home.join(".rpick_deleted_hashes").to_string_lossy().to_string(),
        }
    }
}

/// Get the XDG data directory (or ~/.local/share on Linux, ~/Library on macOS)
fn dirs_data_dir() -> PathBuf {
    // On macOS, store in ~/Library (like gopick uses ~/.gopick_*)
    std::env::var("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("/tmp"))
}

use std::path::PathBuf;

impl Config {
    /// Create a new config with a given minimum duration filter
    pub fn new(min_duration_minutes: f64) -> Self {
        let mut cfg = Self::default();
        cfg.min_duration_secs = min_duration_minutes * 60.0;
        cfg
    }
}
