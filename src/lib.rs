//! rpick — Video file manager and organizer
//!
//! Rust port of gopick (Go video sorting/flagging utility).
//! Provides CLI commands for scanning, OCR, duplicates, and file management.

pub mod cache;
pub mod commands;
pub mod config;
pub mod filesystem;
pub mod ocr;
pub mod ui;
pub mod video;
