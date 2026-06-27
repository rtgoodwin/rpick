//! Integration tests for rpick
//!
//! Tests cover: scanning, caching, OCR search, file operations,
//! duplicate detection, duration prefix fixing.

use std::fs;
use std::path::Path;
use std::io::Write;
use tempfile::TempDir;

/// ── Helpers ──────────────────────────────────────────────────────

fn create_video_file(dir: &Path, name: &str) -> String {
    let path = dir.join(name);
    let mut f = fs::File::create(&path).unwrap();
    // Write some binary data to simulate a video file
    f.write_all(b"\x00\x00\x00\x00\x00\x00\x00\x00").unwrap();
    f.write_all(&[0u8; 1024]).unwrap();
    path.to_string_lossy().to_string()
}

fn create_dir(dir: &Path, name: &str) {
    fs::create_dir(dir.join(name)).unwrap();
}

/// ── Scanning Tests ───────────────────────────────────────────────

#[test]
fn test_scan_videos_finds_files() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();

    create_video_file(dir, "test.mp4");
    create_video_file(dir, "test.mov");
    create_video_file(dir, "test.txt"); // not a video

    let files = rpick::filesystem::scan_videos(dir.to_string_lossy().as_ref()).unwrap();
    assert_eq!(files.len(), 2, "should find 2 video files");
}

#[test]
fn test_scan_videos_skips_organized_folders() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();

    create_dir(dir, "Good");
    create_dir(dir, "Fine");
    create_video_file(dir, "test.mp4");
    create_video_file(&dir.join("Good"), "good.mp4");

    let files = rpick::filesystem::scan_videos(dir.to_string_lossy().as_ref()).unwrap();
    assert_eq!(files.len(), 1, "should skip Good/Fine/Dupes folders");
}

/// ── Cache Tests ──────────────────────────────────────────────────

#[test]
fn test_cache_save_and_load() {
    use rpick::cache::VideoFileCache;
    use rpick::video::CachedVideoInfo;

    let tmp = TempDir::new().unwrap();
    let path = tmp.path().join("cache.json");
    let path_str = path.to_string_lossy().to_string();

    let mut cache = VideoFileCache::new();
    cache.set(
        "/path/to/video.mp4".to_string(),
        CachedVideoInfo {
            path: "/path/to/video.mp4".to_string(),
            duration: 123.45,
            hash: [0x01; 16],
            size: 1024,
            mod_time_secs: 1000,
            ocr_results: Vec::new(),
        },
    );

    cache.save(&path_str).unwrap();

    let loaded = VideoFileCache::load(&path_str).unwrap();
    assert_eq!(loaded.len(), 1, "should have 1 entry");
    let info = loaded.get("/path/to/video.mp4").unwrap();
    assert!((info.duration - 123.45).abs() < 0.01);
}

#[test]
fn test_cache_stale_detection() {
    use rpick::cache::VideoFileCache;
    use rpick::video::CachedVideoInfo;

    let tmp = TempDir::new().unwrap();
    let video_path = tmp.path().join("video.mp4");
    let video_path_str = video_path.to_string_lossy().to_string();

    // Create a file
    fs::write(&video_path, b"data").unwrap();
    let meta = fs::metadata(&video_path).unwrap();
    let size = meta.len() as i64;
    let mtime = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs())
        .unwrap_or(0);

    let info = CachedVideoInfo {
        path: video_path_str.clone(),
        duration: 10.0,
        hash: [0x02; 16],
        size,
        mod_time_secs: mtime,
        ocr_results: Vec::new(),
    };

    assert!(!VideoFileCache::is_stale(&video_path_str, &info));

    // Modify the file
    fs::write(&video_path, b"new data").unwrap();
    assert!(VideoFileCache::is_stale(&video_path_str, &info));
}

/// ── OCR Search Tests ─────────────────────────────────────────────

#[test]
fn test_search_ocr_results() {
    use rpick::ocr::search_ocr_results;
    use rpick::video::{OcrResult, Rect};

    let results = vec![
        OcrResult {
            text: "Hello World".to_string(),
            confidence: 0.95,
            bounding_box: Rect { x: 0, y: 0, width: 100, height: 20 },
            engine: "Apple Vision".to_string(),
        },
        OcrResult {
            text: "Goodbye".to_string(),
            confidence: 0.8,
            bounding_box: Rect { x: 0, y: 0, width: 50, height: 10 },
            engine: "Apple Vision".to_string(),
        },
    ];

    let matches = search_ocr_results("hello", &results);
    assert_eq!(matches.len(), 1, "case-insensitive match");
    assert_eq!(matches[0].text, "Hello World");

    let matches = search_ocr_results("xyz", &results);
    assert_eq!(matches.len(), 0);
}

