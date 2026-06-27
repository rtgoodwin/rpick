//! Core video file types and processing

use std::time::SystemTime;
use serde::{Deserialize, Serialize};

/// State of a video file during a session
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum VideoFileState {
    Active,   // Still in original location
    Good,     // Moved to Good/
    Fine,     // Moved to Fine/
    Trashed,  // Sent to trash
}

/// A single video file with metadata
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VideoFile {
    pub path: String,
    pub duration: f64,
    pub hash: [u8; 16],
    pub mod_time: Option<SystemTime>,
    pub state: VideoFileState,
    pub orig_path: Option<String>,
    pub dest_path: Option<String>,
    pub ocr_results: Vec<OcrResult>,
}

/// OCR result for a single detected text region
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OcrResult {
    pub text: String,
    pub confidence: f64,
    pub bounding_box: Rect,
    pub engine: String,
}

/// Simple rectangle
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

/// Cached video info (persisted to disk)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CachedVideoInfo {
    pub path: String,
    pub duration: f64,
    pub hash: [u8; 16],
    pub mod_time_secs: u64,  // seconds since epoch
    pub size: i64,
    pub ocr_results: Vec<OcrResult>,
}

impl From<&VideoFile> for CachedVideoInfo {
    fn from(vf: &VideoFile) -> Self {
        let mod_time_secs = vf
            .mod_time
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_secs())
            .unwrap_or(0);

        let size = std::fs::metadata(&vf.path)
            .map(|m| m.len() as i64)
            .unwrap_or(0);

        Self {
            path: vf.path.clone(),
            duration: vf.duration,
            hash: vf.hash,
            mod_time_secs,
            size,
            ocr_results: vf.ocr_results.clone(),
        }
    }
}

impl VideoFile {
    /// Create a new VideoFile from a path
    pub fn new(path: String) -> Self {
        Self {
            path,
            duration: 0.0,
            hash: [0u8; 16],
            mod_time: None,
            state: VideoFileState::Active,
            orig_path: None,
            dest_path: None,
            ocr_results: Vec::new(),
        }
    }
}

/// Supported video file extensions
pub const VIDEO_EXTENSIONS: &[&str] = &[
    "mp4", "avi", "mkv", "mov", "wmv", "flv", "webm", "m4v",
    "3gp", "ogv", "ts", "m2ts", "mts", "mpg", "mpeg",
    "divx", "xvid", "rm", "rmvb", "asf", "vob",
];

/// Check if a file path has a video extension
pub fn is_video_extension(path: &str) -> bool {
    let lower = path.to_lowercase();
    let ext = std::path::Path::new(&lower)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("");
    VIDEO_EXTENSIONS.contains(&ext)
}
