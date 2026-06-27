//! UI utilities for terminal output (non-OpenCV mode)
//!
//! In interactive mode, gopick uses OpenCV HighGUI for video playback.
//! The Rust port currently lists files in terminal mode. This module will
//! be expanded when OpenCV bindings are integrated.

/// Print a formatted time duration
pub fn format_duration(total_secs: f64) -> String {
    let total = total_secs as u64;
    let mins = total / 60;
    let secs = total % 60;
    format!("[{:02}:{:02}]", mins, secs)
}

/// Print a progress-style message
pub fn progress_line(index: usize, total: usize, path: &str) -> String {
    format!("{:>4}/{} {}", index + 1, total, path)
}
