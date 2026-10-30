// SPDX-License-Identifier: GPL-3.0-or-later

//! DASH manifests (MPD, ISO/IEC 23009-1). Every quality and sound track is listed in the
//! one file, so they come out ready to download.

use dash_mpd_core::{
    AdaptationSet, BaseURL, ContentProtection, MPD, Period, Representation, SegmentTemplate,
};
use url::Url;

use crate::{
    AudioTrack, ByteRange, Container, MAX_PIECES, Manifest, MediaError, Piece, Quality, Source,
    StreamKind, Track, join, sort_best_first,
};

/// The [`AudioTrack::group`] of every sound track: in DASH any of them goes with any picture.
const AUDIO_GROUP: &str = "audio";

pub fn read_manifest(text: &str, url: &Url) -> Result<Manifest, MediaError> {
    let mpd = dash_mpd_core::parse(text).map_err(|e| MediaError::Invalid(e.to_string()))?;
    if mpd.mpdtype.as_deref() == Some("dynamic") {
        return Err(MediaError::Live);
    }
    refuse_protected(&mpd)?;

    let periods: Vec<&Period> = mpd
        .periods
        .iter()
        .filter(|p| !p.adaptations.is_empty())
        .collect();
    let period = match periods.as_slice() {
        [] => return Err(MediaError::Invalid("it lists nothing to play".into())),
        [period] => *period,
        _ => {
            return Err(MediaError::Unsupported(
                "The video is made of several parts (DASH periods), which OmniDM can't download yet"
                    .into(),
            ));
        }
    };
    let duration = period
        .duration
        .or_else(|| {
            let start = period.start.unwrap_or_default();
            mpd.mediaPresentationDuration
                .map(|d| d.saturating_sub(start))
        })
        .map(|d| d.as_secs_f64());

    let period_base = base_of(&period.BaseURL, &base_of(&mpd.base_url, url)?)?;
    let mut video = Vec::new();
    let mut audio = Vec::new();
    for adaptation in &period.adaptations {
        let is_video = dash_mpd_core::is_video_adaptation(&adaptation);
        let is_audio = dash_mpd_core::is_audio_adaptation(&adaptation);
        if !is_video && !is_audio {
            continue; // subtitles, thumbnails
        }
        let adaptation_base = base_of(&adaptation.BaseURL, &period_base)?;
        for rep in &adaptation.representations {
            let base = base_of(&rep.BaseURL, &adaptation_base)?;
            let track = track(period, adaptation, rep, &base, duration)?;
            if is_video {
                video.push(Quality {
                    bandwidth: rep.bandwidth.unwrap_or(0),
                    width: rep
                        .width
                        .or(adaptation.width)
                        .and_then(|w| u32::try_from(w).ok()),
                    height: rep
                        .height
                        .or(adaptation.height)
                        .and_then(|h| u32::try_from(h).ok()),
                    frame_rate: rep
                        .frameRate
                        .as_deref()
                        .or(adaptation.frameRate.as_deref())
                        .and_then(frame_rate),
                    codecs: rep.codecs.clone().or_else(|| adaptation.codecs.clone()),
                    audio_group: None,
                    source: Source::Track(track),
                });
            } else {
                let language = rep.lang.clone().or_else(|| adaptation.lang.clone());
                let label = rep.Label.first().or(adaptation.Label.first());
                audio.push(AudioTrack {
                    name: label
                        .map(|l| l.content.clone())
                        .or_else(|| language.clone())
                        .unwrap_or_else(|| "Sound".into()),
                    language,
                    group: Some(AUDIO_GROUP.into()),
                    bandwidth: rep.bandwidth,
                    codecs: rep.codecs.clone().or_else(|| adaptation.codecs.clone()),
                    default: false,
                    source: Source::Track(track),
                });
            }
        }
    }

    // Best sound first, so it is the one picked unless the user chooses.
    audio.sort_by_key(|a| std::cmp::Reverse(a.bandwidth));
    let (mut qualities, audio) = if video.is_empty() {
        // Sound only: its tracks are the qualities to choose from.
        let qualities = audio
            .into_iter()
            .map(|a| Quality {
                bandwidth: a.bandwidth.unwrap_or(0),
                width: None,
                height: None,
                frame_rate: None,
                codecs: a.codecs,
                audio_group: None,
                source: a.source,
            })
            .collect();
        (qualities, Vec::new())
    } else {
        if !audio.is_empty() {
            for quality in &mut video {
                quality.audio_group = Some(AUDIO_GROUP.into());
            }
        }
        (video, audio)
    };
    if qualities.is_empty() {
        return Err(MediaError::Invalid("it lists no picture or sound".into()));
    }
    sort_best_first(&mut qualities);

    Ok(Manifest {
        kind: StreamKind::Dash,
        title: None,
        duration,
        qualities,
        audio,
    })
}

