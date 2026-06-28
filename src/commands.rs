//! Command handlers for each rpick mode

use crate::cache::{DeletedHashes, VideoFileCache};
use crate::config::Config;
use crate::filesystem;
use crate::ocr;
use crate::video::{VideoFile, VideoFileState};
use indicatif::{ProgressBar, ProgressStyle};

/// ── Helper ───────────────────────────────────────────────────────

fn make_config(min_dur_minutes: f64) -> Config {
    Config::new(min_dur_minutes)
}

fn load_cache() -> VideoFileCache {
    let cfg = Config::default();
    match VideoFileCache::load(&cfg.cache_path) {
        Ok(c) => {
            println!("Loaded {} cached entries", c.len());
            c
        }
        Err(e) => {
            println!("Cache load: {} (starting fresh)", e);
            VideoFileCache::new()
        }
    }
}

fn save_cache(cache: &VideoFileCache) {
    let cfg = Config::default();
    match cache.save(&cfg.cache_path) {
        Ok(()) => println!("Saved {} cached entries", cache.len()),
        Err(e) => eprintln!("Warning: cache save failed: {}", e),
    }
}

fn load_deleted_hashes() -> DeletedHashes {
    let cfg = Config::default();
    match DeletedHashes::load(&cfg.deleted_hashes_path) {
        Ok(dh) => {
            println!("Loaded {} deleted hashes", dh.len());
            dh
        }
        Err(e) => {
            eprintln!("Warning: {}", e);
            DeletedHashes::new()
        }
    }
}

fn save_deleted_hashes(dh: &DeletedHashes) {
    let cfg = Config::default();
    if let Err(e) = dh.save(&cfg.deleted_hashes_path) {
        eprintln!("Warning: {}", e);
    }
}

/// ── Scan ─────────────────────────────────────────────────────────

pub fn run_scan(with_ocr: bool, min_dur: f64) {
    let cfg = make_config(min_dur);
    let mut cache = load_cache();

    println!("Scanning for video files...");
    let video_files = match filesystem::scan_videos(".") {
        Ok(f) => f,
        Err(e) => {
            eprintln!("Error scanning: {}", e);
            return;
        }
    };

    if video_files.is_empty() {
        println!("No video files found.");
        return;
    }

    println!("Found {} video files", video_files.len());

    // Process each file — probe duration + hash + optional OCR
    let pb = ProgressBar::new(video_files.len() as u64);
    pb.set_style(
        ProgressStyle::default_bar()
            .template("{spinner} {pos}/{len} files [{elapsed}] {msg}")
            .unwrap(),
    );

    let mut processed = Vec::new();

    for vf in &video_files {
        pb.set_message(vf.path.clone());
        pb.inc(1);

        // Decide whether cache can be used:
        // - If file is stale → must re-process
        // - If OCR was requested but cache has no OCR results → must re-process
        let use_cache = match cache.get(&vf.path) {
            Some(cached) => {
                if VideoFileCache::is_stale(&vf.path, cached) {
                    false
                } else if with_ocr && cached.ocr_results.is_empty() {
                    false
                } else {
                    true
                }
            }
            None => false,
        };

        if use_cache {
            let cached = cache.get(&vf.path).unwrap();
            processed.push(VideoFile {
                path: vf.path.clone(),
                duration: cached.duration,
                hash: cached.hash,
                mod_time: vf.mod_time,
                state: VideoFileState::Active,
                orig_path: None,
                dest_path: None,
                ocr_results: cached.ocr_results.clone(),
            });
            continue;
        }

        // Cache miss or re-process needed — compute metadata
        let mut processed_vf = VideoFile::new(vf.path.clone());
        processed_vf.mod_time = vf.mod_time;

        // Probe duration via ffprobe
        if let Some(dur) = probe_duration(&vf.path) {
            processed_vf.duration = dur;
        }

        // OCR: extract a frame via ffmpeg, then run OCR on it
        if with_ocr {
            let ocr_results = extract_ocr_from_video(&vf.path);
            match ocr_results {
                Ok(results) => {
                    if !results.is_empty() {
                        println!("  OCR -> {}:", vf.path);
                        for r in &results {
                            println!("    \"{}\" (conf {:.1})", r.text, r.confidence);
                        }
                    } else {
                        println!("  OCR -> {}: (no text found)", vf.path);
                    }
                    processed_vf.ocr_results = results;
                }
                Err(e) => eprintln!("  OCR warning: {}", e),
            }
        }

        // Add/replace in cache
        cache.set(vf.path.clone(), (&processed_vf).into());
        processed.push(processed_vf);
    }

    pb.finish_and_clear();

    // Filter by minimum duration
    let filtered: Vec<VideoFile> = if cfg.min_duration_secs > 0.0 {
        processed
            .into_iter()
            .filter(|vf| vf.duration >= cfg.min_duration_secs)
            .collect()
    } else {
        processed
    };

    println!(
        "Scan complete. {} videos ({} after duration filter ≥{:.0}s)",
        video_files.len(),
        filtered.len(),
        cfg.min_duration_secs,
    );

    save_cache(&cache);
}

