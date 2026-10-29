// SPDX-License-Identifier: GPL-3.0-or-later

//! HLS and DASH parsing, AES-128 decryption and the FFmpeg remux adapter.
//!
//! A stream's manifest (an HLS playlist or a DASH MPD) is read into a [`Manifest`]: the
//! qualities and audio tracks it offers (§12). Each chosen one becomes a [`Track`], a list
//! of small pieces that download like ordinary files and are then joined into one.

pub mod aes128;
pub mod dash;
pub mod hls;

use std::fmt;

pub use url::Url;

/// Most pieces one track may have: about 4 days of 2-second pieces. A manifest asking for
/// more is broken or hostile.
pub const MAX_PIECES: usize = 200_000;

/// Which format a manifest is in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StreamKind {
    Hls,
    Dash,
    /// A YouTube video, read with yt-dlp.
    YouTube,
}

/// What a stream offers.
#[derive(Debug, Clone, PartialEq)]
pub struct Manifest {
    pub kind: StreamKind,
    /// The video's own title, when the source tells it (playlists don't).
    pub title: Option<String>,
    /// Length in seconds, when the manifest says.
    pub duration: Option<f64>,
    /// The qualities to choose from, best first.
    pub qualities: Vec<Quality>,
    /// Separate sound tracks, for qualities whose sound isn't part of them.
    pub audio: Vec<AudioTrack>,
}

impl Manifest {
    /// The sound tracks `quality` can play with; none when its sound is part of it.
    pub fn audio_for(&self, quality: &Quality) -> impl Iterator<Item = &AudioTrack> {
        let group = quality.audio_group.clone();
        self.audio
            .iter()
            .filter(move |a| group.is_some() && a.group == group)
    }

    /// The sound track to use with `quality` unless the user picks another: the one the
    /// manifest marks as default, else the first.
    pub fn default_audio(&self, quality: &Quality) -> Option<&AudioTrack> {
        self.audio_for(quality)
            .find(|a| a.default)
            .or_else(|| self.audio_for(quality).next())
    }
}

/// One quality of the picture (or of the sound, for a stream without a picture).
#[derive(Debug, Clone, PartialEq)]
pub struct Quality {
    /// Bits per second; 0 when unknown.
    pub bandwidth: u64,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub frame_rate: Option<f64>,
    pub codecs: Option<String>,
    /// Which [`AudioTrack::group`] it plays with; `None` when its sound is part of it.
    pub audio_group: Option<String>,
    pub source: Source,
}

/// A sound track, such as one language.
#[derive(Debug, Clone, PartialEq)]
pub struct AudioTrack {
    pub name: String,
    pub language: Option<String>,
    pub group: Option<String>,
    /// Bits per second, when known.
    pub bandwidth: Option<u64>,
    pub codecs: Option<String>,
    pub default: bool,
    pub source: Source,
}

/// Where a quality's or sound track's pieces come from.
// A video has a handful of these, so the size of a ready track doesn't matter.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, PartialEq)]
pub enum Source {
    /// An HLS media playlist still to fetch; read it with [`hls::read_track`].
    Playlist(Url),
    /// Ready to download.
    Track(Track),
}

/// The pieces of one quality or sound track, in playing order.
#[derive(Debug, Clone, PartialEq)]
pub struct Track {
    /// Tells the track apart when its addresses change between reads (a YouTube format's
    /// number); `None` when the addresses themselves do.
    pub id: Option<String>,
    /// Request headers its pieces need, beyond the page's (yt-dlp names them).
    pub headers: Vec<(String, String)>,
    /// A header piece that goes before the others (fragmented MP4).
    pub init: Option<Piece>,
    pub pieces: Vec<Piece>,
    pub container: Container,
    /// Length in seconds, when known.
    pub duration: Option<f64>,
}

/// One piece of a track: a small file, or a part of one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Piece {
    pub url: Url,
    pub range: Option<ByteRange>,
    /// Set when the piece is locked with AES-128 (see [`aes128`]).
    pub key: Option<Key>,
}

/// Bytes `start` to `end` of a file. Both ends are included, as in HTTP `Range`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ByteRange {
    pub start: u64,
    pub end: u64,
}

impl ByteRange {
    /// How many bytes it covers.
    pub fn size(&self) -> u64 {
        self.end - self.start + 1
    }
}

/// How to unlock a piece: the address of its 16-byte AES-128 key, and its IV.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Key {
    pub url: Url,
    pub iv: [u8; 16],
}

/// The format the pieces are in, once joined.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Container {
    /// MPEG transport stream.
    Ts,
    /// (Fragmented) MP4.
    Mp4,
    WebM,
}

impl Container {
    /// The file extension for it, without a dot.
    pub fn extension(self) -> &'static str {
        match self {
            Container::Ts => "ts",
            Container::Mp4 => "mp4",
            Container::WebM => "webm",
        }
    }
}

/// Why a stream can't be downloaded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MediaError {
    /// Locked with DRM; holds what locks it (`Widevine`, `SAMPLE-AES`, …).
    Protected(String),
    /// A live broadcast: its playlist never ends.
    Live,
    /// Valid, but uses something OmniDm can't download yet; says what.
    Unsupported(String),
    /// Not a playlist, or a broken one; says what is wrong.
    Invalid(String),
}

impl fmt::Display for MediaError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MediaError::Protected(what) => write!(
                f,
                "This video is protected with DRM ({what}), so it can't be downloaded"
            ),
            MediaError::Live => {
                f.write_str("This is a live stream, which OmniDm can't download yet")
            }
            MediaError::Unsupported(what) => f.write_str(what),
            MediaError::Invalid(what) => write!(f, "The video's playlist couldn't be read: {what}"),
        }
    }
}

impl std::error::Error for MediaError {}

/// Reads a manifest of either kind, telling them apart by their content. `url` is where it
/// came from, after redirects: the pieces' addresses are relative to it.
pub fn read_manifest(text: &str, url: &Url) -> Result<Manifest, MediaError> {
    let start = text.trim_start_matches('\u{feff}').trim_start();
    if start.starts_with("#EXTM3U") {
        hls::read_manifest(start.as_bytes(), url)
    } else if start.starts_with('<') && start.contains("<MPD") {
        dash::read_manifest(start, url)
    } else {
        Err(MediaError::Invalid(
            "it isn't an HLS or DASH playlist".into(),
        ))
    }
}

/// `reference` resolved against `base`; only web addresses are allowed.
fn join(base: &Url, reference: &str) -> Result<Url, MediaError> {
    let url = base
        .join(reference.trim())
        .map_err(|_| MediaError::Invalid(format!("a bad address: {reference}")))?;
    match url.scheme() {
        "http" | "https" => Ok(url),
        scheme => Err(MediaError::Unsupported(format!(
            "The video uses {scheme}: addresses, which OmniDm can't download"
        ))),
    }
}

/// Orders qualities best first: by picture height, then by bandwidth.
fn sort_best_first(qualities: &mut [Quality]) {
    qualities.sort_by(|a, b| {
        (b.height.unwrap_or(0), b.bandwidth).cmp(&(a.height.unwrap_or(0), a.bandwidth))
    });
}