/// Refuses a manifest with any `ContentProtection`, naming the DRM system if it can.
fn refuse_protected(mpd: &MPD) -> Result<(), MediaError> {
    let mut found: Vec<&ContentProtection> = mpd.ContentProtection.iter().collect();
    for period in &mpd.periods {
        found.extend(&period.ContentProtection);
        for adaptation in &period.adaptations {
            found.extend(&adaptation.ContentProtection);
            for rep in &adaptation.representations {
                found.extend(&rep.ContentProtection);
            }
        }
    }
    if found.is_empty() {
        return Ok(());
    }
    let names: Vec<String> = found
        .iter()
        .map(|cp| dash_mpd_core::content_protection_type(cp))
        .collect();
    // "cenc" only says the pieces are encrypted; prefer the DRM system that holds the key.
    let name = names
        .iter()
        .find(|n| *n != "cenc")
        .or(names.first())
        .cloned()
        .unwrap_or_default();
    Err(MediaError::Protected(name))
}

/// The pieces of one representation, from whichever of `SegmentTemplate`, `SegmentList`
/// or a single file (`SegmentBase`) describes them.
fn track(
    period: &Period,
    adaptation: &AdaptationSet,
    rep: &Representation,
    base: &Url,
    duration: Option<f64>,
) -> Result<Track, MediaError> {
    let mime = rep
        .mimeType
        .as_deref()
        .or(adaptation.mimeType.as_deref())
        .unwrap_or("");
    let container = if mime.contains("webm") {
        Container::WebM
    } else if mime.contains("mp2t") {
        Container::Ts
    } else {
        Container::Mp4
    };

    // A template's settings are inherited from the levels above and can be overridden.
    let template = [
        period.SegmentTemplate.as_ref(),
        adaptation.SegmentTemplate.as_ref(),
        rep.SegmentTemplate.as_ref(),
    ]
    .into_iter()
    .flatten()
    .fold(None, |general: Option<SegmentTemplate>, specific| {
        Some(match general {
            Some(general) => merge(general, specific),
            None => specific.clone(),
        })
    });
    let list = rep
        .SegmentList
        .as_ref()
        .or(adaptation.SegmentList.as_ref())
        .or(period.SegmentList.as_ref());

    let (init, pieces) = if let Some(template) = template {
        from_template(&template, rep, base, duration)?
    } else if let Some(list) = list {
        let init = match &list.Initialization {
            Some(init) => Some(Piece {
                url: match &init.sourceURL {
                    Some(source) => join(base, source)?,
                    None => base.clone(),
                },
                range: init.range.as_deref().map(range_of).transpose()?,
                key: None,
            }),
            None => None,
        };
        let pieces = list
            .segment_urls
            .iter()
            .map(|segment| {
                Ok(Piece {
                    url: match &segment.media {
                        Some(media) => join(base, media)?,
                        None => base.clone(),
                    },
                    range: segment.mediaRange.as_deref().map(range_of).transpose()?,
                    key: None,
                })
            })
            .collect::<Result<Vec<_>, MediaError>>()?;
        (init, pieces)
    } else {
        // One file holds everything (SegmentBase, or nothing said at all).
        let whole = Piece {
            url: base.clone(),
            range: None,
            key: None,
        };
        (None, vec![whole])
    };

    if pieces.is_empty() {
        return Err(MediaError::Invalid("a track lists no pieces".into()));
    }
    if pieces.len() > MAX_PIECES {
        return Err(MediaError::Invalid("it lists far too many pieces".into()));
    }
    Ok(Track {
        id: None,
        headers: Vec::new(),
        init,
        pieces,
        container,
        duration,
    })
}

