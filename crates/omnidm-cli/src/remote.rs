// SPDX-License-Identifier: GPL-3.0-or-later

//! `omnidm get`, `omnidm list` and `omnidm queue` through the running app, over the
//! protocol (plan §8.2).

use std::path::PathBuf;

use base64::Engine;
use omnidm_ipc::frame::{read_frame_async, write_frame_async};
use omnidm_ipc::protocol::{
    AddRequest, DownloadRef, DownloadSummary, Envelope, FinishStatus, Header, Message, Progress,
};
use tokio::io::{AsyncRead, AsyncWrite, ReadHalf, WriteHalf};
use tokio_util::sync::CancellationToken;

use crate::{CliError, GetOptions, GetResult};

/// Largest reply accepted from the app (a long download list).
const MAX_REPLY: u32 = 64 * 1024 * 1024;

/// One connection to the app.
struct Conversation<S> {
    reader: ReadHalf<S>,
    writer: WriteHalf<S>,
    next: u64,
}

impl<S: AsyncRead + AsyncWrite> Conversation<S> {
    fn new(connection: S) -> Self {
        let (reader, writer) = tokio::io::split(connection);
        Self {
            reader,
            writer,
            next: 0,
        }
    }

    /// Sends `message`; returns its id.
    async fn send(&mut self, message: Message) -> Result<String, CliError> {
        self.next += 1;
        let id = format!("cli-{}", self.next);
        let bytes = serde_json::to_vec(&Envelope::new(id.clone(), message))
            .map_err(|e| CliError::App(e.to_string()))?;
        write_frame_async(&mut self.writer, &bytes)
            .await
            .map_err(|e| CliError::App(format!("lost the connection to OmniDM: {e}")))?;
        Ok(id)
    }

    /// The next message from the app.
    async fn recv(&mut self) -> Result<Envelope, CliError> {
        let frame = read_frame_async(&mut self.reader, MAX_REPLY)
            .await
            .map_err(|e| CliError::App(format!("lost the connection to OmniDM: {e}")))?
            .ok_or_else(|| CliError::App("OmniDM closed the connection".into()))?;
        serde_json::from_slice(&frame).map_err(|e| CliError::App(format!("unreadable reply: {e}")))
    }

    /// The reply to `id`, skipping anything else.
    async fn reply_to(&mut self, id: &str) -> Result<Message, CliError> {
        loop {
            let envelope = self.recv().await?;
            if envelope.re.as_deref() == Some(id) {
                return match envelope.message {
                    Message::Error(e) => Err(CliError::App(e.message)),
                    message => Ok(message),
                };
            }
        }
    }
}

/// Adds the download to the running app and follows it until it ends. Cancelling pauses
/// it in the app.
pub async fn get_via_app<S: AsyncRead + AsyncWrite>(
    connection: S,
    opts: GetOptions,
    cancel: CancellationToken,
    mut progress: impl FnMut(&Progress),
) -> Result<GetResult, CliError> {
    let dir = std::path::absolute(&opts.dir)?;
    let mut app = Conversation::new(connection);
    let add = app
        .send(Message::Add(AddRequest {
            url: opts.url,
            dir: dir.to_string_lossy().into_owned(),
            file_name: opts.output,
            connections: Some(opts.connections),
            headers: opts
                .headers
                .into_iter()
                .map(|(name, value)| Header { name, value })
                .collect(),
            cookies: opts.cookies,
            data: opts
                .data
                .map(|data| base64::engine::general_purpose::STANDARD.encode(data)),
            queue: opts.queue.clone(),
        }))
        .await?;
    let download_id = match app.reply_to(&add).await? {
        Message::Added(added) => added.download_id,
        other => return Err(unexpected(&other)),
    };
    if opts.queue.is_some() {
        // It runs when its queue does, not now.
        return Ok(GetResult::Queued(download_id));
    }

    let watch = app
        .send(Message::Watch(DownloadRef { download_id }))
        .await?;
    let mut paused = false;
    loop {
        let envelope = tokio::select! {
            _ = cancel.cancelled(), if !paused => {
                paused = true;
                app.send(Message::Pause(DownloadRef { download_id })).await?;
                continue;
            }
            envelope = app.recv() => envelope?,
        };
        if envelope.re.as_deref() != Some(watch.as_str()) {
            continue;
        }
        match envelope.message {
            Message::Progress(update) => progress(&update),
            Message::Finished(done) => {
                return match (done.status, done.path, done.error) {
                    (FinishStatus::Completed, Some(path), _) => {
                        Ok(GetResult::Completed(PathBuf::from(path)))
                    }
                    (FinishStatus::Paused, _, None) => Ok(GetResult::Paused),
                    (_, _, Some(error)) => Err(CliError::App(error)),
                    _ => Err(CliError::App("the download failed".into())),
                };
            }
            Message::Error(e) => return Err(CliError::App(e.message)),
            other => return Err(unexpected(&other)),
        }
    }
}

/// The app's downloads, oldest first.
pub async fn list_via_app<S: AsyncRead + AsyncWrite>(
    connection: S,
) -> Result<Vec<DownloadSummary>, CliError> {
    let mut app = Conversation::new(connection);
    let list = app.send(Message::List).await?;
    match app.reply_to(&list).await? {
        Message::Downloads(downloads) => Ok(downloads.items),
        other => Err(unexpected(&other)),
    }
}

/// Sends one message to the app and returns its reply (an `error` reply becomes `Err`).
pub async fn request<S: AsyncRead + AsyncWrite>(
    connection: S,
    message: Message,
) -> Result<Message, CliError> {
    let mut app = Conversation::new(connection);
    let id = app.send(message).await?;
    app.reply_to(&id).await
}

fn unexpected(message: &Message) -> CliError {
    CliError::App(format!("unexpected reply from OmniDM: {message:?}"))
}
