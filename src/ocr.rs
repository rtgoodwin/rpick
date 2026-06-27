//! OCR text detection — macOS Vision framework via FFI
//!
//! On macOS we compile a small Objective-C helper (src/vision_helper.m) that
//! uses `VNRecognizeTextRequest`. On other platforms we fall back to tesseract.

use std::ffi::CStr;
use std::io::BufWriter;

use crate::video::{OcrResult, Rect};

// ── macOS Vision FFI ──────────────────────────────────────────────

#[cfg(target_os = "macos")]
mod ffi {
    unsafe extern "C" {
        pub fn vision_perform_ocr(data: *const u8, len: usize) -> i32;
        pub fn vision_result_count() -> i32;
        pub fn vision_result_text(idx: i32) -> *const std::ffi::c_char;
        pub fn vision_result_confidence(idx: i32) -> f64;
        pub fn vision_result_rect(idx: i32) -> *const f64;
    }
}

#[cfg(target_os = "macos")]
fn run_vision_ocr(jpeg_bytes: &[u8]) -> Result<Vec<OcrResult>, String> {
    let count = unsafe { ffi::vision_perform_ocr(jpeg_bytes.as_ptr(), jpeg_bytes.len()) };
    if count < 0 {
        return Err(format!("Vision OCR failed with code {}", count));
    }

    let n = count as i32;
    let mut results = Vec::with_capacity(n as usize);

    for i in 0..n {
        let text_ptr = unsafe { ffi::vision_result_text(i) };
        let confidence = unsafe { ffi::vision_result_confidence(i) };
        let rect_ptr = unsafe { ffi::vision_result_rect(i) };

        let text = if text_ptr.is_null() {
            String::new()
        } else {
            unsafe { CStr::from_ptr(text_ptr) }
                .to_string_lossy()
                .to_string()
        };

        let (x, y, w, h) = if rect_ptr.is_null() {
            (0, 0, 0, 0)
        } else {
            let coords = unsafe { std::slice::from_raw_parts(rect_ptr, 4) };
            (
                coords[0] as i32,
                coords[1] as i32,
                coords[2] as i32,
                coords[3] as i32,
            )
        };

        results.push(OcrResult {
            text,
            confidence,
            bounding_box: Rect { x, y, width: w, height: h },
            engine: "Apple Vision".to_string(),
        });
    }

    Ok(results)
}

// ── Platform-specific entry point ─────────────────────────────────

#[cfg(target_os = "macos")]
pub fn extract_text_from_image(image_path: &str) -> Result<Vec<OcrResult>, String> {
    // Load image, encode as JPEG bytes, pass to Vision FFI
    let img = image::open(image_path)
        .map_err(|e| format!("Failed to open image: {}", e))?
        .to_rgb8();

    let mut buf = Vec::new();
    {
        let mut writer = BufWriter::new(&mut buf);
        image::codecs::jpeg::JpegEncoder::new(&mut writer)
            .encode(&img, img.width(), img.height(), image::ExtendedColorType::Rgb8)
            .map_err(|e| format!("JPEG encode failed: {}", e))?;
    }

    run_vision_ocr(&buf)
}

#[cfg(not(target_os = "macos"))]
pub fn extract_text_from_image(image_path: &str) -> Result<Vec<OcrResult>, String> {
    // Fallback: tesseract CLI
    let output = std::process::Command::new("tesseract")
        .arg(image_path)
        .arg("stdout")
        .arg("--psm")
        .arg("6")
        .output()
        .map_err(|e| format!("Failed to run tesseract: {}", e))?;

    if output.status.success() {
        let text = String::from_utf8_lossy(&output.stdout);
        let cleaned: String = text
            .chars()
            .filter(|c| *c == '\n' || *c == ' ' || c.is_alphanumeric() || c.is_ascii_punctuation())
            .collect();
        if cleaned.trim().is_empty() {
            return Ok(Vec::new());
        }
        Ok(vec![OcrResult {
            text: cleaned.trim().to_string(),
            confidence: 0.8,
            bounding_box: Rect { x: 0, y: 0, width: 0, height: 0 },
            engine: "tesseract".to_string(),
        }])
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        Err(format!("Tesseract failed: {}", stderr.trim()))
    }
}

// ── Search ────────────────────────────────────────────────────────

/// Search OCR results for a query string (case-insensitive)
pub fn search_ocr_results(query: &str, results: &[OcrResult]) -> Vec<OcrResult> {
    let query_lower = query.to_lowercase();
    results
        .iter()
        .filter(|r| r.text.to_lowercase().contains(&query_lower))
        .cloned()
        .collect()
}
