//! Streams served in segments (DASH, HLS): an initialization segment, then media segments read
//! back to back as one byte stream. The decoder can't seek in that stream (it only holds what it
//! has read), so a seek reopens it at the segment that holds the position and drops the frames
//! before it (TASK-seek-segmented-streams-1104).

use std::collections::VecDeque;
use std::io::Read;

/// A segmented stream: where its segments are and when each starts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Segments {
    /// The initialization segment (codec setup), read before any media segment.
    pub init: Option<String>,
    /// The media segments in play order.
    pub media: Vec<String>,
    /// Where each media segment starts, in ms from the first one; `None` when the manifest
    /// doesn't say (then the stream can't seek).
    pub starts_ms: Option<Vec<u64>>,
    /// Where the last segment ends: the track's length, when known.
    pub end_ms: Option<u64>,
}

impl Segments {
    /// The init segment, then the media segments from `first` on.
    pub fn urls_from(&self, first: usize) -> Vec<String> {
        self.init
            .iter()
            .chain(self.media.iter().skip(first))
            .cloned()
            .collect()
    }

    /// Every URL in play order.
    pub fn urls(&self) -> Vec<String> {
        self.urls_from(0)
    }

    /// The segment that holds `ms`, with its start. `None` when the stream has no times.
    pub fn at(&self, ms: u64) -> Option<(usize, u64)> {
        let starts = self.starts_ms.as_ref()?;
        let k = starts.partition_point(|s| *s <= ms).saturating_sub(1);
        Some((k, starts.get(k).copied().unwrap_or(0)))
    }
}

/// The segments, read one after another as one stream (blocking; for the audio thread).
pub struct SegmentReader {
    client: &'static reqwest::blocking::Client,
    urls: VecDeque<String>,
    current: Option<reqwest::blocking::Response>,
}

impl SegmentReader {
    pub fn new(client: &'static reqwest::blocking::Client, urls: Vec<String>) -> Self {
        Self {
            client,
            urls: urls.into(),
            current: None,
        }
    }

    /// How many segments are still to be read (the one being read not counted).
    pub fn remaining(&self) -> usize {
        self.urls.len()
    }
}

impl Read for SegmentReader {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        loop {
            if let Some(r) = self.current.as_mut() {
                let n = r.read(buf)?;
                if n > 0 {
                    return Ok(n);
                }
                self.current = None;
            }
            let Some(url) = self.urls.pop_front() else {
                return Ok(0);
            };
            let resp = self
                .client
                .get(&url)
                .send()
                .and_then(|r| r.error_for_status())
                .map_err(|e| std::io::Error::other(format!("segment: {}", e.without_url())))?;
            self.current = Some(resp);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_position_finds_its_segment() {
        let s = Segments {
            init: Some("init".into()),
            media: vec!["1".into(), "2".into(), "3".into()],
            starts_ms: Some(vec![0, 4000, 8000]),
            end_ms: Some(10_000),
        };
        assert_eq!(s.at(0), Some((0, 0)));
        assert_eq!(s.at(3999), Some((0, 0)));
        assert_eq!(s.at(4000), Some((1, 4000)));
        assert_eq!(s.at(9_500), Some((2, 8000)));
        assert_eq!(s.at(60_000), Some((2, 8000)), "past the end: the last segment");
        assert_eq!(s.urls_from(1), vec!["init", "2", "3"]);
        let untimed = Segments { starts_ms: None, ..s };
        assert_eq!(untimed.at(5000), None);
    }

    // ---- a segmented stream end to end: fragmented MP4 (PCM) served over HTTP ----

    const RATE: u32 = 8000;
    /// Frames per segment (one second).
    const SEG: u32 = RATE;
    const SEGMENTS: u32 = 4;

    fn mp4_box(kind: &[u8; 4], body: &[u8]) -> Vec<u8> {
        let mut b = ((8 + body.len()) as u32).to_be_bytes().to_vec();
        b.extend_from_slice(kind);
        b.extend_from_slice(body);
        b
    }

    fn full(kind: &[u8; 4], version: u8, flags: u32, body: &[u8]) -> Vec<u8> {
        let mut v = vec![version];
        v.extend_from_slice(&flags.to_be_bytes()[1..]);
        v.extend_from_slice(body);
        mp4_box(kind, &v)
    }

    fn cat(parts: &[Vec<u8>]) -> Vec<u8> {
        parts.concat()
    }

    /// The left channel of frame `f` says where it is (`f / 4`), the right its negative.
    fn sample(f: u32) -> i16 {
        (f / 4) as i16
    }

