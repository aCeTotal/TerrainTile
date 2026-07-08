use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use serde_json::json;

use crate::pipeline::config::PipelineConfig;
use crate::pipeline::progress::Progress;
use crate::pipeline::runner;
use crate::server::state::{AppState, SharedState};

/// Start the pipeline on a background thread and bridge its progress
/// messages into the shared snapshot + SSE broadcast. Returns an error if a
/// run is already active.
pub fn start(state: &SharedState, cfg: PipelineConfig, inputs: Vec<PathBuf>) -> Result<(), String> {
    let cancel = Arc::new(AtomicBool::new(false));
    {
        let mut inner = state.inner.lock().unwrap();
        if inner.snapshot.running {
            return Err("en jobb kjører allerede".into());
        }
        inner.cancel = Some(cancel.clone());
        inner.output = Some(cfg.output.clone());
        inner.snapshot.running = true;
        inner.snapshot.status = "Starter…".into();
        inner.snapshot.done = 0;
        inner.snapshot.total = 0;
        inner.snapshot.log.clear();
        inner.snapshot.report = None;
        inner.snapshot.output = Some(cfg.output.display().to_string());
    }
    let _ = state.events.send(json!({ "type": "started" }).to_string());

    if let Err(e) = crate::server::project::save(&cfg, &inputs) {
        eprintln!("project.json: {e:#}");
    }

    let (tx, rx) = crossbeam_channel::unbounded();
    let cancel2 = cancel.clone();
    std::thread::spawn(move || runner::run(cfg, inputs, tx, cancel2));

    let state = state.clone();
    std::thread::spawn(move || {
        for msg in rx {
            let finished = matches!(
                msg,
                Progress::Cancelled { .. } | Progress::Error(_) | Progress::Finished { .. }
            );
            forward(&state, msg);
            if finished {
                return;
            }
        }
        // Channel closed without a terminal message: the pipeline died.
        state.publish(
            |s| {
                s.running = false;
                s.status = "Prosessering stoppet uventet".into();
                AppState::push_log(s, "Prosessering stoppet uventet".into());
            },
            json!({ "type": "error", "text": "Prosessering stoppet uventet" }),
        );
    });
    Ok(())
}

fn forward(state: &SharedState, msg: Progress) {
    match msg {
        Progress::Stage(text) => state.publish(
            |s| {
                s.status = text.clone();
                AppState::push_log(s, text.clone());
            },
            json!({ "type": "stage", "text": text }),
        ),
        Progress::TileDone { done, total, name } => state.publish(
            |s| {
                s.done = done;
                s.total = total;
                s.status = format!("Flis {done}/{total}  ({name})");
            },
            json!({ "type": "tile", "done": done, "total": total, "name": name }),
        ),
        Progress::Warn(text) => state.publish(
            |s| AppState::push_log(s, format!("⚠ {text}")),
            json!({ "type": "warn", "text": text }),
        ),
        Progress::Cancelled { done, total } => {
            let text = format!("Avbrutt ved {done}/{total} — start igjen for å fortsette");
            state.publish(
                |s| {
                    s.running = false;
                    s.status = text.clone();
                    AppState::push_log(s, text.clone());
                },
                json!({ "type": "cancelled", "text": text }),
            );
        }
        Progress::Error(text) => state.publish(
            |s| {
                s.running = false;
                s.status = format!("Feil: {text}");
                AppState::push_log(s, format!("Feil: {text}"));
            },
            json!({ "type": "error", "text": text }),
        ),
        Progress::Finished { tiles, skipped, failed, secs, report } => {
            let text = format!(
                "Ferdig: {tiles} fliser ({skipped} gjenbrukt, {failed} feilet) på {secs:.0} s — validering {}",
                if report.ok() { "OK" } else { "FEIL" }
            );
            let rep = serde_json::to_value(&report).unwrap_or_default();
            state.publish(
                |s| {
                    s.running = false;
                    s.status = text.clone();
                    s.report = Some(report);
                    AppState::push_log(s, text.clone());
                },
                json!({ "type": "finished", "text": text, "report": rep }),
            );
        }
    }
}

/// Request cancellation of the active run, if any.
pub fn cancel(state: &SharedState) -> bool {
    let inner = state.inner.lock().unwrap();
    match (&inner.cancel, inner.snapshot.running) {
        (Some(c), true) => {
            c.store(true, Ordering::Relaxed);
            true
        }
        _ => false,
    }
}
