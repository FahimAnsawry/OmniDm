// SPDX-License-Identifier: GPL-3.0-or-later

use omnidm_media::{
    ByteRange, Container, Manifest, MediaError, Source, StreamKind, Track, Url, read_manifest,
};

const MPD_URL: &str = "https://cdn.example.com/films/one/manifest.mpd";

fn read(text: &str) -> Result<Manifest, MediaError> {
    read_manifest(text, &Url::parse(MPD_URL).unwrap())
}

fn track(source: &Source) -> &Track {
    match source {
        Source::Track(track) => track,
        Source::Playlist(_) => panic!("DASH tracks come ready"),
    }
}

fn urls(track: &Track) -> Vec<&str> {
    track.pieces.iter().map(|p| p.url.as_str()).collect()
}

fn mpd(period: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<MPD xmlns="urn:mpeg:dash:schema:mpd:2011" type="static" mediaPresentationDuration="PT20S" minBufferTime="PT2S" profiles="urn:mpeg:dash:profile:isoff-on-demand:2011">
  {period}
</MPD>"#
    )
}

#[test]
fn timeline_template_with_video_and_sound() {
    let text = mpd(r#"<BaseURL>media/</BaseURL>
  <Period id="1">
    <AdaptationSet mimeType="video/mp4" contentType="video" segmentAlignment="true">
      <SegmentTemplate timescale="1000" initialization="$RepresentationID$/init.mp4" media="$RepresentationID$/$Number%05d$-$Time$.m4s" startNumber="1">
        <SegmentTimeline>
          <S t="0" d="4000" r="2"/>
          <S d="3000"/>
          <S d="1000" r="-1"/>
        </SegmentTimeline>
      </SegmentTemplate>
      <Representation id="v480" bandwidth="900000" width="854" height="480" codecs="avc1.4d401e" frameRate="30000/1001"/>
      <Representation id="v1080" bandwidth="4500000" width="1920" height="1080" codecs="avc1.640028" frameRate="25"/>
    </AdaptationSet>
    <AdaptationSet mimeType="audio/mp4" lang="en">
      <Label>English</Label>
      <SegmentTemplate timescale="48000" initialization="a/init-$Bandwidth$.mp4" media="a/$Number$.m4s" duration="192000" startNumber="0"/>
      <Representation id="a64" bandwidth="64000" codecs="mp4a.40.2"/>
      <Representation id="a128" bandwidth="128000" codecs="mp4a.40.2"/>
    </AdaptationSet>
    <AdaptationSet mimeType="text/vtt" lang="en">
      <Representation id="subs" bandwidth="100"><BaseURL>subs.vtt</BaseURL></Representation>
    </AdaptationSet>
  </Period>"#);
    let manifest = read(&text).unwrap();
    assert_eq!(manifest.kind, StreamKind::Dash);
    assert_eq!(manifest.duration, Some(20.0));

    let heights: Vec<_> = manifest.qualities.iter().map(|q| q.height).collect();
    assert_eq!(
        heights,
        [Some(1080), Some(480)],
        "best first, subtitles left out"
    );
    let best = &manifest.qualities[0];
    assert_eq!(best.bandwidth, 4_500_000);
    assert_eq!(best.frame_rate, Some(25.0));
    assert!((manifest.qualities[1].frame_rate.unwrap() - 29.97).abs() < 0.01);

    let video = track(&best.source);
    assert_eq!(video.container, Container::Mp4);
    assert_eq!(
        video.init.as_ref().unwrap().url.as_str(),
        "https://cdn.example.com/films/one/media/v1080/init.mp4"
    );
    // 3 × 4 s, then 3 s, then 1 s pieces until the 20 s are filled.
    let names: Vec<_> = urls(video)
        .iter()
        .map(|u| u.rsplit('/').next().unwrap().to_owned())
        .collect();
    assert_eq!(
        names,
        [
            "00001-0.m4s",
            "00002-4000.m4s",
            "00003-8000.m4s",
            "00004-12000.m4s",
            "00005-15000.m4s",
            "00006-16000.m4s",
            "00007-17000.m4s",
            "00008-18000.m4s",
            "00009-19000.m4s",
        ]
    );

    // Sound: the better one is picked unless the user chooses; 20 s of 4 s pieces.
    let sound = manifest.default_audio(best).unwrap();
    assert_eq!(sound.name, "English");
    assert_eq!(sound.language.as_deref(), Some("en"));
    assert_eq!(sound.bandwidth, Some(128_000));
    let sound = track(&sound.source);
    assert_eq!(
        sound.init.as_ref().unwrap().url.as_str(),
        "https://cdn.example.com/films/one/media/a/init-128000.mp4"
    );
    assert_eq!(sound.pieces.len(), 5);
    assert!(urls(sound)[0].ends_with("/a/0.m4s") && urls(sound)[4].ends_with("/a/4.m4s"));
    assert_eq!(manifest.audio_for(&manifest.qualities[1]).count(), 2);
}

#[test]
fn template_settings_are_inherited_and_overridden() {
    let text = mpd(r#"<Period duration="PT10S">
    <AdaptationSet mimeType="video/webm">
      <SegmentTemplate timescale="1" duration="4" media="$RepresentationID$-$Number$.webm" initialization="$RepresentationID$-init.webm"/>
      <Representation id="low" bandwidth="1">
        <SegmentTemplate media="other/$Number$.webm" startNumber="10"/>
      </Representation>
    </AdaptationSet>
  </Period>"#);
    let manifest = read(&text).unwrap();
    assert_eq!(manifest.duration, Some(10.0), "the period's own length");
    let low = track(&manifest.qualities[0].source);
    assert_eq!(low.container, Container::WebM);
    assert!(
        low.init
            .as_ref()
            .unwrap()
            .url
            .as_str()
            .ends_with("/low-init.webm")
    );
    let names: Vec<_> = urls(low)
        .iter()
        .map(|u| u.rsplit("one/").next().unwrap().to_owned())
        .collect();
    assert_eq!(names, ["other/10.webm", "other/11.webm", "other/12.webm"]);
    assert_eq!(manifest.audio.len(), 0);
    assert_eq!(
        manifest.qualities[0].audio_group, None,
        "nothing separate to play with"
    );
}

#[test]
fn segment_lists_and_single_files() {
    let text = mpd(r#"<Period>
    <AdaptationSet mimeType="video/mp4">
      <Representation id="list" bandwidth="2000" height="720">
        <BaseURL>https://media.example.net/list.mp4</BaseURL>
        <SegmentList>
          <Initialization range="0-799"/>
          <SegmentURL mediaRange="800-1799"/>
          <SegmentURL media="part2.mp4" mediaRange="0-99"/>
        </SegmentList>
      </Representation>
      <Representation id="single" bandwidth="1000" height="480">
        <BaseURL>single.mp4</BaseURL>
        <SegmentBase indexRange="700-899"><Initialization range="0-699"/></SegmentBase>
      </Representation>
    </AdaptationSet>
  </Period>"#);
    let manifest = read(&text).unwrap();
    let list = track(&manifest.qualities[0].source);
    let init = list.init.as_ref().unwrap();
    assert_eq!(init.url.as_str(), "https://media.example.net/list.mp4");
    assert_eq!(init.range, Some(ByteRange { start: 0, end: 799 }));
    let pieces: Vec<_> = list
        .pieces
        .iter()
        .map(|p| (p.url.as_str(), p.range))
        .collect();
    assert_eq!(
        pieces,
        [
            (
                "https://media.example.net/list.mp4",
                Some(ByteRange {
                    start: 800,
                    end: 1799
                })
            ),
            (
                "https://media.example.net/part2.mp4",
                Some(ByteRange { start: 0, end: 99 })
            ),
        ]
    );

    // One file that holds everything downloads as one piece.
    let single = track(&manifest.qualities[1].source);
    assert_eq!(single.init, None);
    assert_eq!(
        urls(single),
        ["https://cdn.example.com/films/one/single.mp4"]
    );
    assert_eq!(single.pieces[0].range, None);
}

#[test]
fn sound_only_streams_offer_their_sound_as_qualities() {
    let text = mpd(r#"<Period>
    <AdaptationSet contentType="audio" mimeType="audio/mp4">
      <Representation id="lo" bandwidth="48000"><BaseURL>lo.m4a</BaseURL></Representation>
      <Representation id="hi" bandwidth="160000"><BaseURL>hi.m4a</BaseURL></Representation>
    </AdaptationSet>
  </Period>"#);
    let manifest = read(&text).unwrap();
    let bandwidths: Vec<_> = manifest.qualities.iter().map(|q| q.bandwidth).collect();
    assert_eq!(bandwidths, [160_000, 48_000]);
    assert!(manifest.audio.is_empty());
}

#[test]
fn drm_live_and_several_parts_are_refused() {
    let protected = mpd(r#"<Period>
    <AdaptationSet mimeType="video/mp4">
      <ContentProtection schemeIdUri="urn:mpeg:dash:mp4protection:2011" value="cenc"/>
      <ContentProtection schemeIdUri="urn:uuid:edef8ba9-79d6-4ace-a3c8-27dcd51d21ed"/>
      <Representation id="v" bandwidth="1"><BaseURL>v.mp4</BaseURL></Representation>
    </AdaptationSet>
  </Period>"#);
    assert_eq!(
        read(&protected).unwrap_err(),
        MediaError::Protected("Widevine".into())
    );

    let live = mpd("").replace(r#"type="static""#, r#"type="dynamic""#);
    assert_eq!(read(&live).unwrap_err(), MediaError::Live);

    let one = r#"<Period><AdaptationSet mimeType="video/mp4"><Representation id="v" bandwidth="1"><BaseURL>v.mp4</BaseURL></Representation></AdaptationSet></Period>"#;
    assert!(matches!(
        read(&mpd(&one.repeat(2))),
        Err(MediaError::Unsupported(_))
    ));
    assert!(matches!(read(&mpd("")), Err(MediaError::Invalid(_))));
}

#[test]
fn hostile_manifests_are_cut_short() {
    let text = mpd(r#"<Period>
    <AdaptationSet mimeType="video/mp4">
      <SegmentTemplate media="$Number$.m4s" timescale="1">
        <SegmentTimeline><S t="0" d="1" r="999999999"/></SegmentTimeline>
      </SegmentTemplate>
      <Representation id="v" bandwidth="1"/>
    </AdaptationSet>
  </Period>"#);
    assert!(matches!(read(&text), Err(MediaError::Invalid(_))));
}