/// Probe video duration using ffprobe
fn probe_duration(path: &str) -> Option<f64> {
    let output = std::process::Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-show_entries",
            "format=duration",
            "-of",
            "default=noprint_wrappers=1:nokey=1",
            path,
        ])
        .output()
        .ok()?;

    if output.status.success() {
        let s = String::from_utf8_lossy(&output.stdout);
        s.trim().parse::<f64>().ok()
    } else {
        None
    }
}

/// Extract a frame from a video using ffmpeg and run OCR on it
fn extract_ocr_from_video(path: &str) -> Result<Vec<crate::video::OcrResult>, String> {
    // Create a temp file for the extracted frame
    let tmp_dir = std::env::temp_dir();
    let frame_path = tmp_dir.join("rpick_ocr_frame.jpg");

    // Use ffmpeg to extract a frame at 1 second (or first frame if shorter)
    let output = std::process::Command::new("ffmpeg")
        .args([
            "-v",
            "error",
            "-y",                     // overwrite output
            "-i",
            path,
            "-vf",
            "select='eq(pict_type,PICT_TYPE_I)'", // try keyframe
            "-frames:v",
            "1",
            &frame_path.to_string_lossy(),
        ])
        .output()
        .map_err(|e| format!("Failed to run ffmpeg: {}", e))?;

    if !output.status.success() {
        // Fallback: try extracting first frame without keyframe selection
        let output2 = std::process::Command::new("ffmpeg")
            .args([
                "-v",
                "error",
                "-y",
                "-i",
                path,
                "-frames:v",
                "1",
                &frame_path.to_string_lossy(),
            ])
            .output()
            .map_err(|e| format!("Failed to run ffmpeg (fallback): {}", e))?;

        if !output2.status.success() {
            return Err("ffmpeg could not extract a frame".to_string());
        }
    }

    // Now run OCR on the extracted frame
    let results = ocr::extract_text_from_image(&frame_path.to_string_lossy())?;

    // Clean up temp file
    std::fs::remove_file(&frame_path).ok();

    Ok(results)
}

/// ── Find OCR ─────────────────────────────────────────────────────

pub fn run_find_ocr(query: &str, trash: bool, play: bool) {
    let cache = load_cache();
    let _cfg = Config::default();

    println!("Searching OCR cache for: '{}'", query);

    let mut matched = Vec::new();

    for (path, info) in &cache.files {
        let matched_ocr = ocr::search_ocr_results(query, &info.ocr_results);
        if !matched_ocr.is_empty() {
            matched.push(path.clone());
            println!("  {} -> {}", path, matched_ocr[0].text);
        }
    }

    println!("Found {} matching files", matched.len());

    if trash && !matched.is_empty() {
        println!("Moving matched files to trash...");
        for path in &matched {
            match filesystem::send_to_trash(path) {
                Ok(()) => println!("  Trashed: {}", path),
                Err(e) => eprintln!("  Failed: {} ({})", path, e),
            }
        }
    }

    if play && !matched.is_empty() {
        println!(
            "Would play {} videos (playback not yet implemented)",
            matched.len()
        );
    }
}

/// ── Dupes ────────────────────────────────────────────────────────

