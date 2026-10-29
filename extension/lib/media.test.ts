// SPDX-License-Identifier: GPL-3.0-or-later

import { describe, expect, it } from 'vitest';
import {
  addFound,
  applyInspection,
  downloadable,
  forFrame,
  inspectPlaylist,
  MAX_PER_TAB,
  mediaName,
  qualityLabel,
  shown,
  sizeLabel,
  streamKind,
  youtubeVideo,
  type FoundMedia,
} from './media';

const BASE = 'https://cdn.example.com/show/master.m3u8';

describe('streamKind', () => {
  it('knows playlists by type, then by name', () => {
    expect(streamKind('https://x/a', 'application/vnd.apple.mpegurl')).toBe('hls');
    expect(streamKind('https://x/a', 'Application/X-MpegURL; charset=utf-8')).toBe('hls');
    expect(streamKind('https://x/a', 'application/dash+xml')).toBe('dash');
    expect(streamKind('https://x/v/index.M3U8?token=1', 'text/plain')).toBe('hls');
    expect(streamKind('https://x/v/manifest.mpd', undefined)).toBe('dash');
    expect(streamKind('https://x/v/seg1.ts', 'video/mp2t')).toBeUndefined();
    expect(streamKind('https://x/v/clip.mp4', 'video/mp4')).toBeUndefined();
    expect(streamKind('not a url', undefined)).toBeUndefined();
  });
});

describe('inspectPlaylist', () => {
  it('finds the qualities and sound of a master playlist', () => {
    const text = `#EXTM3U
#EXT-X-MEDIA:TYPE=AUDIO,GROUP-ID="a",NAME="English",URI="audio/en.m3u8"
#EXT-X-STREAM-INF:BANDWIDTH=800000,RESOLUTION=640x360
low/index.m3u8
#EXT-X-STREAM-INF:BANDWIDTH=3000000
https://other.example.com/high.m3u8
#EXT-X-I-FRAME-STREAM-INF:BANDWIDTH=86000,URI="iframes.m3u8"
`;
    expect(inspectPlaylist('hls', text, BASE)).toEqual({
      state: 'ready',
      parts: [
        'https://cdn.example.com/show/audio/en.m3u8',
        'https://cdn.example.com/show/low/index.m3u8',
        'https://other.example.com/high.m3u8',
        'https://cdn.example.com/show/iframes.m3u8',
      ],
    });
  });

  it('tells finished, live and locked playlists apart', () => {
    const media = (extra: string) => `#EXTM3U\n#EXT-X-TARGETDURATION:4\n${extra}#EXTINF:4,\na.ts\n`;
    expect(inspectPlaylist('hls', `${media('')}#EXT-X-ENDLIST\n`, BASE).state).toBe('ready');
    expect(inspectPlaylist('hls', media(''), BASE).state).toBe('live');
    expect(inspectPlaylist('hls', media('#EXT-X-PLAYLIST-TYPE:VOD\n'), BASE).state).toBe('ready');
    // Plain AES-128 can be unlocked; DRM can't.
    const aes = media('#EXT-X-KEY:METHOD=AES-128,URI="k"\n') + '#EXT-X-ENDLIST\n';
    expect(inspectPlaylist('hls', aes, BASE).state).toBe('ready');
    const off = media('#EXT-X-KEY:METHOD=NONE\n') + '#EXT-X-ENDLIST\n';
    expect(inspectPlaylist('hls', off, BASE).state).toBe('ready');
    for (const key of [
      '#EXT-X-KEY:METHOD=SAMPLE-AES,URI="skd://k",KEYFORMAT="com.apple.streamingkeydelivery"',
      '#EXT-X-KEY:METHOD=AES-128,URI="https://lic",' +
        'KEYFORMAT="urn:uuid:edef8ba9-79d6-4ace-a3c8-27dcd51d21ed"',
    ]) {
      const locked = `${media(`${key}\n`)}#EXT-X-ENDLIST\n`;
      expect(inspectPlaylist('hls', locked, BASE).state).toBe('protected');
    }
    const session =
      '#EXTM3U\n#EXT-X-SESSION-KEY:METHOD=SAMPLE-AES,URI="skd://k"\n' +
      '#EXT-X-STREAM-INF:BANDWIDTH=1\na.m3u8\n';
    expect(inspectPlaylist('hls', session, BASE).state).toBe('protected');
  });

  it('drops what is not a video playlist', () => {
    expect(inspectPlaylist('hls', '<html>Not found</html>', BASE).state).toBeUndefined();
    // An internet radio list.
    const radio = '#EXTM3U\n#EXTINF:-1,Radio\nhttp://radio/stream\n';
    expect(inspectPlaylist('hls', radio, BASE).state).toBeUndefined();
  });

  it('reads DASH manifests', () => {
    const mpd = (attributes: string, body = '') =>
      '<?xml version="1.0"?>' +
      `<MPD xmlns="urn:mpeg:dash:schema:mpd:2011" ${attributes}><Period>${body}</Period></MPD>`;
    expect(inspectPlaylist('dash', mpd('type="static"'), BASE).state).toBe('ready');
    expect(inspectPlaylist('dash', mpd('type="dynamic"'), BASE).state).toBe('live');
    const drm = mpd(
      'type="static"',
      '<AdaptationSet><ContentProtection schemeIdUri="urn:mpeg:dash:mp4protection:2011"/></AdaptationSet>',
    );
    expect(inspectPlaylist('dash', drm, BASE).state).toBe('protected');
    expect(inspectPlaylist('dash', '{"not": "xml"}', BASE).state).toBeUndefined();
  });
});