/// `specific`'s settings over `general`'s.
fn merge(general: SegmentTemplate, specific: &SegmentTemplate) -> SegmentTemplate {
    let s = specific.clone();
    SegmentTemplate {
        media: s.media.or(general.media),
        initialization: s.initialization.or(general.initialization),
        startNumber: s.startNumber.or(general.startNumber),
        endNumber: s.endNumber.or(general.endNumber),
        duration: s.duration.or(general.duration),
        timescale: s.timescale.or(general.timescale),
        presentationTimeOffset: s.presentationTimeOffset.or(general.presentationTimeOffset),
        Initialization: s.Initialization.or(general.Initialization),
        SegmentTimeline: s.SegmentTimeline.or(general.SegmentTimeline),
        ..general
    }
}

/// The header piece and pieces a `SegmentTemplate` describes, either piece by piece
/// (`SegmentTimeline`) or as equal lengths filling the period (`@duration`).
fn from_template(
    template: &SegmentTemplate,
    rep: &Representation,
    base: &Url,
    duration: Option<f64>,
) -> Result<(Option<Piece>, Vec<Piece>), MediaError> {
    let id = rep.id.as_deref().unwrap_or("");
    let bandwidth = rep.bandwidth.unwrap_or(0);
    let piece =
        |number: Option<u64>, time: Option<u64>, pattern: &str| -> Result<Piece, MediaError> {
            Ok(Piece {
                url: join(base, &fill(pattern, id, bandwidth, number, time))?,
                range: None,
                key: None,
            })
        };

    let init = match (&template.initialization, &template.Initialization) {
        (Some(pattern), _) => Some(piece(None, None, pattern)?),
        (None, Some(init)) => Some(Piece {
            url: match &init.sourceURL {
                Some(source) => join(base, &fill(source, id, bandwidth, None, None))?,
                None => base.clone(),
            },
            range: init.range.as_deref().map(range_of).transpose()?,
            key: None,
        }),
        (None, None) => None,
    };

    let media = template
        .media
        .as_deref()
        .ok_or_else(|| MediaError::Invalid("a segment template without @media".into()))?;
    let timescale = template.timescale.unwrap_or(1).max(1);
    let offset = template.presentationTimeOffset.unwrap_or(0);
    let mut number = template.startNumber.unwrap_or(1);
    let mut pieces = Vec::new();
    let too_many = || MediaError::Invalid("it lists far too many pieces".into());

    if let Some(timeline) = &template.SegmentTimeline {
        let mut time = 0u64;
        for (index, s) in timeline.segments.iter().enumerate() {
            if let Some(t) = s.t {
                time = t;
            }
            if s.d == 0 {
                return Err(MediaError::Invalid("a piece with no length".into()));
            }
            let repeats = match s.r {
                Some(r) if r >= 0 => r as u64,
                Some(_) => {
                    // Repeats until the next entry's start, or the end of the period.
                    let end = match timeline.segments.get(index + 1).and_then(|next| next.t) {
                        Some(next) => next,
                        None => {
                            let seconds = duration.ok_or_else(|| {
                                MediaError::Invalid("a timeline repeats without an end".into())
                            })?;
                            offset + (seconds * timescale as f64).round() as u64
                        }
                    };
                    end.saturating_sub(time).div_ceil(s.d).saturating_sub(1)
                }
                None => 0,
            };
            if repeats >= (MAX_PIECES - pieces.len()) as u64 {
                return Err(too_many());
            }
            for _ in 0..=repeats {
                pieces.push(piece(Some(number), Some(time), media)?);
                time += s.d;
                number += 1;
            }
        }
    } else if let Some(length) = template.duration.filter(|d| *d > 0.0) {
        let seconds = duration.ok_or_else(|| {
            MediaError::Invalid("a segment template whose length is unknown".into())
        })?;
        let count = (seconds * timescale as f64 / length).ceil();
        let count = match template.endNumber {
            Some(end) if end >= number => count.min((end - number + 1) as f64),
            _ => count,
        };
        if count > MAX_PIECES as f64 {
            return Err(too_many());
        }
        for index in 0..count as u64 {
            let time = offset + (index as f64 * length) as u64;
            pieces.push(piece(Some(number), Some(time), media)?);
            number += 1;
        }
    } else {
        return Err(MediaError::Invalid(
            "a segment template with neither a timeline nor a length".into(),
        ));
    }
    Ok((init, pieces))
}