pub fn run_dupes() {
    println!("Scanning for duplicate files...");

    let video_files = match filesystem::scan_videos(".") {
        Ok(f) => f,
        Err(e) => {
            eprintln!("Error scanning: {}", e);
            return;
        }
    };

    if video_files.is_empty() {
        println!("No video files found.");
        return;
    }

    let pb = ProgressBar::new(video_files.len() as u64);
    pb.set_style(
        ProgressStyle::default_bar()
            .template("{spinner} Hashing {pos}/{len} files [{elapsed}]")
            .unwrap(),
    );

    let mut with_hashes: Vec<VideoFile> = Vec::new();
    for mut vf in video_files {
        pb.inc(1);
        pb.set_message(vf.path.clone());
        if let Some(hash) = compute_file_hash(&vf.path) {
            vf.hash = hash;
            with_hashes.push(vf);
        }
    }
    pb.finish_and_clear();

    let duplicates = filesystem::find_duplicates(&with_hashes);

    if duplicates.is_empty() {
        println!("No duplicates found.");
        return;
    }

    println!("\nFound {} duplicate groups:", duplicates.len());
    for (hash_hex, paths) in &duplicates {
        println!("  Hash: {} ({} files)", &hash_hex[..8], paths.len());
        for p in paths {
            println!("    {}", p);
        }
    }

    println!("\nMoving duplicates to ../Dupes...");
    let mut moved = 0usize;
    for (_, paths) in &duplicates {
        let mut sorted_paths = paths.clone();
        sorted_paths.sort_by(|a, b| {
            let ma = std::fs::metadata(a)
                .and_then(|m| Ok(m.modified().ok()))
                .unwrap_or(None);
            let mb = std::fs::metadata(b)
                .and_then(|m| Ok(m.modified().ok()))
                .unwrap_or(None);
            ma.cmp(&mb).reverse()
        });

        for extra in sorted_paths.iter().skip(1) {
            match filesystem::move_to_folder(extra, "../Dupes") {
                Ok(dest) => {
                    println!("  Moved: {} -> {}", extra, dest);
                    moved += 1;
                }
                Err(e) => eprintln!("  Failed: {} ({})", extra, e),
            }
        }
    }

    println!("Moved {} duplicate files to ../Dupes", moved);
}

/// Simple file hash (first 16KB hashed with BLAKE3, truncated to 16 bytes)
fn compute_file_hash(path: &str) -> Option<[u8; 16]> {
    let data = std::fs::read(path).ok()?;
    let limited = data.into_iter().take(16384).collect::<Vec<_>>();
    let hash = blake3::hash(&limited);
    let mut result = [0u8; 16];
    result.copy_from_slice(&hash.as_bytes()[..16]);
    Some(result)
}

/// ── Fix Special ──────────────────────────────────────────────────

pub fn run_fix_special() {
    println!("Fixing special characters in filenames...");
    match filesystem::fix_special_characters(".") {
        Ok((fixed, failed)) => {
            println!("Fixed: {} files", fixed);
            if failed > 0 {
                println!("Failed: {} files", failed);
            }
            if fixed == 0 && failed == 0 {
                println!("No files with special characters found.");
            }
        }
        Err(e) => eprintln!("Error: {}", e),
    }
}

/// ── Add Duration ─────────────────────────────────────────────────

pub fn run_add_dur() {
    println!("Adding duration prefixes to filenames...");

    let video_files = match filesystem::scan_videos(".") {
        Ok(f) => f,
        Err(e) => {
            eprintln!("Error scanning: {}", e);
            return;
        }
    };

    if video_files.is_empty() {
        println!("No video files found.");
        return;
    }

    let mut with_dur: Vec<VideoFile> = Vec::new();
    for mut vf in video_files {
        if let Some(dur) = probe_duration(&vf.path) {
            vf.duration = dur;
        }
        with_dur.push(vf);
    }

    match filesystem::add_duration_prefixes(&with_dur) {
        Ok((fixed, failed)) => {
            println!("Added duration prefix to {} files", fixed);
            if failed > 0 {
                println!("Failed: {} files", failed);
            }
        }
        Err(e) => eprintln!("Error: {}", e),
    }
}

/// ── Fix Duration ─────────────────────────────────────────────────

pub fn run_fix_dur() {
    println!("Fixing incorrect duration prefixes in filenames...");

    let video_files = match filesystem::scan_videos(".") {
        Ok(f) => f,
        Err(e) => {
            eprintln!("Error scanning: {}", e);
            return;
        }
    };

    if video_files.is_empty() {
        println!("No video files found.");
        return;
    }

    let mut with_dur: Vec<VideoFile> = Vec::new();
    for mut vf in video_files {
        if let Some(dur) = probe_duration(&vf.path) {
            vf.duration = dur;
        }
        with_dur.push(vf);
    }

    match filesystem::fix_duration_prefixes(&with_dur) {
        Ok((fixed, failed)) => {
            println!("Fixed duration prefix on {} files", fixed);
            if failed > 0 {
                println!("Failed: {} files", failed);
            }
        }
        Err(e) => eprintln!("Error: {}", e),
    }
}