/// ── File Operation Tests ─────────────────────────────────────────

#[test]
fn test_move_file() {
    use rpick::filesystem::move_file;

    let tmp = TempDir::new().unwrap();
    let src = tmp.path().join("src.txt");
    let dst = tmp.path().join("sub/dst.txt");

    fs::write(&src, b"data").unwrap();
    move_file(
        src.to_string_lossy().as_ref(),
        dst.to_string_lossy().as_ref(),
    )
    .unwrap();

    assert!(dst.exists(), "file should be moved");
    assert!(!src.exists(), "original should be gone");
}

#[test]
fn test_move_to_folder_with_rename() {
    use rpick::filesystem::move_to_folder;

    let tmp = TempDir::new().unwrap();
    let src = tmp.path().join("test.mp4");
    let target = tmp.path().join("target");

    fs::write(&src, b"data").unwrap();
    fs::create_dir(&target).unwrap();

    // Move first file
    let dest = move_to_folder(
        src.to_string_lossy().as_ref(),
        target.to_string_lossy().as_ref(),
    )
    .unwrap();
    assert!(Path::new(&dest).exists());

    // Move second file with same name (should rename)
    let src2 = tmp.path().join("test.mp4");
    fs::write(&src2, b"data2").unwrap();
    let dest2 = move_to_folder(
        src2.to_string_lossy().as_ref(),
        target.to_string_lossy().as_ref(),
    )
    .unwrap();
    assert!(Path::new(&dest2).exists());
    assert!(dest2.contains("_1"), "should rename to avoid collision");
}

/// ── Duplicate Detection Tests ────────────────────────────────────

#[test]
fn test_find_duplicates() {
    use rpick::filesystem::find_duplicates;
    use rpick::video::VideoFile;

    let vf1 = VideoFile {
        path: "/path/a.mp4".to_string(),
        duration: 10.0,
        hash: [0xAA; 16],
        mod_time: None,
        state: rpick::video::VideoFileState::Active,
        orig_path: None,
        dest_path: None,
        ocr_results: Vec::new(),
    };

    let vf2 = VideoFile {
        path: "/path/b.mp4".to_string(),
        duration: 10.0,
        hash: [0xAA; 16],
        mod_time: None,
        state: rpick::video::VideoFileState::Active,
        orig_path: None,
        dest_path: None,
        ocr_results: Vec::new(),
    };

    let vf3 = VideoFile {
        path: "/path/c.mp4".to_string(),
        duration: 10.0,
        hash: [0xBB; 16],
        mod_time: None,
        state: rpick::video::VideoFileState::Active,
        orig_path: None,
        dest_path: None,
        ocr_results: Vec::new(),
    };

    let dupes = find_duplicates(&[vf1, vf2, vf3]);
    assert_eq!(dupes.len(), 1, "one duplicate group");
    assert_eq!(dupes[0].1.len(), 2, "two files in group");
}

#[test]
fn test_find_duplicates_no_matches() {
    use rpick::filesystem::find_duplicates;
    use rpick::video::VideoFile;

    let vf1 = VideoFile {
        path: "/path/a.mp4".to_string(),
        duration: 10.0,
        hash: [0xAA; 16],
        mod_time: None,
        state: rpick::video::VideoFileState::Active,
        orig_path: None,
        dest_path: None,
        ocr_results: Vec::new(),
    };

    let vf2 = VideoFile {
        path: "/path/b.mp4".to_string(),
        duration: 10.0,
        hash: [0xBB; 16],
        mod_time: None,
        state: rpick::video::VideoFileState::Active,
        orig_path: None,
        dest_path: None,
        ocr_results: Vec::new(),
    };

    let dupes = find_duplicates(&[vf1, vf2]);
    assert_eq!(dupes.len(), 0, "no duplicates");
}

/// ── Special Characters Tests ─────────────────────────────────────

#[test]
fn test_fix_special_characters() {
    use rpick::filesystem::fix_special_characters;

    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();

    // Create a file with special characters
    let bad_name = dir.join("test[1].mp4");
    fs::write(&bad_name, b"data").unwrap();

    let (fixed, failed) = fix_special_characters(dir.to_string_lossy().as_ref()).unwrap();
    assert_eq!(fixed, 1, "should fix 1 file");
    assert_eq!(failed, 0, "should have 0 failures");

    // Check new name exists
    assert!(dir.join("test(1).mp4").exists(), "should rename to clean name");
    assert!(!bad_name.exists(), "original should be gone");
}

#[test]
fn test_fix_special_characters_none_to_fix() {
    use rpick::filesystem::fix_special_characters;

    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();

    create_video_file(dir, "clean.mp4");
    create_video_file(dir, "normal.mov");

    let (fixed, failed) = fix_special_characters(dir.to_string_lossy().as_ref()).unwrap();
    assert_eq!(fixed, 0);
    assert_eq!(failed, 0);
}

