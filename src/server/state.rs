use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};

use tokio::sync::Mutex as AsyncMutex;

use serde::Serialize;
use tokio::sync::broadcast;

use crate::import::dataset::DatasetInfo;
use crate::validate::check::Report;

/// Everything the browser needs to render current pipeline status.
/// Sent as one snapshot on connect, kept in sync via SSE events.
#[derive(Clone, Serialize, Default)]
pub struct Snapshot {
    pub running: bool,
    pub status: String,
    pub done: usize,
    pub total: usize,
    pub log: Vec<String>,
    pub report: Option<Report>,
    /// Output dir of the current/last run, if any — the viewer reads
    /// `/data/dataset.json` when this is set and the run finished.
    pub output: Option<String>,
}

pub struct Inner {
    pub snapshot: Snapshot,
    /// Cancel flag of the active run.
    pub cancel: Option<Arc<AtomicBool>>,
    /// Last successful full scan: inputs + info, used by /api/grid.
    pub scanned: Option<(Vec<PathBuf>, DatasetInfo)>,
    /// Output dir served under /data/.
    pub output: Option<PathBuf>,
}

pub struct AppState {
    pub inner: Mutex<Inner>,
    /// Pre-serialized JSON events for SSE subscribers.
    pub events: broadcast::Sender<String>,
    /// Serializes far.bin cache builds (concurrent viewers must not build twice).
    pub far_lock: AsyncMutex<()>,
    /// True while the overview mosaic is being built in the background.
    pub overview_building: AtomicBool,
}

pub type SharedState = Arc<AppState>;

impl AppState {
    pub fn new() -> SharedState {
        let (events, _) = broadcast::channel(1024);
        Arc::new(AppState {
            inner: Mutex::new(Inner {
                snapshot: Snapshot {
                    status: "Velg høydedata for å starte".into(),
                    ..Default::default()
                },
                cancel: None,
                scanned: None,
                output: None,
            }),
            events,
            far_lock: AsyncMutex::new(()),
            overview_building: AtomicBool::new(false),
        })
    }

    /// Output dir of the current/last run, if any.
    pub fn output(&self) -> Option<PathBuf> {
        self.inner.lock().unwrap().output.clone()
    }

    /// Update the snapshot and broadcast one event to all SSE clients.
    pub fn publish(&self, update: impl FnOnce(&mut Snapshot), event: serde_json::Value) {
        {
            let mut inner = self.inner.lock().unwrap();
            update(&mut inner.snapshot);
        }
        let _ = self.events.send(event.to_string());
    }

    /// Append to the log with a hard cap — a runaway warning stream must
    /// never grow memory without bound.
    pub fn push_log(snap: &mut Snapshot, line: String) {
        if snap.log.last() == Some(&line) {
            return;
        }
        snap.log.push(line);
        if snap.log.len() > 2000 {
            snap.log.drain(..500);
        }
    }
}
