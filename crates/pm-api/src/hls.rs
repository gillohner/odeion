//! A minimal HLS reader for on-demand audio (TIDAL stream manifests): a
//! master playlist's best variant, then the media playlist's init section
//! and segments read back to back as one byte stream. Each `#EXTINF` duration
//! is kept, so a seek can start from the segment that holds the position.
//! Live playlists, encryption and discontinuities are out of scope and refused.

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MediaPlaylist {
    /// `#EXT-X-MAP` initialisation section, fetched first.
    pub init: Option<String>,
    pub segments: Vec<String>,
    /// Each segment's `#EXTINF` duration in ms; `None` when one has none.
    pub durations_ms: Option<Vec<u64>>,
}

impl MediaPlaylist {
    /// The segments with their times (when every one has a duration), for a seekable stream.
    pub fn into_segments(self) -> crate::segmented::Segments {
        let (starts_ms, end_ms) = match self.durations_ms {
            Some(d) => {
                let mut at = 0u64;
                let starts = d
                    .iter()
                    .map(|ms| {
                        let s = at;
                        at += ms;
                        s
                    })
                    .collect();
                (Some(starts), Some(at))
            }
            None => (None, None),
        };
        crate::segmented::Segments {
            init: self.init,
            media: self.segments,
            starts_ms,
            end_ms,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Playlist {
    /// Variant URLs with their bandwidth, in playlist order.
    Master(Vec<(u64, String)>),
    Media(MediaPlaylist),
}

fn resolve(base: &str, href: &str) -> String {
    if href.starts_with("http://") || href.starts_with("https://") {
        return href.to_string();
    }
    match url::Url::parse(base).and_then(|b| b.join(href)) {
        Ok(u) => u.to_string(),
        Err(_) => href.to_string(),
    }
}

fn attr<'a>(line: &'a str, key: &str) -> Option<&'a str> {
    let rest = line.split_once(':')?.1;
    for part in rest.split(',') {
        let (k, v) = part.split_once('=')?;
        if k.trim() == key {
            return Some(v.trim().trim_matches('"'));
        }
    }
    None
}

/// Parse a playlist fetched from `base` (relative URLs resolve against it).
pub fn parse(base: &str, text: &str) -> Result<Playlist, String> {
    if !text.trim_start().starts_with("#EXTM3U") {
        return Err("not an HLS playlist".into());
    }
    let mut variants = Vec::new();
    let mut pending_bandwidth: Option<u64> = None;
    let mut init = None;
    let mut segments = Vec::new();
    // The `#EXTINF` before the next segment line, and whether every segment had one
    let (mut durations, mut pending_inf, mut all_timed) = (Vec::new(), None::<u64>, true);
    let mut is_master = false;
    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() {
            continue;
        }
        if let Some(rest) = line.strip_prefix('#') {
            if rest.starts_with("EXT-X-STREAM-INF") {
                is_master = true;
                pending_bandwidth = attr(line, "BANDWIDTH").and_then(|b| b.parse().ok());
            } else if let Some(inf) = rest.strip_prefix("EXTINF:") {
                // `#EXTINF:<seconds>,<title>`
                pending_inf = inf
                    .split(',')
                    .next()
                    .and_then(|v| v.trim().parse::<f64>().ok())
                    .filter(|v| v.is_finite() && *v >= 0.0)
                    .map(|v| (v * 1000.0).round() as u64);
            } else if rest.starts_with("EXT-X-MAP") {
                init = attr(line, "URI").map(|u| resolve(base, u));
            } else if rest.starts_with("EXT-X-KEY") {
                let method = attr(line, "METHOD").unwrap_or("NONE");
                if method != "NONE" {
                    return Err(format!("encrypted HLS stream ({method}) is not supported"));
                }
            } else if rest.starts_with("EXT-X-PLAYLIST-TYPE") && !line.contains("VOD") {
                return Err("only on-demand (VOD) playlists are supported".into());
            }
            continue;
        }
        if is_master {
            variants.push((pending_bandwidth.take().unwrap_or(0), resolve(base, line)));
        } else {
            segments.push(resolve(base, line));
            match pending_inf.take() {
                Some(ms) => durations.push(ms),
                None => all_timed = false,
            }
        }
    }
    if is_master {
        if variants.is_empty() {
            return Err("master playlist lists no variants".into());
        }
        Ok(Playlist::Master(variants))
    } else if segments.is_empty() {
        Err("media playlist lists no segments".into())
    } else {
        Ok(Playlist::Media(MediaPlaylist {
            init,
            segments,
            durations_ms: all_timed.then_some(durations),
        }))
    }
}

