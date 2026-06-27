//! rpick — Video file manager and organizer
//!
//! Rust port of gopick (Go video sorting/flagging utility).
//! Provides CLI commands for scanning, OCR, duplicates, and file management.

use clap::{Parser, Subcommand};

use rpick::commands;

#[derive(Parser)]
#[command(name = "rpick")]
#[command(about = "Video file manager and organizer")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Normal interactive mode (video file manager with playback)
    #[command(name = "normal")]
    Normal {
        /// Minimum video duration in minutes (default: no filter)
        #[arg(long, default_value = "0")]
        min_dur: f64,
    },

    /// Scan video files (with optional OCR)
    #[command(name = "scan")]
    Scan {
        /// Perform OCR text detection on each file
        #[arg(long)]
        ocr: bool,

        /// Minimum video duration in minutes
        #[arg(long, default_value = "0")]
        min_dur: f64,
    },

    /// Find files matching OCR text in cache
    #[command(name = "find-ocr")]
    FindOcr {
        /// Query string to search for
        #[arg(long)]
        query: String,

        /// Trash matching files
        #[arg(long)]
        trash: bool,

        /// Play matching files
        #[arg(long)]
        play: bool,
    },

    /// Find and move duplicate files
    #[command(name = "dupes")]
    Dupes,

    /// Fix special characters in filenames
    #[command(name = "fix-special")]
    FixSpecial,

    /// Add duration prefix to filenames
    #[command(name = "add-dur")]
    AddDur,

    /// Fix incorrect duration prefixes in filenames
    #[command(name = "fix-dur")]
    FixDur,
}

fn main() {
    let cli = Cli::parse();

    match &cli.command {
        Commands::Normal { min_dur } => {
            commands::run_normal(*min_dur);
        }
        Commands::Scan { ocr, min_dur } => {
            commands::run_scan(*ocr, *min_dur);
        }
        Commands::FindOcr { query, trash, play } => {
            commands::run_find_ocr(query, *trash, *play);
        }
        Commands::Dupes => {
            commands::run_dupes();
        }
        Commands::FixSpecial => {
            commands::run_fix_special();
        }
        Commands::AddDur => {
            commands::run_add_dur();
        }
        Commands::FixDur => {
            commands::run_fix_dur();
        }
    }
}
