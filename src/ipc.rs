use std::{io, path::PathBuf};

use futures::SinkExt;
use log::{debug, error, info};
use tokio::net::{UnixListener, UnixStream};
use tokio::runtime::Handle;
use tokio::sync::watch::{Receiver, Sender};
use tokio_stream::StreamExt;
use tokio_stream::wrappers::WatchStream;
use tokio_util::codec::{FramedRead, FramedWrite, LinesCodec};

use crate::events::{Event, EventManager};
use crate::model::playable::Playable;
use crate::spotify::PlayerEvent;

/// The longest command line, in bytes, read from an IPC client.
const MAX_LINE_LENGTH: usize = 64 * 1024;

pub struct IpcSocket {
    tx: Sender<Status>,
    path: PathBuf,
}

#[derive(Clone, Debug, Serialize)]
struct Status {
    mode: PlayerEvent,
    playable: Option<Playable>,
}

impl Drop for IpcSocket {
    fn drop(&mut self) {
        self.try_remove_socket();
    }
}

impl IpcSocket {
    pub fn new(handle: &Handle, path: PathBuf, ev: EventManager) -> io::Result<Self> {
        let path = if path.exists() && Self::is_open_socket(&path) {
            let mut new_path = path;
            new_path.set_file_name(format!("ttunes.{}.sock", std::process::id()));
            new_path
        } else if path.exists() && !Self::is_open_socket(&path) {
            std::fs::remove_file(&path)?;
            path
        } else {
            path
        };

        info!("Creating IPC domain socket at {path:?}");

        let status = Status {
            mode: PlayerEvent::Stopped,
            playable: None,
        };

        let (tx, rx) = tokio::sync::watch::channel(status);
        let listener_path = path.clone();
        handle.spawn(async move {
            match UnixListener::bind(&listener_path) {
                Ok(listener) => Self::worker(listener, ev, rx).await,
                Err(e) => error!("Could not create IPC domain socket at {listener_path:?}: {e}"),
            }
        });

        Ok(Self { tx, path })
    }

    fn is_open_socket(path: &PathBuf) -> bool {
        std::os::unix::net::UnixStream::connect(path).is_ok()
    }

    pub fn publish(&self, event: &PlayerEvent, playable: Option<Playable>) {
        let status = Status {
            mode: event.clone(),
            playable,
        };
        // Kept even without a receiver (there's none if the socket couldn't be made).
        self.tx.send_replace(status);
    }

    async fn worker(listener: UnixListener, ev: EventManager, tx: Receiver<Status>) {
        loop {
            match listener.accept().await {
                Ok((stream, sockaddr)) => {
                    debug!("Connection from {sockaddr:?}");
                    tokio::spawn(Self::stream_handler(
                        stream,
                        ev.clone(),
                        WatchStream::new(tx.clone()),
                    ));
                }
                Err(e) => error!("Error accepting connection: {e}"),
            }
        }
    }

    async fn stream_handler(
        mut stream: UnixStream,
        ev: EventManager,
        mut rx: WatchStream<Status>,
    ) -> Result<(), String> {
        let (reader, writer) = stream.split();
        // Bounded, so a client that never sends a newline can't use up memory.
        let mut framed_reader =
            FramedRead::new(reader, LinesCodec::new_with_max_length(MAX_LINE_LENGTH));
        let mut framed_writer = FramedWrite::new(writer, LinesCodec::new());

        loop {
            tokio::select! {
                line = framed_reader.next() => {
                    match line {
                        Some(Ok(line)) => {
                            debug!("Received line: \"{line}\"");
                            ev.send(Event::IpcInput(line));
                        }
                        Some(Err(e)) => error!("Error reading line: {e}"),
                        None => {
                            debug!("Closing IPC connection");
                            return Ok(())
                        }
                    }
                }
                Some(status) = rx.next() => {
                    debug!("IPC Status update: {status:?}");
                    let status_str = serde_json::to_string(&status).map_err(|e| e.to_string())?;
                    framed_writer.send(status_str).await.map_err(|e| e.to_string())?;
                }
                else => {
                    error!("All streams are closed");
                    return Ok(())
                }
            }
        }
    }

    /// Try to remove the IPC socket if there is one for this instance of `ttunes`. Don't do
    /// anything if the socket has already been removed for some reason.
    fn try_remove_socket(&mut self) {
        if std::fs::remove_file(&self.path).is_ok() {
            info!("removed socket at {:?}", self.path);
        } else {
            info!("socket already removed");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_socket_that_cant_be_made_leaves_player_events_be() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        // In a directory that isn't there, so binding fails.
        let path = std::env::temp_dir()
            .join(format!("ttunes-ipc-test-{}", std::process::id()))
            .join("missing")
            .join("ttunes.sock");
        let socket = IpcSocket::new(runtime.handle(), path, EventManager::new_for_test()).unwrap();
        // Let the listener try, and give up.
        runtime.block_on(async { tokio::time::sleep(std::time::Duration::from_millis(50)).await });
        socket.publish(&PlayerEvent::Stopped, None);
        socket.publish(&PlayerEvent::FinishedTrack, None);
    }
}