/// Fetch `url` (master or media playlist) down to its media playlist; the highest-bandwidth
/// variant is used.
pub fn open_segments(
    client: &'static reqwest::blocking::Client,
    url: &str,
) -> Result<crate::segmented::Segments, String> {
    let mut url = url.to_string();
    for _ in 0..2 {
        let text = client
            .get(&url)
            .send()
            .map_err(|e| e.to_string())?
            .error_for_status()
            .map_err(|e| e.to_string())?
            .text()
            .map_err(|e| e.to_string())?;
        match parse(&url, &text)? {
            Playlist::Master(mut variants) => {
                variants.sort_by_key(|(bw, _)| *bw);
                url = variants.pop().map(|(_, u)| u).ok_or("no variant")?;
            }
            Playlist::Media(m) => return Ok(m.into_segments()),
        }
    }
    Err("playlist nesting too deep".into())
}

/// Reads the init section and every segment in order as one stream.
pub struct HlsReader;

impl HlsReader {
    /// The playlist at `url` as one stream (see [`open_segments`]).
    pub fn open(
        client: &'static reqwest::blocking::Client,
        url: &str,
    ) -> Result<crate::segmented::SegmentReader, String> {
        let segs = open_segments(client, url)?;
        Ok(crate::segmented::SegmentReader::new(client, segs.urls()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn master_and_media_playlists_parse() {
        let master = "#EXTM3U\n#EXT-X-STREAM-INF:BANDWIDTH=900000,CODECS=\"flac\"\nlow/pl.m3u8\n#EXT-X-STREAM-INF:BANDWIDTH=2000000\nhttps://cdn/hi/pl.m3u8\n";
        let p = parse("https://host/a/master.m3u8", master).unwrap();
        assert_eq!(
            p,
            Playlist::Master(vec![
                (900_000, "https://host/a/low/pl.m3u8".into()),
                (2_000_000, "https://cdn/hi/pl.m3u8".into())
            ])
        );
        let media = "#EXTM3U\n#EXT-X-VERSION:7\n#EXT-X-PLAYLIST-TYPE:VOD\n#EXT-X-MAP:URI=\"init.mp4\"\n#EXTINF:4.0,\nseg0.m4s\n#EXTINF:4.0,\n/abs/seg1.m4s\n#EXT-X-ENDLIST\n";
        let p = parse("https://cdn/hi/pl.m3u8", media).unwrap();
        assert_eq!(
            p,
            Playlist::Media(MediaPlaylist {
                init: Some("https://cdn/hi/init.mp4".into()),
                segments: vec![
                    "https://cdn/hi/seg0.m4s".into(),
                    "https://cdn/abs/seg1.m4s".into()
                ],
                durations_ms: Some(vec![4000, 4000]),
            })
        );
        assert!(parse("x", "not a playlist").is_err());
        assert!(parse("x", "#EXTM3U\n#EXT-X-KEY:METHOD=AES-128,URI=\"k\"\ns.ts\n").is_err());
        assert!(parse("x", "#EXTM3U\n#EXT-X-KEY:METHOD=NONE\ns.ts\n").is_ok());
    }

    #[test]
    fn segment_durations_become_start_times() {
        let media = "#EXTM3U\n#EXT-X-PLAYLIST-TYPE:VOD\n#EXTINF:4.0,\na.mp3\n#EXTINF:3.5,title\nb.mp3\n#EXTINF:2.25,\nc.mp3\n#EXT-X-ENDLIST\n";
        let Playlist::Media(m) = parse("https://h/p.m3u8", media).unwrap() else {
            panic!("a media playlist")
        };
        let s = m.into_segments();
        assert_eq!(s.init, None);
        assert_eq!(s.starts_ms, Some(vec![0, 4000, 7500]));
        assert_eq!(s.end_ms, Some(9750));
        assert_eq!(s.at(8000), Some((2, 7500)));
        // A segment without a duration: no times, so no seeking by segment
        let Playlist::Media(m) = parse("https://h/p.m3u8", "#EXTM3U\n#EXTINF:4,\na.mp3\nb.mp3\n").unwrap()
        else {
            panic!("a media playlist")
        };
        assert_eq!(m.into_segments().starts_ms, None);
    }
}
