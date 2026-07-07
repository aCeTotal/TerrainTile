use std::convert::Infallible;

use axum::extract::State;
use axum::response::sse::{Event, KeepAlive, Sse};
use serde_json::json;
use tokio_stream::wrappers::errors::BroadcastStreamRecvError;
use tokio_stream::wrappers::BroadcastStream;
use tokio_stream::{Stream, StreamExt};

use crate::server::state::SharedState;

/// GET /api/events — SSE stream. The first event is a full snapshot so a
/// client that connects mid-run (or reloads the page) sees current state;
/// after that, incremental pipeline events.
pub async fn events(
    State(state): State<SharedState>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let rx = state.events.subscribe();
    let snapshot = {
        let inner = state.inner.lock().unwrap();
        json!({ "type": "snapshot", "snapshot": inner.snapshot }).to_string()
    };
    let first = tokio_stream::once(Ok(Event::default().data(snapshot)));
    let rest = BroadcastStream::new(rx).filter_map(|msg| match msg {
        Ok(data) => Some(Ok(Event::default().data(data))),
        // Client fell behind and missed events; the next status poll or
        // tile event catches it up, so just skip.
        Err(BroadcastStreamRecvError::Lagged(_)) => None,
    });
    Sse::new(first.chain(rest)).keep_alive(KeepAlive::default())
}