    /// `ftyp` + `moov` for one stereo 16-bit PCM (`sowt`) track at `RATE`, fragmented.
    fn init_segment() -> Vec<u8> {
        let matrix: Vec<u8> = [0x0001_0000u32, 0, 0, 0, 0x0001_0000, 0, 0, 0, 0x4000_0000]
            .iter()
            .flat_map(|v| v.to_be_bytes())
            .collect();
        let ftyp = mp4_box(b"ftyp", b"isom\0\0\0\0isomiso6dash");
        let mut mvhd = vec![0u8; 8];
        mvhd.extend_from_slice(&RATE.to_be_bytes());
        mvhd.extend_from_slice(&0u32.to_be_bytes());
        mvhd.extend_from_slice(&0x0001_0000u32.to_be_bytes());
        mvhd.extend_from_slice(&0x0100u16.to_be_bytes());
        mvhd.extend_from_slice(&[0u8; 10]);
        mvhd.extend_from_slice(&matrix);
        mvhd.extend_from_slice(&[0u8; 24]);
        mvhd.extend_from_slice(&2u32.to_be_bytes());
        let mvhd = full(b"mvhd", 0, 0, &mvhd);
        let mut tkhd = vec![0u8; 8];
        tkhd.extend_from_slice(&1u32.to_be_bytes());
        tkhd.extend_from_slice(&[0u8; 4]);
        tkhd.extend_from_slice(&0u32.to_be_bytes());
        tkhd.extend_from_slice(&[0u8; 8]);
        tkhd.extend_from_slice(&[0, 0, 0, 0]);
        tkhd.extend_from_slice(&0x0100u16.to_be_bytes());
        tkhd.extend_from_slice(&[0, 0]);
        tkhd.extend_from_slice(&matrix);
        tkhd.extend_from_slice(&[0u8; 8]);
        let tkhd = full(b"tkhd", 0, 7, &tkhd);
        let mut mdhd = vec![0u8; 8];
        mdhd.extend_from_slice(&RATE.to_be_bytes());
        mdhd.extend_from_slice(&0u32.to_be_bytes());
        mdhd.extend_from_slice(&0x55c4u16.to_be_bytes());
        mdhd.extend_from_slice(&[0, 0]);
        let mdhd = full(b"mdhd", 0, 0, &mdhd);
        let hdlr = full(
            b"hdlr",
            0,
            0,
            &cat(&[vec![0u8; 4], b"soun".to_vec(), vec![0u8; 12], b"sound\0".to_vec()]),
        );
        let smhd = full(b"smhd", 0, 0, &[0u8; 4]);
        let dinf = mp4_box(
            b"dinf",
            &full(
                b"dref",
                0,
                0,
                &cat(&[1u32.to_be_bytes().to_vec(), full(b"url ", 0, 1, &[])]),
            ),
        );
        // A QuickTime sound description, version 0: 2 channels, 16 bits, RATE (16.16)
        let mut sowt = vec![0u8; 6];
        sowt.extend_from_slice(&1u16.to_be_bytes());
        sowt.extend_from_slice(&[0u8; 8]);
        sowt.extend_from_slice(&2u16.to_be_bytes());
        sowt.extend_from_slice(&16u16.to_be_bytes());
        sowt.extend_from_slice(&[0u8; 4]);
        sowt.extend_from_slice(&(RATE << 16).to_be_bytes());
        let stsd = full(
            b"stsd",
            0,
            0,
            &cat(&[1u32.to_be_bytes().to_vec(), mp4_box(b"sowt", &sowt)]),
        );
        let stbl = mp4_box(
            b"stbl",
            &cat(&[
                stsd,
                full(b"stts", 0, 0, &[0u8; 4]),
                full(b"stsc", 0, 0, &[0u8; 4]),
                full(b"stsz", 0, 0, &[0u8; 8]),
                full(b"stco", 0, 0, &[0u8; 4]),
            ]),
        );
        let minf = mp4_box(b"minf", &cat(&[smhd, dinf, stbl]));
        let mdia = mp4_box(b"mdia", &cat(&[mdhd, hdlr, minf]));
        let trak = mp4_box(b"trak", &cat(&[tkhd, mdia]));
        let mehd = full(b"mehd", 0, 0, &(SEG * SEGMENTS).to_be_bytes());
        let mut trex = 1u32.to_be_bytes().to_vec();
        for v in [1u32, 1, 4, 0] {
            trex.extend_from_slice(&v.to_be_bytes());
        }
        let mvex = mp4_box(b"mvex", &cat(&[mehd, full(b"trex", 0, 0, &trex)]));
        cat(&[ftyp, mp4_box(b"moov", &cat(&[mvhd, trak, mvex]))])
    }

