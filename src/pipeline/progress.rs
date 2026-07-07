use crate::validate::check::Report;

/// Messages from the pipeline thread to the GUI.
pub enum Progress {
    Stage(String),
    TileDone { done: usize, total: usize, name: String },
    Warn(String),
    Cancelled { done: usize, total: usize },
    Error(String),
    Finished { tiles: usize, skipped: usize, failed: usize, secs: f64, report: Report },
}
