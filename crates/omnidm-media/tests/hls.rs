// SPDX-License-Identifier: GPL-3.0-or-later

use omnidm_media::{
    ByteRange, Container, MediaError, Source, StreamKind, Track, Url, aes128, hls, read_manifest,
};

fn url(text: &str) -> Url {
    Url::parse(text).unwrap()
}

const BASE: &str = "https://cdn.example.com/show/master.m3u8";

fn track(text: &str) -> Result<Track, MediaError> {
    hls::read_track(
        text.as_bytes(),
        &url("https://cdn.example.com/show/720p/index.m3u8"),
    )
}

fn playlist_url(source: &Source) -> &str {
    match source {
        Source::Playlist(url) => url.as_str(),
        Source::Track(_) => panic!("expected a playlist to fetch"),
    }
}

#[test]
fn master_playlist_lists_qualities_best_first() {
    let text = "#EXTM3U
#EXT-X-MEDIA:TYPE=AUDIO,GROUP-ID=\"aac\",NAME=\"English\",LANGUAGE=\"en\",DEFAULT=YES,URI=\"audio/en.m3u8\"
#EXT-X-MEDIA:TYPE=AUDIO,GROUP-ID=\"aac\",NAME=\"Deutsch\",LANGUAGE=\"de\",URI=\"audio/de.m3u8\"
#EXT-X-MEDIA:TYPE=AUDIO,GROUP-ID=\"inside\",NAME=\"Main\",DEFAULT=YES
#EXT-X-STREAM-INF:BANDWIDTH=1400000,AVERAGE-BANDWIDTH=1200000,RESOLUTION=1280x720,FRAME-RATE=29.970,CODECS=\"avc1.4d401f,mp4a.40.2\",AUDIO=\"aac\"
720p/index.m3u8
#EXT-X-STREAM-INF:BANDWIDTH=5000000,RESOLUTION=1920x1080,AUDIO=\"aac\"
https://other.example.com/1080p.m3u8
#EXT-X-STREAM-INF:BANDWIDTH=600000,RESOLUTION=640x360,AUDIO=\"inside\"
360p/index.m3u8
#EXT-X-STREAM-INF:BANDWIDTH=1400000,RESOLUTION=1280x720,AUDIO=\"aac\"
720p/index.m3u8
#EXT-X-I-FRAME-STREAM-INF:BANDWIDTH=86000,URI=\"iframes.m3u8\"
";
    let manifest = read_manifest(text, &url(BASE)).unwrap();
    assert_eq!(manifest.kind, StreamKind::Hls);
    assert_eq!(manifest.duration, None);

    // The backup copy of 720p and the I-frame playlist are left out.
    let heights: Vec<_> = manifest.qualities.iter().map(|q| q.height).collect();
    assert_eq!(heights, [Some(1080), Some(720), Some(360)]);
    let [q1080, q720, q360] = &manifest.qualities[..] else {
        unreachable!()
    };
    assert_eq!(
        playlist_url(&q1080.source),
        "https://other.example.com/1080p.m3u8"
    );
    assert_eq!(
        playlist_url(&q720.source),
        "https://cdn.example.com/show/720p/index.m3u8"
    );
    assert_eq!(q720.bandwidth, 1_200_000, "the average is the better guess");
    assert_eq!(q720.width, Some(1280));
    assert!((q720.frame_rate.unwrap() - 29.97).abs() < 0.001);
    assert_eq!(q720.codecs.as_deref(), Some("avc1.4d401f,mp4a.40.2"));

    // Separate sound tracks, and the one picked unless the user chooses.
    let names: Vec<_> = manifest.audio_for(q720).map(|a| a.name.as_str()).collect();
    assert_eq!(names, ["English", "Deutsch"]);
    let english = manifest.default_audio(q1080).unwrap();
    assert_eq!(english.language.as_deref(), Some("en"));
    assert_eq!(
        playlist_url(&english.source),
        "https://cdn.example.com/show/audio/en.m3u8"
    );

    // The sound of 360p is part of it: its group has no tracks of its own.
    assert_eq!(q360.audio_group, None);
    assert_eq!(manifest.audio_for(q360).count(), 0);
    assert_eq!(manifest.default_audio(q360), None);
}