    /// Media segment `k` (from 0): `moof` + `mdat` with frames `k * SEG ..`.
    fn media_segment(k: u32) -> Vec<u8> {
        let mfhd = full(b"mfhd", 0, 0, &(k + 1).to_be_bytes());
        // default-base-is-moof, default duration (1) and size (4 bytes a frame)
        let mut tfhd = 1u32.to_be_bytes().to_vec();
        tfhd.extend_from_slice(&1u32.to_be_bytes());
        tfhd.extend_from_slice(&4u32.to_be_bytes());
        let tfhd = full(b"tfhd", 0, 0x02_0018, &tfhd);
        let tfdt = full(b"tfdt", 1, 0, &u64::from(k * SEG).to_be_bytes());
        let traf_len = |off: u32| {
            let mut trun = SEG.to_be_bytes().to_vec();
            trun.extend_from_slice(&off.to_be_bytes());
            mp4_box(
                b"traf",
                &cat(&[tfhd.clone(), tfdt.clone(), full(b"trun", 0, 1, &trun)]),
            )
        };
        let moof_len = mp4_box(b"moof", &cat(&[mfhd.clone(), traf_len(0)])).len() as u32;
        let moof = mp4_box(b"moof", &cat(&[mfhd, traf_len(moof_len + 8)]));
        let pcm: Vec<u8> = (k * SEG..(k + 1) * SEG)
            .flat_map(|f| {
                let v = sample(f);
                [v.to_le_bytes(), (-v).to_le_bytes()].concat()
            })
            .collect();
        cat(&[moof, mp4_box(b"mdat", &pcm)])
    }