/// ── Normal (Playback) ────────────────────────────────────────────

use opencv::core::{Mat, MatTraitConst, Scalar, Point, Rect};
use opencv::highgui::{self, WINDOW_NORMAL};
use opencv::videoio::{VideoCapture, CAP_PROP_FPS, CAP_PROP_FRAME_COUNT, CAP_PROP_POS_MSEC};
use opencv::imgproc::{self, FONT_HERSHEY_SIMPLEX};
use opencv::prelude::{VideoCaptureTraitConst, VideoCaptureTrait};
use std::process::Command;

const KEY_Q: i32 = 'q' as i32;
const KEY_N: i32 = 'n' as i32;
const KEY_A: i32 = 'a' as i32;
const KEY_G: i32 = 'g' as i32;
const KEY_F: i32 = 'f' as i32;
const KEY_D: i32 = 'd' as i32;
const KEY_O: i32 = 'o' as i32;
const KEY_T: i32 = 't' as i32;
const KEY_W: i32 = 'w' as i32;
const KEY_U: i32 = 'u' as i32;
const KEY_Z: i32 = 'z' as i32;
const KEY_QMARK: i32 = '?' as i32;
const KEY_ESCAPE: i32 = 27;
const KEY_SPACE: i32 = 32;
const KEY_LEFT1: i32 = 81;
const KEY_LEFT2: i32 = 2424832;
const KEY_RIGHT1: i32 = 83;
const KEY_RIGHT2: i32 = 2555904;
const KEY_UP1: i32 = 82;
const KEY_UP2: i32 = 2490368;
const KEY_DOWN1: i32 = 84;
const KEY_DOWN2: i32 = 2621440;

pub fn run_normal(min_dur: f64) {
    println!("rpick interactive mode — video file manager");

    let video_files = match filesystem::scan_videos(".") {
        Ok(f) => f,
        Err(e) => {
            eprintln!("Error scanning: {}", e);
            return;
        }
    };

    if video_files.is_empty() {
        println!("No video files found.");
        return;
    }

    println!("Found {} video files", video_files.len());

    let pb = ProgressBar::new(video_files.len() as u64);
    pb.set_style(
        ProgressStyle::default_bar()
            .template("{spinner} Probing {pos}/{len} videos [{elapsed}]")
            .unwrap(),
    );

    let mut with_dur: Vec<VideoFile> = Vec::new();
    for mut vf in video_files {
        pb.inc(1);
        pb.set_message(vf.path.clone());
        if let Some(dur) = probe_duration(&vf.path) {
            vf.duration = dur;
        }
        with_dur.push(vf);
    }
    pb.finish_and_clear();

    let cfg = make_config(min_dur);
    let filtered: Vec<VideoFile> = if cfg.min_duration_secs > 0.0 {
        with_dur
            .into_iter()
            .filter(|vf| vf.duration >= cfg.min_duration_secs)
            .collect()
    } else {
        with_dur
    };

    println!(
        "Ready: {} videos{}",
        filtered.len(),
        if cfg.min_duration_secs > 0.0 {
            format!(" (≥{:.0}s)", cfg.min_duration_secs)
        } else {
            String::new()
        }
    );

    // Load deleted hashes
    let mut deleted_hashes = load_deleted_hashes();

    // Create window
    let window_name = "rpick";
    if let Err(e) = highgui::named_window(window_name, WINDOW_NORMAL) {
        eprintln!("Failed to create window: {}", e);
        return;
    }

    let mut i = 0usize;
    let mut paused = false;

    while i < filtered.len() {
        let vf = &filtered[i];
        println!("Playing ({}/{}) {}", i + 1, filtered.len(), vf.path);

        // Open video file
        let mut cap = match VideoCapture::from_file_def(vf.path.as_str()) {
            Ok(c) => c,
            Err(e) => {
                eprintln!("  Error opening: {}", e);
                i += 1;
                continue;
            }
        };

        let fps = cap.get(CAP_PROP_FPS).unwrap_or(30.0);
        let frame_count = cap.get(CAP_PROP_FRAME_COUNT).unwrap_or(0.0);
        let duration_secs = if fps > 0.0 { frame_count / fps } else { vf.duration };

        let mut frame = Mat::default();
        let mut help_visible = false;
        let mut seek_target: Option<f64> = None;
        let mut current_pos: f64 = 0.0;

        // Main playback loop
        loop {
            // Handle pending seek
            if let Some(target) = seek_target.take() {
                let ms = (target * 1000.0) as f64;
                let _ = cap.set(CAP_PROP_POS_MSEC, ms);
            }

            if !paused {
                match cap.read(&mut frame) {
                    Ok(true) => {}
                    _ => { break; }
                }
                // Track current position from the frame
                current_pos = cap.get(CAP_PROP_POS_MSEC).unwrap_or(0.0) / 1000.0;
            }

            if frame.empty() {
                // No frame available yet
                let key = highgui::wait_key(15).unwrap_or(-1);
                if key != -1 {
                    let brk = handle_key(key, vf, &mut i, &mut paused, &mut help_visible, duration_secs, &mut seek_target, &mut current_pos, filtered.len(), &mut deleted_hashes);
                    if brk {
                        break;
                    }
                }
                continue;
            }

            // Draw overlays (pass mutable frame) with current position
            let current_mins = (current_pos / 60.0) as i32;
            let current_secs = (current_pos % 60.0) as i32;
            draw_ui_overlays(&mut frame, vf, i, filtered.len(), duration_secs, fps, current_mins, current_secs, help_visible);

            // Show frame
            let _ = highgui::imshow(window_name, &frame);

            // Wait for key (1ms delay between frames for ~1000fps loop)
            let key = highgui::wait_key(1).unwrap_or(-1);

            // If help is visible, any key dismisses except '?'
            if help_visible && key != KEY_QMARK && key != -1 {
                help_visible = false;
                continue;
            }

            let brk = handle_key(key, vf, &mut i, &mut paused, &mut help_visible, duration_secs, &mut seek_target, &mut current_pos, filtered.len(), &mut deleted_hashes);
            if brk {
                break;
            }
        }

        let _ = cap.release();
        i += 1;
    }

    let _ = highgui::destroy_all_windows();
    save_deleted_hashes(&deleted_hashes);
    println!("Done.");
}

