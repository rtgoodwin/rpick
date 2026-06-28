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

pub fn run_normal(min_dur: f64) {
    println!("rpick interactive mode — video file manager");
    println!();
    println!("Keyboard shortcuts:");
    println!("  g  Mark as Good (move to ../Good)");
    println!("  f  Mark as Fine (move to ../Fine)");
    println!("  d  Move to trash");
    println!("  u/z Undo last action");
    println!("  n  Next video");
    println!("  q  Quit");
    println!("  Space  Play/Pause");
    println!("  ←/→   Seek -30s/+30s");
    println!("  ↑/↓   Next/Previous");
    println!("  t  OCR text detection");
    println!("  w  Add Purple tag");
    println!("  o  Open in Finder");
    println!("  ?  Help overlay");
    println!();

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

    // TODO: actual video playback with OpenCV
    println!("\nVideo list:");
    for vf in &filtered {
        let total_secs = vf.duration as u64;
        let mins = total_secs / 60;
        let secs = total_secs % 60;
        println!("  [{:02}:{:02}] {}", mins, secs, vf.path);
    }

    println!("\nInteractive playback requires OpenCV integration (see TODO).");
    println!("Non-interactive modes (scan, dupes, fix-special, etc.) work now.");
}