#[test]
fn a_media_playlist_is_one_quality_ready_to_download() {
    let text = "#EXTM3U
#EXT-X-TARGETDURATION:6
#EXTINF:6.0,
a.ts
#EXTINF:4.5,
b.ts
#EXT-X-ENDLIST
";
    let manifest = read_manifest(text, &url(BASE)).unwrap();
    assert_eq!(manifest.qualities.len(), 1);
    assert_eq!(manifest.duration, Some(10.5));
    let Source::Track(track) = &manifest.qualities[0].source else {
        panic!()
    };
    assert_eq!(track.container, Container::Ts);
    assert_eq!(track.init, None);
    let urls: Vec<_> = track.pieces.iter().map(|p| p.url.as_str()).collect();
    assert_eq!(
        urls,
        [
            "https://cdn.example.com/show/a.ts",
            "https://cdn.example.com/show/b.ts"
        ]
    );
    assert!(
        track
            .pieces
            .iter()
            .all(|p| p.key.is_none() && p.range.is_none())
    );
}

#[test]
fn keys_apply_until_the_next_key_tag() {
    let text = "#EXTM3U
#EXT-X-TARGETDURATION:4
#EXT-X-MEDIA-SEQUENCE:100
#EXTINF:4,
clear.ts
#EXT-X-KEY:METHOD=AES-128,URI=\"/keys/k1\"
#EXTINF:4,
locked1.ts
#EXTINF:4,
locked2.ts
#EXT-X-KEY:METHOD=AES-128,URI=\"k2\",IV=0x0000000000000000000000000000ABCD,KEYFORMAT=\"identity\"
#EXTINF:4,
locked3.ts
#EXT-X-KEY:METHOD=NONE
#EXTINF:4,
clear2.ts
#EXT-X-ENDLIST
";
    let track = track(text).unwrap();
    let keys: Vec<_> = track
        .pieces
        .iter()
        .map(|p| p.key.as_ref().map(|k| (k.url.as_str(), k.iv)))
        .collect();
    assert_eq!(
        keys,
        [
            None,
            // Without an IV, a piece's IV is its sequence number.
            Some((
                "https://cdn.example.com/keys/k1",
                aes128::iv_for_sequence(101)
            )),
            Some((
                "https://cdn.example.com/keys/k1",
                aes128::iv_for_sequence(102)
            )),
            Some((
                "https://cdn.example.com/show/720p/k2",
                aes128::iv_for_sequence(0xabcd)
            )),
            None,
        ]
    );
}

#[test]
fn byte_ranges_follow_on_from_the_last_one() {
    let text = "#EXTM3U
#EXT-X-TARGETDURATION:4
#EXTINF:4,
#EXT-X-BYTERANGE:1000@0
all.ts
#EXTINF:4,
#EXT-X-BYTERANGE:500
all.ts
#EXTINF:4,
#EXT-X-BYTERANGE:200@5000
all.ts
#EXT-X-ENDLIST
";
    let ranges: Vec<_> = track(text)
        .unwrap()
        .pieces
        .iter()
        .map(|p| p.range)
        .collect();
    assert_eq!(
        ranges,
        [
            Some(ByteRange { start: 0, end: 999 }),
            Some(ByteRange {
                start: 1000,
                end: 1499
            }),
            Some(ByteRange {
                start: 5000,
                end: 5199
            }),
        ]
    );
    assert_eq!(ranges[1].unwrap().size(), 500);

    let no_start = "#EXTM3U
#EXT-X-TARGETDURATION:4
#EXTINF:4,
#EXT-X-BYTERANGE:500
all.ts
#EXT-X-ENDLIST
";
    assert!(matches!(track(no_start), Err(MediaError::Invalid(_))));
}

#[test]
fn a_header_piece_means_fragmented_mp4() {
    let text = "#EXTM3U
#EXT-X-TARGETDURATION:4
#EXT-X-MAP:URI=\"init.mp4\",BYTERANGE=\"720@0\"
#EXTINF:4,
seg1.m4s
#EXTINF:4,
seg2.m4s
#EXT-X-ENDLIST
";
    let track = track(text).unwrap();
    assert_eq!(track.container, Container::Mp4);
    let init = track.init.unwrap();
    assert_eq!(
        init.url.as_str(),
        "https://cdn.example.com/show/720p/init.mp4"
    );
    assert_eq!(init.range, Some(ByteRange { start: 0, end: 719 }));
    assert_eq!(track.pieces.len(), 2);

    let changes = "#EXTM3U
#EXT-X-TARGETDURATION:4
#EXT-X-MAP:URI=\"init1.mp4\"
#EXTINF:4,
seg1.m4s
#EXT-X-DISCONTINUITY
#EXT-X-MAP:URI=\"init2.mp4\"
#EXTINF:4,
seg2.m4s
#EXT-X-ENDLIST
";
    assert!(matches!(
        self::track(changes),
        Err(MediaError::Unsupported(_))
    ));
}

