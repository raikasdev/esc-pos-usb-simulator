//! Broadcasts parsed print events to any connected browser tabs over a plain
//! WebSocket, so the animation frontend can render them live.

use std::{
    net::{TcpListener, TcpStream},
    sync::{mpsc, Arc, Mutex},
    thread,
    time::Duration,
};

use tungstenite::{accept, Message};

use crate::protocol::Event;

type EventBatch = Vec<Event>;
type Subscribers = Arc<Mutex<Vec<mpsc::Sender<EventBatch>>>>;

#[derive(Clone)]
pub struct Broadcaster {
    subscribers: Subscribers,
}

impl Broadcaster {
    pub fn new() -> Self {
        Self { subscribers: Arc::new(Mutex::new(Vec::new())) }
    }

    pub fn publish(&self, events: EventBatch) {
        let mut subs = self.subscribers.lock().unwrap();
        subs.retain(|tx| tx.send(events.clone()).is_ok());
    }

    fn subscribe(&self) -> mpsc::Receiver<EventBatch> {
        let (tx, rx) = mpsc::channel();
        self.subscribers.lock().unwrap().push(tx);
        rx
    }
}

impl Default for Broadcaster {
    fn default() -> Self {
        Self::new()
    }
}

pub fn serve(addr: &str, broadcaster: Broadcaster) -> std::io::Result<()> {
    let listener = TcpListener::bind(addr)?;
    log::info!("Animation WebSocket server listening on ws://{addr}");

    for stream in listener.incoming() {
        let Ok(stream) = stream else { continue };
        let broadcaster = broadcaster.clone();
        thread::spawn(move || {
            if let Err(err) = handle_connection(stream, broadcaster) {
                log::debug!("animation client connection ended: {err}");
            }
        });
    }

    Ok(())
}

fn handle_connection(stream: TcpStream, broadcaster: Broadcaster) -> anyhow::Result<()> {
    let mut ws = accept(stream).map_err(|e| anyhow::anyhow!("handshake failed: {e}"))?;
    ws.get_mut().set_read_timeout(Some(Duration::from_millis(5)))?;

    let rx = broadcaster.subscribe();
    log::info!("animation client connected");

    loop {
        match rx.recv_timeout(Duration::from_millis(50)) {
            Ok(events) => {
                let json = serde_json::to_string(&events).expect("events always serialize");
                ws.send(Message::Text(json.into()))?;

                for queued_events in rx.try_iter() {
                    let json = serde_json::to_string(&queued_events).expect("events always serialize");
                    ws.send(Message::Text(json.into()))?;
                }

                continue;
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }

        match ws.read() {
            Ok(Message::Close(_)) => break,
            Ok(_) => {}
            Err(tungstenite::Error::Io(ref e))
                if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => {}
            Err(_) => break,
        }
    }

    log::info!("animation client disconnected");
    Ok(())
}
