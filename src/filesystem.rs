//! File system operations: scanning, moving, tagging, duplicates

use crate::video::{is_video_extension, VideoFile, VIDEO_EXTENSIONS};
use std::path::Path;
use walkdir::WalkDir;

/// Scan a directory recursively for video files
pub fn scan_videos(root: &str) -> Result<Vec<VideoFile>, String> {
    let mut files = Vec::new();

    for entry in WalkDir::new(root).follow_links(true) {
        let entry = match entry {
            Ok(e) => e,
            Err(_) => continue,
        };

        if entry.file_type().is_dir() {
            continue;
        }

        let path = entry.path().to_string_lossy().to_string();

        // Skip organized folders
        let lower = path.to_lowercase();
        if lower.contains("good") || lower.contains("fine") || lower.contains("dupes") {
            continue;
        }

        if !is_video_extension(&path) {
            continue;
        }

        let mut vf = VideoFile::new(path);
        if let Ok(meta) = entry.metadata() {
            vf.mod_time = meta.modified().ok();
        }
        files.push(vf);
    }

    Ok(files)
}

/// Move a file from src to dst, creating directories if needed
pub fn move_file(src: &str, dst: &str) -> Result<(), String> {
    let parent = Path::new(dst).parent().unwrap_or(Path::new("."));
    std::fs::create_dir_all(parent)
        .map_err(|e| format!("Failed to create directory {}: {}", parent.display(), e))?;
    std::fs::rename(src, dst)
        .map_err(|e| format!("Failed to move {} -> {}: {}", src, dst, e))
}

/// Send a file to macOS Trash using the `trash` CLI
pub fn send_to_trash(path: &str) -> Result<(), String> {
    let abs = std::fs::canonicalize(path)
        .map_err(|e| format!("Failed to resolve path: {}", e))?;
    let status = std::process::Command::new("trash")
        .arg(abs.to_string_lossy().as_ref())
        .status()
        .map_err(|e| format!("Failed to run trash command: {}", e))?;

    if status.success() {
        Ok(())
    } else {
        Err(format!(
            "trash command failed with exit code {:?}",
            status.code()
        ))
    }
}

/// Move a file to a target folder with unique name generation
pub fn move_to_folder(src: &str, target_folder: &str) -> Result<String, String> {
    let parent = Path::new(target_folder);
    std::fs::create_dir_all(parent)
        .map_err(|e| format!("Failed to create directory: {}", e))?;

    let base_name = Path::new(src)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("unknown");

    let ext = Path::new(src)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("");
    let stem = Path::new(src)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("file");

    let mut dest = Path::new(target_folder).join(base_name);
    let mut counter = 1u32;
    while dest.exists() {
        let new_name = if ext.is_empty() {
            format!("{}_{}", stem, counter)
        } else {
            format!("{}_{}.{}", stem, counter, ext)
        };
        dest = Path::new(target_folder).join(&new_name);
        counter += 1;
    }

    move_file(src, dest.to_string_lossy().as_ref())?;
    Ok(dest.to_string_lossy().to_string())
}

/// macOS Finder tag operations via the `tag` CLI
pub fn set_tag(path: &str, color: &str) -> Result<(), String> {
    let status = std::process::Command::new("tag")
        .arg("--add")
        .arg(color)
        .arg(path)
        .status()
        .map_err(|e| format!("Failed to run tag command: {}", e))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!(
            "tag command failed with exit code {:?}",
            status.code()
        ))
    }
}

pub fn list_tags(path: &str) -> Result<Vec<String>, String> {
    let output = std::process::Command::new("tag")
        .arg("--list")
        .arg(path)
        .output()
        .map_err(|e| format!("Failed to run tag command: {}", e))?;
    if output.status.success() {
        let text = String::from_utf8_lossy(&output.stdout);
        Ok(text.lines().map(|l| l.trim().to_string()).collect())
    } else {
        Err(format!("tag command failed"))
    }
}

/// Open a file location in Finder
pub fn open_in_finder(path: &str) -> Result<(), String> {
    let status = std::process::Command::new("open")
        .arg("-R")
        .arg(path)
        .status()
        .map_err(|e| format!("Failed to run open command: {}", e))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("open command failed"))
    }
}

/// Duplicate detection: group files by hash
pub fn find_duplicates(video_files: &[VideoFile]) -> Vec<(String, Vec<String>)> {
    use std::collections::HashMap;
    let mut hash_map: HashMap<String, Vec<String>> = HashMap::new();

    for vf in video_files {
        let hash_hex = hex::encode(vf.hash);
        if vf.hash == [0u8; 16] {
            continue;
        }
        hash_map
            .entry(hash_hex)
            .or_insert_with(Vec::new)
            .push(vf.path.clone());
    }

    let mut result: Vec<(String, Vec<String>)> = hash_map
        .into_iter()
        .filter(|(_, paths)| paths.len() > 1)
        .collect();

    result.sort_by(|a, b| b.1.len().cmp(&a.1.len()));
    result
}