/// Returns true if the caller should break out of the current video loop (next/quit)
fn handle_key(key: i32, vf: &VideoFile, i: &mut usize, paused: &mut bool, help_visible: &mut bool, duration_secs: f64, seek_target: &mut Option<f64>, current_pos: &mut f64, _total: usize, deleted_hashes: &mut DeletedHashes) -> bool {
    if key == -1 {
        return false;
    }

    match key {
        KEY_Q | KEY_ESCAPE => { // q or Escape
            println!("Quit");
            return true;
        }
        KEY_N | KEY_A => { // n or a -> next
            return true;
        }
        KEY_G => {
            move_file(vf, "../Good");
            return true;
        }
        KEY_F => {
            move_file(vf, "../Fine");
            return true;
        }
        KEY_D => {
            match filesystem::send_to_trash(&vf.path) {
                Ok(_) => {
                    println!("  Trashed: {}", vf.path);
                    // Record hash in deleted hashes
                    if let Some(hash) = compute_file_hash(&vf.path) {
                        deleted_hashes.insert(hash);
                    }
                }
                Err(e) => eprintln!("  Trash failed: {} ({})", vf.path, e),
            }
            return true;
        }
        KEY_O => {
            println!("  Opening in Finder: {}", vf.path);
            let _ = Command::new("open").arg("-R").arg(&vf.path).output();
            return true;
        }
        KEY_T => {
            // OCR on current frame — extract frame via ffmpeg
            let ocr_results = extract_ocr_from_video(&vf.path);
            match ocr_results {
                Ok(results) => {
                    if results.is_empty() {
                        println!("  OCR: (no text found)");
                    } else {
                        println!("  OCR -> {}:", vf.path);
                        for r in &results {
                            println!("    \"{}\" (conf {:.1})", r.text, r.confidence);
                        }
                    }
                }
                Err(e) => eprintln!("  OCR warning: {}", e),
            }
            return false;
        }
        KEY_W => {
            println!("  Adding Purple tag to: {}", vf.path);
            let _ = filesystem::set_tag(&vf.path, "Purple");
            return false;
        }
        KEY_U | KEY_Z => {
            println!("  Undo: not implemented");
            return false;
        }
        KEY_QMARK => {
            *help_visible = !*help_visible;
            return false;
        }
        KEY_SPACE => { // Space -> play/pause
            *paused = !*paused;
            if *paused {
                println!("  Paused");
            } else {
                println!("  Playing");
            }
            return false;
        }
        KEY_LEFT1 | KEY_LEFT2 => { // Left arrow -> seek -30s
            let new_pos = (*current_pos - 30.0).max(0.0);
            *seek_target = Some(new_pos);
            *current_pos = new_pos;
            return false;
        }
        KEY_RIGHT1 | KEY_RIGHT2 => { // Right arrow -> seek +30s
            let new_pos = (*current_pos + 30.0).min(duration_secs);
            *seek_target = Some(new_pos);
            *current_pos = new_pos;
            return false;
        }
        KEY_UP1 | KEY_UP2 => { // Up arrow -> previous video
            if *i > 0 {
                *i -= 1;
            }
            return true;
        }
        KEY_DOWN1 | KEY_DOWN2 => { // Down arrow -> next video
            return true;
        }
        _ => {
            // unknown key, ignore
            return false;
        }
    }
}

