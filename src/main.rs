//! rpick — Video file manager and organizer (Rust port of gopick)
//!
//! CLI flags match gopick's original flag-based interface.

use clap::Parser;

use rpick::commands;

#[derive(Parser)]
#[command(name = "rpick")]
#[command(about = "Video file manager and organizer")]
struct Cli {
    /// Scan and populate cache only, no UI
    #[arg(short = 's', long)]
    scan: bool,

    /// Scan with OCR text detection (slower)
    #[arg(long)]
    scan_ocr: bool,

    /// Search cached OCR results for text
    #[arg(long)]
    find_ocr: Option<String>,

    /// Move -find-ocr matches to trash (requires -find-ocr)
    #[arg(long)]
    trash: bool,

    /// Play the videos found with -find-ocr
    #[arg(long)]
    play: bool,

    /// Move all but one of each dupe to ../Dupes
    #[arg(long)]
    dupes: bool,

    /// Fix special characters in filenames (standalone mode)
    #[arg(long)]
    fix_special: bool,

    /// Add duration to video filenames
    #[arg(long)]
    add_dur: bool,

    /// Fix incorrect duration prefixes in filenames
    #[arg(long)]
    fix_dur: bool,

    /// Minimum duration in minutes to include videos
    #[arg(short = 'd', long = "min-dur", default_value = "0")]
    min_dur: f64,
}

fn main() {
    let cli = Cli::parse();

    // Validate flag combinations
    if cli.fix_special {
        if cli.scan || cli.scan_ocr || cli.find_ocr.is_some() || cli.trash || cli.add_dur || cli.fix_dur {
            eprintln!("Error: -fix-special must be used alone (no other flags)");
            return;
        }
        commands::run_fix_special();
        return;
    }

    if cli.add_dur {
        if cli.scan || cli.scan_ocr || cli.find_ocr.is_some() || cli.trash || cli.fix_special || cli.fix_dur {
            eprintln!("Error: -add-dur must be used alone (no other flags)");
            return;
        }
        commands::run_add_dur();
        return;
    }

    if cli.fix_dur {
        if cli.scan || cli.scan_ocr || cli.find_ocr.is_some() || cli.trash || cli.fix_special || cli.add_dur {
            eprintln!("Error: -fix-dur must be used alone (no other flags)");
            return;
        }
        commands::run_fix_dur();
        return;
    }

    if cli.trash && cli.find_ocr.is_none() {
        eprintln!("Error: -trash flag can only be used with -find-ocr");
        return;
    }

    // Dispatch modes
    if cli.scan || cli.scan_ocr {
        commands::run_scan(cli.scan_ocr, cli.min_dur);
    } else if let Some(query) = cli.find_ocr {
        commands::run_find_ocr(&query, cli.trash, cli.play);
    } else if cli.dupes {
        commands::run_dupes();
    } else {
        // Default: interactive playback mode
        commands::run_normal(cli.min_dur);
    }
}