/// Fix special characters in filenames (replace with safe alternatives)
pub fn fix_special_characters(root: &str) -> Result<(usize, usize), String> {
    let replacements: Vec<(&str, &str)> = vec![
        ("[", "("),
        ("]", ")"),
        ("&", "and"),
        ("#", "_"),
        ("@", "_"),
        ("!", "_"),
        ("$", "_"),
        ("%", "_"),
        ("^", "_"),
        ("*", "_"),
        ("+", "_"),
        ("=", "_"),
        ("{", "("),
        ("}", ")"),
        ("|", "_"),
        ("\\", "_"),
        (":", "_"),
        (";", "_"),
        ("\"", "'"),
        ("<", "_"),
        (">", "_"),
        (",", "_"),
        ("?", "_"),
        ("/", "_"),
    ];

    let mut fixed = 0usize;
    let mut failed = 0usize;

    for entry in WalkDir::new(root).follow_links(true) {
        let entry = match entry {
            Ok(e) => e,
            Err(_) => continue,
        };
        if entry.file_type().is_dir() {
            continue;
        }
        let path = entry.path().to_string_lossy().to_string();
        let lower = path.to_lowercase();

        if !VIDEO_EXTENSIONS
            .iter()
            .any(|ext| lower.ends_with(ext))
        {
            continue;
        }

        if lower.contains("/good/") || lower.contains("/fine/") || lower.contains("/dupes/") {
            continue;
        }

        let filename = entry.file_name().to_string_lossy().to_string();
        let mut cleaned = filename.clone();
        for (old, new) in &replacements {
            cleaned = cleaned.replace(old, new);
        }

        while cleaned.contains("__") {
            cleaned = cleaned.replace("__", "_");
        }
        cleaned = cleaned.trim_start_matches('_').trim_end_matches('_').to_string();

        if cleaned != filename {
            let dir = entry.path().parent().unwrap_or(Path::new("."));
            let new_path = dir.join(&cleaned).to_string_lossy().to_string();

            if Path::new(&new_path).exists() {
                failed += 1;
                continue;
            }

            match std::fs::rename(&path, &new_path) {
                Ok(()) => fixed += 1,
                Err(e) => {
                    eprintln!("Failed to rename {}: {}", path, e);
                    failed += 1;
                }
            }
        }
    }

    Ok((fixed, failed))
}

/// Add duration prefix to video filenames: `[MM:SS] original_name.ext`
pub fn add_duration_prefixes(video_files: &[VideoFile]) -> Result<(usize, usize), String> {
    let mut fixed = 0usize;
    let mut failed = 0usize;

    for vf in video_files {
        if vf.duration <= 0.0 {
            continue;
        }

        let path = Path::new(&vf.path);
        let dir = path.parent().unwrap_or(Path::new("."));
        let filename = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");

        if filename.len() > 8 && filename.as_bytes()[0] == b'[' {
            continue;
        }

        let total_secs = vf.duration as u64;
        let mins = total_secs / 60;
        let secs = total_secs % 60;
        let prefix = if ext.is_empty() {
            format!("[{:02}:{:02}] {}", mins, secs, filename)
        } else {
            format!("[{:02}:{:02}] {}", mins, secs, filename)
        };

        let new_path = dir.join(&prefix).to_string_lossy().to_string();

        if Path::new(&new_path).exists() {
            failed += 1;
            continue;
        }

        match std::fs::rename(&vf.path, &new_path) {
            Ok(()) => fixed += 1,
            Err(e) => {
                eprintln!("Failed to rename {}: {}", vf.path, e);
                failed += 1;
            }
        }
    }

    Ok((fixed, failed))
}

/// Fix incorrect duration prefixes in filenames
pub fn fix_duration_prefixes(video_files: &[VideoFile]) -> Result<(usize, usize), String> {
    let mut fixed = 0usize;
    let mut failed = 0usize;

    for vf in video_files {
        if vf.duration <= 0.0 {
            continue;
        }

        let path = Path::new(&vf.path);
        let dir = path.parent().unwrap_or(Path::new("."));
        let filename = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");

        if !filename.starts_with('[') {
            continue;
        }

        let end_bracket = filename.find(']').unwrap_or(0);
        if end_bracket == 0 {
            continue;
        }

        let prefix_part = &filename[1..end_bracket];
        let colon = prefix_part.find(':');
        if colon.is_none() {
            continue;
        }
        let colon = colon.unwrap();

        let old_mins: u64 = prefix_part[..colon].parse().unwrap_or(0);
        let old_secs: u64 = prefix_part[colon + 1..].parse().unwrap_or(0);
        let old_total = old_mins * 60 + old_secs;

        let actual_total = vf.duration as u64;
        let diff = if old_total > actual_total {
            old_total - actual_total
        } else {
            actual_total - old_total
        };

        if diff <= 1 {
            continue;
        }

        let new_mins = actual_total / 60;
        let new_secs = actual_total % 60;
        let rest = &filename[end_bracket + 1..].trim_start();

        let new_filename = if ext.is_empty() {
            format!("[{:02}:{:02}] {}", new_mins, new_secs, rest)
        } else {
            format!("[{:02}:{:02}] {}", new_mins, new_secs, rest)
        };

        let new_path = dir.join(&new_filename).to_string_lossy().to_string();

        if Path::new(&new_path).exists() {
            failed += 1;
            continue;
        }

        match std::fs::rename(&vf.path, &new_path) {
            Ok(()) => fixed += 1,
            Err(e) => {
                eprintln!("Failed to rename {}: {}", vf.path, e);
                failed += 1;
            }
        }
    }

    Ok((fixed, failed))
}