fn move_file(vf: &VideoFile, dest: &str) {
    match filesystem::move_to_folder(&vf.path, dest) {
        Ok(dest_path) => println!("  Moved: {} -> {}", vf.path, dest_path),
        Err(e) => eprintln!("  Move failed: {} ({})", vf.path, e),
    }
}

fn draw_ui_overlays(frame: &mut Mat, _vf: &VideoFile, index: usize, total: usize, _duration_secs: f64, _fps: f64, current_mins: i32, current_secs: i32, help_visible: bool) {
    let width = frame.cols();
    let height = frame.rows();
    if width == 0 || height == 0 {
        return;
    }

    let white = Scalar::new(255.0, 255.0, 255.0, 0.0);
    let _yellow = Scalar::new(0.0, 255.0, 255.0, 0.0);

    // Time display at top-left
    let time_text = format!("Time: {:02}:{:02}", current_mins, current_secs);
    let _ = imgproc::put_text_def(
        frame,
        &time_text,
        Point::new(10, 30),
        FONT_HERSHEY_SIMPLEX,
        0.6,
        white,
    );

    // File progress at bottom
    let progress_text = format!("File {}/{}", index + 1, total);
    let _ = imgproc::put_text_def(
        frame,
        &progress_text,
        Point::new(10, height - 10),
        FONT_HERSHEY_SIMPLEX,
        0.5,
        white,
    );

    // Progress bar at bottom
    let bar_y = height - 25;
    let bar_width = width - 20;
    let _ = imgproc::rectangle_def(
        frame,
        Rect::new(10, bar_y, bar_width, 5),
        Scalar::new(40.0, 40.0, 40.0, 0.0),
    );

    if help_visible {
        draw_help_overlay(frame, width, height);
    }
}

fn draw_help_overlay(frame: &mut Mat, width: i32, height: i32) {
    let yellow = Scalar::new(0.0, 255.0, 255.0, 0.0);
    let white = Scalar::new(255.0, 255.0, 255.0, 0.0);

    // Semi-transparent overlay
    let _ = imgproc::rectangle_def(
        frame,
        Rect::new(0, 0, width, height),
        Scalar::new(0.0, 0.0, 0.0, 0.0),
    );

    let shortcuts = [
        ("g", "Mark as Good"),
        ("f", "Mark as Fine"),
        ("d", "Delete (trash)"),
        ("t", "OCR text detection"),
        ("n", "Next video"),
        ("u/z", "Undo last action"),
        ("o", "Open in Finder"),
        ("Space", "Play/Pause"),
        ("←/→", "Seek -30s/+30s"),
        ("↑/↓", "Prev/Next"),
        ("w", "Add Purple tag"),
        ("?", "Help overlay"),
        ("q/Esc", "Quit"),
    ];

    let _ = imgproc::put_text_def(
        frame,
        "KEYBOARD SHORTCUTS",
        Point::new(width / 2 - 100, 40),
        FONT_HERSHEY_SIMPLEX,
        0.8,
        yellow,
    );

    for (j, (key, desc)) in shortcuts.iter().enumerate() {
        let y = 80 + (j as i32) * 25;
        let _ = imgproc::put_text_def(
            frame,
            key,
            Point::new(30, y),
            FONT_HERSHEY_SIMPLEX,
            0.5,
            yellow,
        );
        let _ = imgproc::put_text_def(
            frame,
            desc,
            Point::new(100, y),
            FONT_HERSHEY_SIMPLEX,
            0.5,
            white,
        );
    }
}
