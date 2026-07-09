use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};

use tokio::sync::Mutex as AsyncMutex;

use serde::Serialize;
use tokio::sync::broadcast;

use crate::edit::cover::Cover;
use crate::gen::heightfield::HeightSource;
use crate::gen::world::WorldParams;
use crate::tile::classdef::ClassDef;
use crate::tile::grid::{TileGrid, TileId};
use crate::validate::check::Report;

/// Everything the edit endpoints need for the open project: the composite
/// height source (generator + overlays), painted class coverage and the
/// rebuild worker's inbox.
pub struct EditCtx {
    pub world: WorldParams,
    pub classes: Vec<ClassDef>,
    pub grid: TileGrid,
    pub src: Arc<HeightSource>,
    pub cover: Arc<Cover>,
    pub dirty_tx: crossbeam_channel::Sender<BTreeSet<TileId>>,
}

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
    /// Output dir served under /data/.
    pub output: Option<PathBuf>,
    /// Lazily created on the first edit request; dropped when the output
    /// changes (its rebuild worker exits when the channel closes).
    pub edit: Option<Arc<EditCtx>>,
}

pub struct AppState {
    pub inner: Mutex<Inner>,
    /// Pre-serialized JSON events for SSE subscribers.
    pub events: broadcast::Sender<String>,
    /// Serializes far.bin cache builds (concurrent viewers must not build twice).
    pub far_lock: AsyncMutex<()>,
}

pub type SharedState = Arc<AppState>;

impl AppState {
    pub fn new() -> SharedState {
        let (events, _) = broadcast::channel(1024);
        Arc::new(AppState {
            inner: Mutex::new(Inner {
                snapshot: Snapshot {
                    status: "Nytt eller åpne prosjekt for å starte".into(),
                    ..Default::default()
                },
                cancel: None,
                output: None,
                edit: None,
            }),
            events,
            far_lock: AsyncMutex::new(()),
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