/// ── Duration Prefix Tests ────────────────────────────────────────

#[test]
fn test_add_duration_prefixes() {
    use rpick::filesystem::add_duration_prefixes;
    use rpick::video::VideoFile;

    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();

    let path = create_video_file(dir, "test.mp4");
    let vf = VideoFile {
        path: path.clone(),
        duration: 123.0,
        hash: [0; 16],
        mod_time: None,
        state: rpick::video::VideoFileState::Active,
        orig_path: None,
        dest_path: None,
        ocr_results: Vec::new(),
    };

    let (fixed, failed) = add_duration_prefixes(&[vf]).unwrap();
    assert_eq!(fixed, 1, "should add prefix to 1 file");
    assert_eq!(failed, 0);

    // Check new name has duration prefix
    let expected_prefix = "[02:03]";
    let entries: Vec<String> = fs::read_dir(dir)
        .unwrap()
        .filter_map(|e| e.ok().map(|e| e.file_name().to_string_lossy().to_string()))
        .collect();
    let has_prefix = entries.iter().any(|n| n.starts_with(expected_prefix));
    assert!(has_prefix, "file should be renamed with duration prefix");
}

#[test]
fn test_fix_duration_prefixes() {
    use rpick::filesystem::fix_duration_prefixes;
    use rpick::video::VideoFile;

    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();

    // Create a file with wrong duration prefix
    let wrong_path = dir.join("[01:00] test.mp4");
    fs::write(&wrong_path, b"data").unwrap();
    let wrong_path_str = wrong_path.to_string_lossy().to_string();

    let vf = VideoFile {
        path: wrong_path_str,
        duration: 123.0, // actual duration is 123s = 2m3s
        hash: [0; 16],
        mod_time: None,
        state: rpick::video::VideoFileState::Active,
        orig_path: None,
        dest_path: None,
        ocr_results: Vec::new(),
    };

    let (fixed, failed) = fix_duration_prefixes(&[vf]).unwrap();
    assert_eq!(fixed, 1, "should fix 1 file");
    assert_eq!(failed, 0);

    let entries: Vec<String> = fs::read_dir(dir)
        .unwrap()
        .filter_map(|e| e.ok().map(|e| e.file_name().to_string_lossy().to_string()))
        .collect();
    let has_fixed = entries.iter().any(|n| n.starts_with("[02:03]"));
    assert!(has_fixed, "file should be renamed with correct duration prefix");
}

#[test]
fn test_fix_duration_prefixes_no_change() {
    use rpick::filesystem::fix_duration_prefixes;
    use rpick::video::VideoFile;

    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();

    // Create a file with correct prefix
    let correct_path = dir.join("[02:03] test.mp4");
    fs::write(&correct_path, b"data").unwrap();
    let correct_path_str = correct_path.to_string_lossy().to_string();

    let vf = VideoFile {
        path: correct_path_str,
        duration: 123.0, // 2m3s matches prefix
        hash: [0; 16],
        mod_time: None,
        state: rpick::video::VideoFileState::Active,
        orig_path: None,
        dest_path: None,
        ocr_results: Vec::new(),
    };

    let (fixed, failed) = fix_duration_prefixes(&[vf]).unwrap();
    assert_eq!(fixed, 0, "should not fix correct prefix");
    assert_eq!(failed, 0);
}

/// ── Video Extension Tests ────────────────────────────────────────

#[test]
fn test_is_video_extension() {
    use rpick::video::is_video_extension;

    assert!(is_video_extension("test.mp4"));
    assert!(is_video_extension("test.MOV"));
    assert!(is_video_extension("test.AVI"));
    assert!(is_video_extension("test.mkv"));
    assert!(is_video_extension("test.webm"));
    assert!(is_video_extension("test.flv"));

    assert!(!is_video_extension("test.txt"));
    assert!(!is_video_extension("test.jpg"));
    assert!(!is_video_extension("test.pdf"));
    assert!(!is_video_extension("test.mp4.txt"));
}

/// ── Config Tests ─────────────────────────────────────────────────

#[test]
fn test_config_defaults() {
    use rpick::config::Config;

    let cfg = Config::default();
    assert!(!cfg.working_directory.is_empty());
    assert!(cfg.cache_path.contains(".rpick_video_cache.json"));
    assert!(cfg.deleted_hashes_path.contains(".rpick_deleted_hashes"));
}

#[test]
fn test_config_with_min_duration() {
    use rpick::config::Config;

    let cfg = Config::new(5.0);
    assert!((cfg.min_duration_secs - 300.0).abs() < 0.01);
}
