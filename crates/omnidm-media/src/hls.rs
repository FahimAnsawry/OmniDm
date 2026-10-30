// SPDX-License-Identifier: GPL-3.0-or-later

//! HLS playlists (RFC 8216). A master playlist lists qualities, each in a media playlist
//! of its own; a media playlist lists the pieces of one of them.

use std::collections::HashSet;

use m3u8_rs::{
    AlternativeMediaType, KeyMethod, MasterPlaylist, MediaPlaylist, MediaPlaylistType, Playlist,
};
use url::Url;

use crate::{
    AudioTrack, ByteRange, Container, Key, MAX_PIECES, Manifest, MediaError, Piece, Quality,
    Source, StreamKind, Track, aes128, join, sort_best_first,
};

/// Reads a master or media playlist. A media playlist gives one quality, ready to download;
/// a master playlist gives qualities whose media playlists still need fetching.
pub fn read_manifest(text: &[u8], url: &Url) -> Result<Manifest, MediaError> {
    match parse(text)? {
        Playlist::MasterPlaylist(master) => from_master(master, url),
        Playlist::MediaPlaylist(media) => {
            let track = track(media, url)?;
            Ok(Manifest {
                kind: StreamKind::Hls,
                title: None,
                duration: track.duration,
                qualities: vec![Quality {
                    bandwidth: 0,
                    width: None,
                    height: None,
                    frame_rate: None,
                    codecs: None,
                    audio_group: None,
                    source: Source::Track(track),
                }],
                audio: Vec::new(),
            })
        }
    }
}

/// Reads the media playlist of one quality or sound track ([`Source::Playlist`]).
pub fn read_track(text: &[u8], url: &Url) -> Result<Track, MediaError> {
    match parse(text)? {
        Playlist::MediaPlaylist(media) => track(media, url),
        Playlist::MasterPlaylist(_) => Err(MediaError::Invalid(
            "a list of qualities came where the pieces of one were expected".into(),
        )),
    }
}

fn parse(text: &[u8]) -> Result<Playlist, MediaError> {
    m3u8_rs::parse_playlist_res(text)
        .map_err(|_| MediaError::Invalid("it isn't a valid HLS playlist".into()))
}

fn from_master(master: MasterPlaylist, url: &Url) -> Result<Manifest, MediaError> {
    for session_key in &master.session_key {
        lock_of(&session_key.0, url)?;
    }

    let mut audio = Vec::new();
    for media in &master.alternatives {
        // Without a URI, the sound is part of the qualities themselves.
        let (AlternativeMediaType::Audio, Some(uri)) = (&media.media_type, &media.uri) else {
            continue;
        };
        audio.push(AudioTrack {
            name: media.name.clone(),
            language: media.language.clone(),
            group: Some(media.group_id.clone()),
            bandwidth: None,
            codecs: None,
            default: media.default,
            source: Source::Playlist(join(url, uri)?),
        });
    }

    let mut seen = HashSet::new();
    let mut qualities = Vec::new();
    for variant in master.variants {
        // I-frame playlists are for scrubbing previews, and the same quality may be listed
        // again as a backup copy.
        if variant.is_i_frame || !seen.insert(variant.uri.clone()) {
            continue;
        }
        let audio_group = variant.audio.filter(|group| {
            audio
                .iter()
                .any(|a: &AudioTrack| a.group.as_ref() == Some(group))
        });
        qualities.push(Quality {
            bandwidth: variant.average_bandwidth.unwrap_or(variant.bandwidth),
            width: variant.resolution.and_then(|r| u32::try_from(r.width).ok()),
            height: variant
                .resolution
                .and_then(|r| u32::try_from(r.height).ok()),
            frame_rate: variant.frame_rate,
            codecs: variant.codecs,
            audio_group,
            source: Source::Playlist(join(url, &variant.uri)?),
        });
    }
    if qualities.is_empty() {
        return Err(MediaError::Invalid("it lists no qualities".into()));
    }
    sort_best_first(&mut qualities);

    Ok(Manifest {
        kind: StreamKind::Hls,
        title: None,
        duration: None,
        qualities,
        audio,
    })
}