#[test]
fn live_streams_are_refused() {
    let live = "#EXTM3U
#EXT-X-TARGETDURATION:4
#EXT-X-MEDIA-SEQUENCE:5000
#EXTINF:4,
a.ts
";
    assert_eq!(track(live), Err(MediaError::Live));
    let event = format!("#EXTM3U\n#EXT-X-PLAYLIST-TYPE:EVENT\n{}", &live[8..]);
    assert_eq!(track(&event), Err(MediaError::Live));
    // A finished one is fine, as is a VOD playlist that forgot its end tag.
    assert!(track(&format!("{event}#EXT-X-ENDLIST\n")).is_ok());
    assert!(
        track(&format!(
            "#EXTM3U\n#EXT-X-PLAYLIST-TYPE:VOD\n{}",
            &live[8..]
        ))
        .is_ok()
    );
}

#[test]
fn drm_is_refused() {
    let with_key = |key: &str| {
        format!("#EXTM3U\n#EXT-X-TARGETDURATION:4\n{key}\n#EXTINF:4,\na.ts\n#EXT-X-ENDLIST\n")
    };
    assert_eq!(
        track(&with_key(
            "#EXT-X-KEY:METHOD=SAMPLE-AES,URI=\"skd://x\",KEYFORMAT=\"com.apple.streamingkeydelivery\""
        )),
        Err(MediaError::Protected("SAMPLE-AES".into()))
    );
    assert_eq!(
        track(&with_key(
            "#EXT-X-KEY:METHOD=SAMPLE-AES-CTR,URI=\"data:text/plain;base64,AAAA\""
        )),
        Err(MediaError::Protected("SAMPLE-AES-CTR".into()))
    );
    assert_eq!(
        track(&with_key(
            "#EXT-X-KEY:METHOD=AES-128,URI=\"https://lic.example.com\",KEYFORMAT=\"urn:uuid:edef8ba9-79d6-4ace-a3c8-27dcd51d21ed\""
        )),
        Err(MediaError::Protected(
            "urn:uuid:edef8ba9-79d6-4ace-a3c8-27dcd51d21ed".into()
        ))
    );

    let master = "#EXTM3U
#EXT-X-SESSION-KEY:METHOD=SAMPLE-AES,URI=\"skd://x\",KEYFORMAT=\"com.apple.streamingkeydelivery\"
#EXT-X-STREAM-INF:BANDWIDTH=1000000
a.m3u8
";
    let refused = read_manifest(master, &url(BASE)).unwrap_err();
    assert!(matches!(refused, MediaError::Protected(_)));
    assert!(
        refused.to_string().contains("protected with DRM"),
        "{refused}"
    );
}

#[test]
fn bad_playlists_say_so() {
    // Pieces must be web addresses.
    let ftp = "#EXTM3U\n#EXT-X-TARGETDURATION:4\n#EXTINF:4,\nftp://x/a.ts\n#EXT-X-ENDLIST\n";
    assert!(matches!(track(ftp), Err(MediaError::Unsupported(_))));
    let empty = "#EXTM3U\n#EXT-X-TARGETDURATION:4\n#EXT-X-ENDLIST\n";
    assert!(matches!(track(empty), Err(MediaError::Invalid(_))));
    // A list of qualities where one quality's pieces were expected.
    let master = "#EXTM3U\n#EXT-X-STREAM-INF:BANDWIDTH=1\na.m3u8\n";
    assert!(matches!(track(master), Err(MediaError::Invalid(_))));
    assert!(matches!(
        read_manifest("<html>Not found</html>", &url(BASE)),
        Err(MediaError::Invalid(_))
    ));
}

#[test]
fn tells_the_kinds_apart_by_content() {
    let hls = "\u{feff}\n  #EXTM3U\n#EXT-X-TARGETDURATION:4\n#EXTINF:4,\na.ts\n#EXT-X-ENDLIST\n";
    assert_eq!(
        read_manifest(hls, &url(BASE)).unwrap().kind,
        StreamKind::Hls
    );
}