describe('the list of a tab', () => {
  const master = 'https://cdn.example.com/show/master.m3u8';
  const low = 'https://cdn.example.com/show/low/index.m3u8';
  const ready = { state: 'ready' as const, parts: [low] };

  it('shows a video once, whichever playlist is seen first', () => {
    // The master first, then one of its qualities.
    let list: FoundMedia[] = addFound([], master, 'hls');
    list = applyInspection(list, master, ready);
    list = addFound(list, low, 'hls');
    expect(shown(list).map((m) => m.url)).toEqual([master]);

    // A quality first (before the master was read).
    let other: FoundMedia[] = addFound(addFound([], master, 'hls'), low, 'hls');
    expect(shown(other)).toHaveLength(2);
    other = applyInspection(other, master, ready);
    expect(shown(other).map((m) => m.url)).toEqual([master]);
  });

  it('counts what can be downloaded', () => {
    let list: FoundMedia[] = addFound(addFound([], master, 'hls'), 'https://x/live.m3u8', 'hls');
    expect(downloadable(list)).toBe(0);
    list = applyInspection(list, master, ready);
    list = applyInspection(list, 'https://x/live.m3u8', { state: 'live', parts: [] });
    expect(downloadable(list)).toBe(1);
    // Seen again: nothing changes.
    expect(addFound(list, master, 'hls')).toBe(list);
    // Not a video after all: dropped.
    expect(applyInspection(list, 'https://x/live.m3u8', { state: undefined, parts: [] })).toHaveLength(1);
  });

  it('keeps only the latest videos', () => {
    let list: FoundMedia[] = [];
    for (let n = 0; n < MAX_PER_TAB + 5; n++) list = addFound(list, `https://x/${n}.m3u8`, 'hls');
    expect(list).toHaveLength(MAX_PER_TAB);
    expect(list[0]?.url).toBe('https://x/5.m3u8');
  });
});

describe('mediaName', () => {
  it('uses the folder when the name says nothing', () => {
    expect(mediaName('https://x/shows/cats.m3u8?t=1')).toBe('cats.m3u8');
    expect(mediaName('https://x/films/one/manifest.mpd')).toBe('one/manifest.mpd');
    expect(mediaName('https://x/master.m3u8')).toBe('master.m3u8');
  });
});

describe('the menu on a video', () => {
  it('offers the videos of its own frame, else the whole tab\'s', () => {
    const ready = { state: 'ready' as const, parts: [] };
    let list: FoundMedia[] = addFound([], 'https://x/top.m3u8', 'hls', 0);
    list = addFound(list, 'https://x/embed.m3u8', 'hls', 7);
    list = applyInspection(applyInspection(list, 'https://x/top.m3u8', ready), 'https://x/embed.m3u8', ready);
    expect(forFrame(list, 7).map((m) => m.url)).toEqual(['https://x/embed.m3u8']);
    expect(forFrame(list, 3)).toHaveLength(2);
    list = applyInspection(list, 'https://x/top.m3u8', { state: 'live', parts: [] });
    expect(forFrame(list, 0).map((m) => m.url)).toEqual(['https://x/embed.m3u8']);
  });

  it('names qualities and sizes', () => {
    const fullHd = { height: 1080, width: 1920, bandwidth: 1, audio: [], frameRate: 60 };
    expect(qualityLabel(fullHd)).toBe('1080p60');
    expect(qualityLabel({ height: 720, bandwidth: 1, audio: [], frameRate: 29.97 })).toBe('720p');
    expect(qualityLabel({ bandwidth: 128_000, audio: [] })).toBe('Sound only');
    expect(sizeLabel(470 * 1024 * 1024)).toBe('~470 MB');
    expect(sizeLabel(1.25 * 1024 ** 3)).toBe('~1.3 GB');
  });
});

describe('youtubeVideo', () => {
  it('knows every kind of YouTube link', () => {
    const watch = 'https://www.youtube.com/watch?v=dQw4w9WgXcQ';
    expect(youtubeVideo('https://www.youtube.com/watch?v=dQw4w9WgXcQ&t=42s')).toBe(watch);
    expect(youtubeVideo('https://m.youtube.com/watch?feature=share&v=dQw4w9WgXcQ')).toBe(watch);
    expect(youtubeVideo('https://youtu.be/dQw4w9WgXcQ?si=x')).toBe(watch);
    expect(youtubeVideo('https://www.youtube.com/shorts/dQw4w9WgXcQ')).toBe(watch);
    expect(youtubeVideo('https://www.youtube-nocookie.com/embed/dQw4w9WgXcQ?autoplay=1')).toBe(watch);
    expect(mediaName(watch)).toBe('YouTube video');
    for (const not of [
      'https://www.youtube.com/',
      'https://www.youtube.com/@someone/videos',
      'https://www.youtube.com/watch?v=tooShort',
      'https://youtube.com.example.net/watch?v=dQw4w9WgXcQ',
    ]) {
      expect(youtubeVideo(not)).toBeUndefined();
    }
  });
});
