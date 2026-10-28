// SPDX-License-Identifier: GPL-3.0-or-later

//! Speaks the protocol (plan §8.2) on one connection: the extension through the native
//! messaging host, or the CLI.

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};

use omnidm_ipc::frame::{MAX_FROM_EXTENSION, read_frame_async, write_frame_async};
use omnidm_ipc::protocol::{
    BrowserCookie, CaptureManyResult, CookiesRequest, DownloadList, DownloadRef, Envelope,
    ErrorInfo, FinishStatus, Finished, HelloAck, Message, PROTOCOL_VERSION,
};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::sync::{broadcast, mpsc, oneshot};

use crate::manager::{CookieSource, Manager, Update};

/// Serves every client of the app's named pipe, each on its own task.
#[cfg(windows)]
pub async fn serve(
    manager: Manager,
    mut listener: omnidm_ipc::pipe::PipeListener,
) -> std::io::Result<()> {
    loop {
        let pipe = listener.accept().await?;
        tokio::spawn(handle_connection(manager.clone(), pipe));
    }
}

/// Serves one connection until the other side closes it.
pub async fn handle_connection<S>(manager: Manager, stream: S)
where
    S: AsyncRead + AsyncWrite + Send + 'static,
{
    let (mut reader, mut writer) = tokio::io::split(stream);
    let (out, mut outgoing) = mpsc::unbounded_channel::<Envelope>();
    // One writer, so replies, progress and cookie requests never interleave.
    let writer_task = tokio::spawn(async move {
        while let Some(envelope) = outgoing.recv().await {
            let Ok(bytes) = serde_json::to_vec(&envelope) else {
                continue;
            };
            if write_frame_async(&mut writer, &bytes).await.is_err() {
                break;
            }
        }
    });
    let browser = Arc::new(BrowserLink {
        out: out.clone(),
        pending: Mutex::new(HashMap::new()),
    });
    let as_source: Arc<dyn CookieSource> = browser.clone();
    let mut said_hello = false;

    while let Ok(Some(frame)) = read_frame_async(&mut reader, MAX_FROM_EXTENSION).await {
        let envelope: Envelope = match serde_json::from_slice(&frame) {
            Ok(envelope) => envelope,
            Err(e) => {
                let _ = out.send(error("badMessage", &e.to_string()));
                continue;
            }
        };
        let id = envelope.id.clone();
        if envelope.v != PROTOCOL_VERSION {
            let _ = out.send(error("unsupportedVersion", "use protocol version 1").reply_to(id));
            continue;
        }
        let reply = |message: Message| {
            let _ = out.send(Envelope::new(new_id(), message).reply_to(id.clone()));
        };
        match envelope.message {
            Message::Hello(_) => {
                // Only the extension says hello: it can provide fresh cookies.
                said_hello = true;
                manager.set_cookie_source(Some(as_source.clone()));
                reply(Message::HelloAck(HelloAck {
                    app_version: env!("CARGO_PKG_VERSION").into(),
                    protocol: PROTOCOL_VERSION,
                }));
            }
            Message::Ping => reply(Message::Pong),
            Message::Capture(capture) => {
                // The probe takes a while; keep reading meanwhile.
                let (manager, out) = (manager.clone(), out.clone());
                tokio::spawn(async move {
                    let result = manager.capture(capture).await;
                    let reply = Envelope::new(new_id(), Message::CaptureResult(result));
                    let _ = out.send(reply.reply_to(id));
                });
            }
            Message::CaptureMany(many) => {
                reply(Message::CaptureManyResult(CaptureManyResult {
                    results: manager.capture_many(many.items),
                }));
            }
            Message::CookiesResponse(response) => {
                if let Some(re) = &envelope.re {
                    browser.answer(re, response.cookies);
                }
            }
            Message::Add(add) => match manager.add(add) {
                Ok(download_id) => reply(Message::Added(DownloadRef { download_id })),
                Err(message) => {
                    let _ = out.send(error("invalid", &message).reply_to(id));
                }
            },
            Message::Watch(target) => {
                tokio::spawn(watch(manager.clone(), target.download_id, id, out.clone()));
            }
            Message::Pause(target) => {
                if manager.pause(target.download_id) || exists(&manager, target.download_id) {
                    reply(Message::Ack);
                } else {
                    let _ = out.send(error("notFound", "no such download").reply_to(id));
                }
            }
            Message::Resume(target) => {
                // Resuming may wait for the browser's cookies; keep reading meanwhile.
                let (manager, out) = (manager.clone(), out.clone());
                tokio::spawn(async move {
                    let message = match manager.resume(target.download_id).await {
                        Ok(()) => Envelope::new(new_id(), Message::Ack),
                        Err(e) if e == "no such download" => error("notFound", &e),
                        Err(e) => error("invalid", &e),
                    };
                    let _ = out.send(message.reply_to(id));
                });
            }
            Message::Show => {
                manager.show();
                reply(Message::Ack);
            }
            Message::List => reply(Message::Downloads(DownloadList {
                items: manager.list(),
            })),
            other => {
                let kind = serde_json::to_value(&other)
                    .ok()
                    .and_then(|v| v.get("type").and_then(|t| t.as_str()).map(String::from))
                    .unwrap_or_default();
                let message = format!("the app doesn't take {kind:?} messages");
                let _ = out.send(error("unexpected", &message).reply_to(id));
            }
        }
    }

    if said_hello {
        manager.clear_cookie_source(&as_source);
    }
    drop(out);
    drop(as_source);
    drop(browser);
    let _ = writer_task.await;
}

