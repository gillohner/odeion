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
}
