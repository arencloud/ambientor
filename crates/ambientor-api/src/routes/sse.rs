use std::convert::Infallible;
use std::sync::Arc;
use std::time::Duration;

use axum::{
    extract::{Path, State},
    response::sse::{Event, KeepAlive, Sse},
};
use futures::stream::Stream;
use tokio::sync::broadcast;
use tokio_stream::StreamExt;
use tokio_stream::wrappers::BroadcastStream;

use crate::state::AppState;

pub struct SseHub {
    tx: broadcast::Sender<String>,
}

impl SseHub {
    pub fn new() -> Self {
        let (tx, _) = broadcast::channel(256);
        Self { tx }
    }

    pub fn publish(&self, channel: &str, payload: &serde_json::Value) {
        let msg = serde_json::json!({ "channel": channel, "payload": payload }).to_string();
        let _ = self.tx.send(msg);
    }

    pub fn subscribe(&self) -> broadcast::Receiver<String> {
        self.tx.subscribe()
    }
}

fn event_stream(rx: broadcast::Receiver<String>) -> impl Stream<Item = Result<Event, Infallible>> {
    BroadcastStream::new(rx).filter_map(|msg| msg.ok().map(|data| Ok(Event::default().data(data))))
}

pub async fn subscribe(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let _ = id;
    let rx = {
        let hub = state.sse.read().await;
        hub.subscribe()
    };
    let stream = event_stream(rx);
    Sse::new(stream).keep_alive(KeepAlive::new().interval(Duration::from_secs(15)))
}