/// Fills in a template's `$RepresentationID$`, `$Number$`, `$Time$`, `$Bandwidth$` and
/// `$$`. Numbers can have a width (`$Number%05d$`). Anything else is left as it is.
fn fill(pattern: &str, id: &str, bandwidth: u64, number: Option<u64>, time: Option<u64>) -> String {
    let mut out = String::with_capacity(pattern.len() + 16);
    let mut rest = pattern;
    while let Some(open) = rest.find('$') {
        out.push_str(&rest[..open]);
        let after = &rest[open + 1..];
        let Some(close) = after.find('$') else {
            out.push_str(&rest[open..]);
            return out;
        };
        let name = &after[..close];
        let (name, format) = name.split_once('%').unwrap_or((name, ""));
        let value = match name {
            "" => Some("$".to_owned()),
            "RepresentationID" => Some(id.to_owned()),
            "Number" => number.map(|n| padded(n, format)),
            "Time" => time.map(|t| padded(t, format)),
            "Bandwidth" => Some(padded(bandwidth, format)),
            _ => None,
        };
        match value {
            Some(value) => out.push_str(&value),
            None => out.push_str(&rest[open..open + close + 2]),
        }
        rest = &after[close + 1..];
    }
    out.push_str(rest);
    out
}

/// `value` written as a `%0Nd` format asks: at least N digits, zero-padded.
fn padded(value: u64, format: &str) -> String {
    let width = format
        .strip_suffix('d')
        .map(|w| w.trim_start_matches('0'))
        .and_then(|w| w.parse::<usize>().ok())
        .unwrap_or(0);
    format!("{value:0width$}")
}

/// The first `BaseURL` resolved against `parent`, or `parent` if there is none.
fn base_of(list: &[BaseURL], parent: &Url) -> Result<Url, MediaError> {
    match list.first() {
        Some(base) => join(parent, &base.base),
        None => Ok(parent.clone()),
    }
}

/// Reads a `first-last` byte range.
fn range_of(text: &str) -> Result<ByteRange, MediaError> {
    let bad = || MediaError::Invalid(format!("a bad byte range: {text}"));
    let (start, end) = text.trim().split_once('-').ok_or_else(bad)?;
    let start: u64 = start.parse().map_err(|_| bad())?;
    let end: u64 = end.parse().map_err(|_| bad())?;
    if end < start {
        return Err(bad());
    }
    Ok(ByteRange { start, end })
}

/// Reads a frame rate such as `25` or `30000/1001`.
fn frame_rate(text: &str) -> Option<f64> {
    match text.split_once('/') {
        Some((num, den)) => {
            let den: f64 = den.parse().ok()?;
            (den != 0.0).then_some(num.parse::<f64>().ok()? / den)
        }
        None => text.parse().ok(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fills_templates() {
        let fill = |p| fill(p, "v1", 800_000, Some(7), Some(90_000));
        assert_eq!(fill("$RepresentationID$/seg-$Number$.m4s"), "v1/seg-7.m4s");
        assert_eq!(fill("$Number%05d$.m4s"), "00007.m4s");
        assert_eq!(fill("t$Time$-$Bandwidth$"), "t90000-800000");
        assert_eq!(fill("a$$b"), "a$b");
        assert_eq!(fill("$Unknown$-$Number$"), "$Unknown$-7");
        assert_eq!(fill("no-end$Number"), "no-end$Number");
        assert_eq!(
            super::fill("init-$Number$.mp4", "v1", 0, None, None),
            "init-$Number$.mp4"
        );
    }

    #[test]
    fn reads_ranges_and_rates() {
        assert_eq!(range_of("0-99").unwrap(), ByteRange { start: 0, end: 99 });
        assert!(range_of("99-0").is_err());
        assert!(range_of("5").is_err());
        assert_eq!(frame_rate("25"), Some(25.0));
        assert!((frame_rate("30000/1001").unwrap() - 29.97).abs() < 0.01);
        assert_eq!(frame_rate("1/0"), None);
    }
}
