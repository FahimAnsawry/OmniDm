// SPDX-License-Identifier: GPL-3.0-or-later

//! Message framing: a 4-byte little-endian length, then that many bytes of UTF-8 JSON.
//! It is the browser's native messaging format, so the host can relay frames unchanged.

use std::fmt;
use std::io::{self, Read, Write};

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

/// Largest message the app accepts from the extension.
pub const MAX_FROM_EXTENSION: u32 = 4 * 1024 * 1024;
/// Largest message the browser accepts from a native host.
pub const MAX_TO_EXTENSION: u32 = 1024 * 1024;

#[derive(Debug)]
pub enum FrameError {
    Io(io::Error),
    /// The length field announced more than the reader allows.
    TooLarge {
        size: u64,
        limit: u32,
    },
}

impl fmt::Display for FrameError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(e) => write!(f, "{e}"),
            Self::TooLarge { size, limit } => {
                write!(f, "message of {size} bytes is over the {limit}-byte limit")
            }
        }
    }
}

impl std::error::Error for FrameError {}

impl From<io::Error> for FrameError {
    fn from(e: io::Error) -> Self {
        Self::Io(e)
    }
}

/// The length field for a message of `len` bytes.
pub fn check_length(len: usize) -> Result<u32, FrameError> {
    u32::try_from(len).map_err(|_| FrameError::TooLarge {
        size: len as u64,
        limit: u32::MAX,
    })
}

pub fn write_frame(w: &mut impl Write, message: &[u8]) -> Result<(), FrameError> {
    let len = check_length(message.len())?;
    w.write_all(&len.to_le_bytes())?;
    w.write_all(message)?;
    w.flush()?;
    Ok(())
}

/// Reads one frame of at most `max` bytes. `None` means the other side closed cleanly
/// between frames.
pub fn read_frame(r: &mut impl Read, max: u32) -> Result<Option<Vec<u8>>, FrameError> {
    let mut len = [0u8; 4];
    let first = loop {
        match r.read(&mut len) {
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            other => break other?,
        }
    };
    if first == 0 {
        return Ok(None);
    }
    r.read_exact(&mut len[first..])?;
    let len = u32::from_le_bytes(len);
    if len > max {
        return Err(FrameError::TooLarge {
            size: len.into(),
            limit: max,
        });
    }
    let mut message = vec![0u8; len as usize];
    r.read_exact(&mut message)?;
    Ok(Some(message))
}

pub async fn write_frame_async(
    w: &mut (impl AsyncWrite + Unpin),
    message: &[u8],
) -> Result<(), FrameError> {
    let len = check_length(message.len())?;
    w.write_all(&len.to_le_bytes()).await?;
    w.write_all(message).await?;
    w.flush().await?;
    Ok(())
}

/// Async [`read_frame`].
pub async fn read_frame_async(
    r: &mut (impl AsyncRead + Unpin),
    max: u32,
) -> Result<Option<Vec<u8>>, FrameError> {
    let mut len = [0u8; 4];
    let first = r.read(&mut len).await?;
    if first == 0 {
        return Ok(None);
    }
    r.read_exact(&mut len[first..]).await?;
    let len = u32::from_le_bytes(len);
    if len > max {
        return Err(FrameError::TooLarge {
            size: len.into(),
            limit: max,
        });
    }
    let mut message = vec![0u8; len as usize];
    r.read_exact(&mut message).await?;
    Ok(Some(message))
}
