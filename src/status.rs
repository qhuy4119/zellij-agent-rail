//! Shared attention-status helpers and the rail's palette.
//!
//! Colors use the terminal's own 16-color palette (bright red / green /
//! yellow / cyan, dim, bold) instead of fixed truecolor, so the rail adapts
//! to dark and light terminal themes alike.

use zellij_tile::prelude::*;

/// Priority of a status for aggregation and sorting (higher wins).
pub fn status_rank(status: &AttentionStatus) -> u8 {
    match status {
        AttentionStatus::NeedsYou => 7,
        AttentionStatus::Failed => 6,
        AttentionStatus::Done => 5,
        AttentionStatus::Working => 4,
        AttentionStatus::NewOutput => 3,
        AttentionStatus::Quiet => 2,
        AttentionStatus::Idle => 0,
    }
}

/// Glyph and 16-color code for an attention status.
pub fn status_glyph(status: &AttentionStatus) -> (&'static str, &'static str) {
    match status {
        AttentionStatus::NeedsYou => ("◉", C_NEEDS),
        AttentionStatus::Failed => ("✗", C_FAILED),
        AttentionStatus::Done => ("✓", C_DONE),
        AttentionStatus::Working => ("▸", C_WORK),
        AttentionStatus::Quiet => ("◌", DIM),
        AttentionStatus::NewOutput => ("✚", C_NEW_OUT),
        AttentionStatus::Idle => ("·", DIM),
    }
}

/// Short human label for a status.
pub fn status_label(status: &AttentionStatus) -> &'static str {
    match status {
        AttentionStatus::NeedsYou => "needs you",
        AttentionStatus::Failed => "failed",
        AttentionStatus::Done => "done",
        AttentionStatus::Working => "working",
        AttentionStatus::Quiet => "quiet",
        AttentionStatus::NewOutput => "new output",
        AttentionStatus::Idle => "idle",
    }
}

// --- palette: terminal-default colors, theme-adaptive ---

pub const DIM: &str = "\u{1b}[2m";
pub const BOLD: &str = "\u{1b}[1m";
pub const RESET: &str = "\u{1b}[0m";

/// Bright red — needs you, failed.
pub const C_NEEDS: &str = "\u{1b}[91m";
pub const C_FAILED: &str = "\u{1b}[91m";
/// Bright green — done.
pub const C_DONE: &str = "\u{1b}[92m";
/// Bright yellow — working.
pub const C_WORK: &str = "\u{1b}[93m";
/// Bright cyan — new output, foreign sessions.
pub const C_NEW_OUT: &str = "\u{1b}[96m";
/// Magenta — the rail's title accent.
pub const C_ACCENT: &str = "\u{1b}[35m";