    /// Serves `files` by path on 127.0.0.1 (HTTP/1.1, one request a connection); counts requests.
    fn serve(
        files: std::collections::HashMap<String, Vec<u8>>,
    ) -> (String, std::sync::Arc<std::sync::Mutex<Vec<String>>>) {
        use std::io::Write;
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let log = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let seen = log.clone();
        std::thread::spawn(move || {
            for conn in listener.incoming() {
                let Ok(mut c) = conn else { continue };
                let mut req = Vec::new();
                let mut buf = [0u8; 1024];
                while !req.windows(4).any(|w| w == b"\r\n\r\n") {
                    match c.read(&mut buf) {
                        Ok(0) | Err(_) => break,
                        Ok(n) => req.extend_from_slice(&buf[..n]),
                    }
                }
                let line = String::from_utf8_lossy(&req)
                    .lines()
                    .next()
                    .unwrap_or("")
                    .to_string();
                let path = line.split(' ').nth(1).unwrap_or("").to_string();
                seen.lock().unwrap().push(path.clone());
                let (status, body) = match files.get(&path) {
                    Some(b) => ("200 OK", b.clone()),
                    None => ("404 Not Found", Vec::new()),
                };
                let _ = write!(
                    c,
                    "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                let _ = c.write_all(&body);
            }
        });
        (base, log)
    }

    fn served() -> (Segments, std::sync::Arc<std::sync::Mutex<Vec<String>>>) {
        let mut files = std::collections::HashMap::new();
        files.insert("/init.mp4".to_string(), init_segment());
        for k in 0..SEGMENTS {
            files.insert(format!("/{}.m4s", k + 1), media_segment(k));
        }
        let (base, log) = serve(files);
        let segs = Segments {
            init: Some(format!("{base}/init.mp4")),
            media: (1..=SEGMENTS).map(|n| format!("{base}/{n}.m4s")).collect(),
            starts_ms: Some((0..SEGMENTS).map(|k| u64::from(k) * 1000).collect()),
            end_ms: Some(u64::from(SEGMENTS) * 1000),
        };
        (segs, log)
    }

    /// The next `n` frames (left, right), whatever the blocks' sizes.
    fn take(src: &mut dyn pm_zones::FrameSource, n: usize) -> Vec<(i32, i32)> {
        let mut out = Vec::new();
        while out.len() < n {
            let Some(b) = src.next_block().unwrap() else { break };
            let pm_model::Samples::PcmI32(v) = &b.samples else {
                panic!("PCM")
            };
            out.extend(v.chunks(2).map(|f| (f[0], f[1])));
        }
        out.truncate(n);
        out
    }

    /// The decoder's scale: frame 4 holds 1 (and the right channel its negative).
    fn scale_of(src: &mut dyn pm_zones::FrameSource) -> i32 {
        let first = take(src, 8);
        let scale = first[4].0;
        assert!(scale > 0, "{first:?}");
        assert_eq!(first[0], (0, 0));
        assert_eq!(first[4].1, -scale, "right is the negative");
        scale
    }

    /// The frame the stream is at, read from the next frame's left channel (to within 4).
    fn position(src: &mut dyn pm_zones::FrameSource, scale: i32) -> u32 {
        let f = take(src, 1);
        assert_eq!(f.len(), 1, "the stream ended");
        (f[0].0 / scale) as u32 * 4
    }

    #[test]
    fn a_segmented_stream_seeks_by_segment() {
        let (segs, log) = served();
        let mut src = crate::sources::frame_source_from_segments(segs, Some("m4a"), "test:dash").unwrap();
        assert_eq!(src.format().rate_hz, RATE);
        assert_eq!(src.duration_ms(), Some(4000));
        let scale = scale_of(src.as_mut());
        // Forward past a segment: into the third one (2.5 s). The stream opens again from the
        // init segment and segment 3, never 1 or 2 (the first decoder may have read ahead)
        src.seek_ms(2500).unwrap();
        let at = position(src.as_mut(), scale);
        assert!(at.abs_diff(2 * SEG + SEG / 2) <= 4, "at frame {at}");
        let fetched = log.lock().unwrap().clone();
        let reopened = fetched.iter().rposition(|p| p == "/init.mp4").unwrap();
        assert!(reopened > 0, "opened again: {fetched:?}");
        assert_eq!(
            fetched.get(reopened + 1).map(String::as_str),
            Some("/3.m4s"),
            "{fetched:?}"
        );
        assert!(
            !fetched[reopened..].iter().any(|p| p == "/1.m4s" || p == "/2.m4s"),
            "{fetched:?}"
        );
        // Back to the start
        src.seek_ms(0).unwrap();
        assert_eq!(position(src.as_mut(), scale), 0);
        // Into the last one, then on to the end without an error
        src.seek_ms(3900).unwrap();
        let at = position(src.as_mut(), scale);
        assert!(at.abs_diff(3 * SEG + SEG * 9 / 10) <= 4, "at frame {at}");
        let mut frames = 0;
        while let Some(b) = src.next_block().unwrap() {
            frames += b.frames();
        }
        assert_eq!(frames, SEG as usize / 10 - 1, "the rest of the last segment");
        assert_eq!(src.finalize(), None, "no whole-file check after a seek");
    }

    #[test]
    fn a_stream_without_segment_times_says_it_cannot_seek() {
        let (segs, _) = served();
        let untimed = Segments {
            starts_ms: None,
            end_ms: None,
            ..segs
        };
        let mut src = crate::sources::frame_source_from_segments(untimed, Some("m4a"), "test:hls").unwrap();
        assert_eq!(take(src.as_mut(), 8).len(), 8);
        assert!(matches!(src.seek_ms(2000), Err(pm_zones::SourceError::Seek(_))));
        assert_eq!(take(src.as_mut(), 8).len(), 8, "plays on");
    }

    /// The same stream as an HLS playlist with `#EXT-X-MAP` and `#EXTINF`.
    #[test]
    fn an_hls_playlist_seeks_by_segment() {
        let (segs, _) = served();
        let base = segs
            .init
            .as_deref()
            .unwrap()
            .trim_end_matches("/init.mp4")
            .to_string();
        let mut text = "#EXTM3U\n#EXT-X-PLAYLIST-TYPE:VOD\n#EXT-X-MAP:URI=\"init.mp4\"\n".to_string();
        for n in 1..=SEGMENTS {
            text.push_str(&format!("#EXTINF:1.000,\n{n}.m4s\n"));
        }
        text.push_str("#EXT-X-ENDLIST\n");
        let crate::hls::Playlist::Media(m) = crate::hls::parse(&format!("{base}/pl.m3u8"), &text).unwrap()
        else {
            panic!("a media playlist")
        };
        let hls = m.into_segments();
        assert_eq!(hls, segs, "the same segments and times as the DASH manifest");
        let mut src = crate::sources::frame_source_from_segments(hls, Some("m4a"), "test:hls").unwrap();
        let scale = scale_of(src.as_mut());
        src.seek_ms(1250).unwrap();
        let at = position(src.as_mut(), scale);
        assert!(at.abs_diff(SEG + SEG / 4) <= 4, "at frame {at}");
    }
}