fn exists(manager: &Manager, id: i64) -> bool {
    manager.list().iter().any(|d| d.download_id == id)
}

/// Sends `progress` and then `finished` for one download, as replies to `re`.
async fn watch(manager: Manager, id: i64, re: String, out: mpsc::UnboundedSender<Envelope>) {
    let mut updates = manager.subscribe();
    if !manager.is_active(id) {
        // Already over (or never started): report how it ended.
        let Some(summary) = manager.list().into_iter().find(|d| d.download_id == id) else {
            let _ = out.send(error("notFound", "no such download").reply_to(re));
            return;
        };
        let status = match summary.status.as_str() {
            "completed" => FinishStatus::Completed,
            "failed" | "cancelled" => FinishStatus::Failed,
            _ => FinishStatus::Paused,
        };
        let done = Finished {
            download_id: id,
            status,
            path: None,
            error: summary.error,
        };
        let _ = out.send(Envelope::new(new_id(), Message::Finished(done)).reply_to(re));
        return;
    }
    loop {
        match updates.recv().await {
            Ok(Update::Progress(progress)) if progress.download_id == id => {
                let message = Envelope::new(new_id(), Message::Progress(progress));
                if out.send(message.reply_to(&re)).is_err() {
                    return;
                }
            }
            Ok(Update::Finished(done)) if done.download_id == id => {
                let _ = out.send(Envelope::new(new_id(), Message::Finished(done)).reply_to(re));
                return;
            }
            Ok(_) | Err(broadcast::error::RecvError::Lagged(_)) => {}
            Err(broadcast::error::RecvError::Closed) => return,
        }
    }
}

/// The browser on the other end of a connection, asked for fresh cookies.
struct BrowserLink {
    out: mpsc::UnboundedSender<Envelope>,
    /// Cookie requests waiting for an answer, by request id.
    pending: Mutex<HashMap<String, oneshot::Sender<Vec<BrowserCookie>>>>,
}

impl BrowserLink {
    fn answer(&self, re: &str, cookies: Vec<BrowserCookie>) {
        if let Some(waiting) = self.pending.lock().unwrap().remove(re) {
            let _ = waiting.send(cookies);
        }
    }
}

impl CookieSource for BrowserLink {
    fn cookies_for(
        &self,
        url: String,
    ) -> Pin<Box<dyn Future<Output = Option<Vec<BrowserCookie>>> + Send>> {
        let id = new_id();
        let (tx, rx) = oneshot::channel();
        self.pending.lock().unwrap().insert(id.clone(), tx);
        let request = Envelope::new(id, Message::CookiesRequest(CookiesRequest { url }));
        let sent = self.out.send(request);
        Box::pin(async move {
            sent.ok()?;
            rx.await.ok()
        })
    }
}

fn error(code: &str, message: &str) -> Envelope {
    Envelope::new(
        new_id(),
        Message::Error(ErrorInfo {
            code: code.into(),
            message: message.into(),
        }),
    )
}

fn new_id() -> String {
    format!("app-{:016x}", fastrand::u64(..))
}