fn track(media: MediaPlaylist, url: &Url) -> Result<Track, MediaError> {
    // A playlist that is still growing never says it is complete.
    if !media.end_list && media.playlist_type != Some(MediaPlaylistType::Vod) {
        return Err(MediaError::Live);
    }
    if media.segments.is_empty() {
        return Err(MediaError::Invalid("it lists no pieces".into()));
    }
    if media.segments.len() > MAX_PIECES {
        return Err(MediaError::Invalid("it lists far too many pieces".into()));
    }

    // m3u8-rs puts a key or map only on the first piece after its tag; it applies until
    // the next one.
    let mut lock: Option<Lock> = None;
    let mut init: Option<Piece> = None;
    // Where the last byte range ended, in which file: a range without an offset follows it.
    let mut last_range: Option<(Url, u64)> = None;
    let mut pieces = Vec::with_capacity(media.segments.len());
    let mut duration = 0.0;

    for (index, segment) in media.segments.iter().enumerate() {
        let sequence = media.media_sequence + index as u64;
        // m3u8-rs 6.0.1 rejects `METHOD=NONE` without an IV (its check is the wrong way
        // round) and keeps the tag as an unknown one instead.
        let unlocks = segment.unknown_tags.iter().any(|tag| {
            tag.tag == "X-KEY"
                && tag
                    .rest
                    .as_deref()
                    .is_some_and(|r| r.contains("METHOD=NONE"))
        });
        if unlocks {
            lock = None;
        }
        if let Some(key) = &segment.key {
            lock = lock_of(key, url)?;
        }
        let key_for = |lock: &Option<Lock>| {
            lock.as_ref().map(|(key_url, iv)| Key {
                url: key_url.clone(),
                iv: iv.unwrap_or_else(|| aes128::iv_for_sequence(sequence)),
            })
        };

        if let Some(map) = &segment.map {
            let map_url = join(url, &map.uri)?;
            let range = match &map.byte_range {
                Some(r) => Some(range_of(r.length, r.offset.unwrap_or(0))?),
                None => None,
            };
            let piece = Piece {
                url: map_url,
                range,
                key: key_for(&lock),
            };
            match &init {
                None => init = Some(piece),
                Some(known) if known.url == piece.url && known.range == piece.range => {}
                Some(_) => {
                    return Err(MediaError::Unsupported(
                        "The video changes format partway through, which OmniDM can't download yet"
                            .into(),
                    ));
                }
            }
        }

        let piece_url = join(url, &segment.uri)?;
        let range = match &segment.byte_range {
            Some(r) => {
                let start = match (r.offset, &last_range) {
                    (Some(offset), _) => offset,
                    (None, Some((last_url, end))) if *last_url == piece_url => *end,
                    (None, _) => {
                        return Err(MediaError::Invalid(
                            "a byte range doesn't say where it starts".into(),
                        ));
                    }
                };
                let range = range_of(r.length, start)?;
                last_range = Some((piece_url.clone(), range.end + 1));
                Some(range)
            }
            None => None,
        };
        duration += f64::from(segment.duration);
        pieces.push(Piece {
            url: piece_url,
            range,
            key: key_for(&lock),
        });
    }

    Ok(Track {
        id: None,
        headers: Vec::new(),
        // Pieces with a header piece are fragmented MP4 (RFC 8216 §3.3).
        container: if init.is_some() {
            Container::Mp4
        } else {
            Container::Ts
        },
        init,
        pieces,
        duration: Some(duration),
    })
}

/// A key's address, and the IV its tag gives, if any.
type Lock = (Url, Option<[u8; 16]>);

/// What an `EXT-X-KEY` means for the pieces after it: `None` when they aren't locked, or
/// the key's address and the IV, if it gives one. DRM is refused.
fn lock_of(key: &m3u8_rs::Key, url: &Url) -> Result<Option<Lock>, MediaError> {
    match &key.method {
        KeyMethod::None => Ok(None),
        KeyMethod::AES128 => {
            // A key format other than the plain one means the key comes from a DRM system.
            if let Some(format) = key.keyformat.as_deref().filter(|f| *f != "identity") {
                return Err(MediaError::Protected(format.to_owned()));
            }
            let uri = key
                .uri
                .as_deref()
                .ok_or_else(|| MediaError::Invalid("a key without an address".into()))?;
            let iv = key.iv.as_deref().map(aes128::parse_iv).transpose()?;
            Ok(Some((join(url, uri)?, iv)))
        }
        KeyMethod::SampleAES => Err(MediaError::Protected("SAMPLE-AES".into())),
        KeyMethod::Other(method) => Err(MediaError::Protected(method.clone())),
    }
}

fn range_of(length: u64, start: u64) -> Result<ByteRange, MediaError> {
    if length == 0 {
        return Err(MediaError::Invalid("an empty byte range".into()));
    }
    let end = start
        .checked_add(length - 1)
        .ok_or_else(|| MediaError::Invalid("a byte range past the end of any file".into()))?;
    Ok(ByteRange { start, end })
}
