//! The zone's queue items are the page's URLs: each opens through [`RangeReader`] and decodes
//! with pm-audio (FLAC, ALAC, WAV, AIFF, MP3, AAC/MP4, Ogg Vorbis); the chain to the output is
//! planned as the node plans it for the phone's zone ([`pm_audio::Plan::negotiate_for`] against
//! what the output takes for that source). What can't be decoded here (a playlist, HLS or DASH,
//! DRM, another codec) is refused when it is opened, so the page plays it itself.

use crate::range::RangeReader;
use pm_audio::{PipelineConfig, Plan};
use pm_model::{AudioBlock, AudioFormat, OutputCaps};
use pm_zones::{FrameSource, OutputId, Processor, QueueItem, SourceError, SourceFactory, StreamInfo};
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Duration;

/// What a refusal's message starts with: the URL opened, but it is something the app can't play
/// itself (a playlist, a web page, a codec it doesn't decode). Anything else (the node busy, the
/// network gone) may pass, and the page tries again as with `<audio>`.
pub const CANT_PLAY_HERE: &str = "the app can't play this itself: ";

/// Whether an open's error is a refusal ([`CANT_PLAY_HERE`]).
pub fn is_refusal(message: &str) -> bool {
    message.contains(CANT_PLAY_HERE)
}

/// The container behind a content type, for symphonia's probe; `Err` for what is never audio
/// it can decode (a playlist or a web page).
pub fn ext_for(content_type: &str, url: &str) -> Result<Option<&'static str>, String> {
    let ct = content_type.to_ascii_lowercase();
    let by_type = match ct.as_str() {
        "audio/flac" | "audio/x-flac" => Some("flac"),
        "audio/mp4" | "audio/x-m4a" | "audio/m4a" | "audio/aac" | "audio/aacp" | "video/mp4" => Some("m4a"),
        "audio/mpeg" | "audio/mp3" => Some("mp3"),
        "audio/ogg" | "application/ogg" | "audio/vorbis" => Some("ogg"),
        "audio/wav" | "audio/x-wav" | "audio/wave" | "audio/vnd.wave" => Some("wav"),
        "audio/aiff" | "audio/x-aiff" => Some("aiff"),
        "application/vnd.apple.mpegurl"
        | "application/x-mpegurl"
        | "audio/mpegurl"
        | "audio/x-mpegurl"
        | "application/dash+xml"
        | "audio/x-scpls" => {
            return Err(format!("{ct} is a playlist, not one file"));
        }
        t if t.starts_with("text/") || t == "application/json" => {
            return Err(format!("the node sent {t} instead of audio"));
        }
        _ => None,
    };
    if by_type.is_some() {
        return Ok(by_type);
    }
    let path = url.split(['?', '#']).next().unwrap_or("");
    let ext = path
        .rsplit('/')
        .next()
        .and_then(|f| f.rsplit_once('.'))
        .map(|(_, e)| e.to_ascii_lowercase());
    Ok(match ext.as_deref() {
        Some("flac") => Some("flac"),
        Some("m4a" | "mp4" | "alac" | "aac") => Some("m4a"),
        Some("mp3") => Some("mp3"),
        Some("ogg" | "oga") => Some("ogg"),
        Some("wav" | "wave") => Some("wav"),
        Some("aif" | "aiff") => Some("aiff"),
        Some("m3u" | "m3u8" | "pls" | "mpd") => return Err("a playlist, not one file".into()),
        _ => None,
    })
}

/// One decoded URL.
pub struct UrlSource {
    inner: pm_audio::play::Source,
    info: StreamInfo,
}

impl FrameSource for UrlSource {
    fn format(&self) -> AudioFormat {
        self.inner.format()
    }
    fn duration_ms(&self) -> Option<u64> {
        self.inner.duration_ms()
    }
    fn next_block(&mut self) -> Result<Option<AudioBlock>, SourceError> {
        FrameSource::next_block(&mut self.inner)
    }
    fn seek_ms(&mut self, ms: u64) -> Result<(), SourceError> {
        FrameSource::seek_ms(&mut self.inner, ms)
    }
    fn finalize(&mut self) -> Option<bool> {
        FrameSource::finalize(&mut self.inner)
    }
    fn lossy_source(&self) -> bool {
        self.inner.is_lossy()
    }
    fn stream_info(&self) -> Option<StreamInfo> {
        Some(self.info.clone())
    }
}

/// Opens URLs (or files kept on this device, see [`UrlSources::set_local`]) and plans them.
pub struct UrlSources {
    client: reqwest::blocking::Client,
    cfg: PipelineConfig,
    /// Opened ahead by the page's call (its first bytes checked), taken by the zone's open.
    ready: Mutex<HashMap<String, Box<dyn FrameSource>>>,
    /// A copy of the URL's file on this device, when there is one.
    local: Mutex<Option<LocalCopy>>,
    /// The URLs last opened from the copy on this device (the page says "From this phone").
    from_device: Mutex<std::collections::HashSet<String>>,
}

/// Where a URL's file is kept on this device, if anywhere.
pub type LocalCopy = Box<dyn Fn(&str) -> Option<std::path::PathBuf> + Send + Sync>;

impl Default for UrlSources {
    fn default() -> Self {
        Self::new()
    }
}

impl UrlSources {
    pub fn new() -> Self {
        let client = reqwest::blocking::Client::builder()
            .connect_timeout(Duration::from_secs(10))
            // Each read of the body (the blocking client times every read, not the whole song)
            .timeout(Duration::from_secs(30))
            .build()
            .unwrap_or_else(|_| reqwest::blocking::Client::new());
        Self {
            client,
            // What the node does for a zone without DSP: plays the source as it is where the
            // output takes it, else converts (and the path says so)
            cfg: PipelineConfig::default(),
            ready: Mutex::default(),
            local: Mutex::default(),
            from_device: Mutex::default(),
        }
    }

    /// Files kept on this device play from there instead of the network.
    pub fn set_local(&self, f: Option<LocalCopy>) {
        *self.local.lock().unwrap_or_else(|e| e.into_inner()) = f;
    }

    /// The file kept on this device for `url`, if any.
    pub fn local_copy(&self, url: &str) -> Option<std::path::PathBuf> {
        self.local
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
            .and_then(|f| f(url))
    }

    fn mark(&self, url: &str, here: bool) {
        let mut m = self.from_device.lock().unwrap_or_else(|e| e.into_inner());
        // (only the songs of now matter: what plays and what comes next)
        if m.len() > 8 {
            m.clear();
        }
        if here {
            m.insert(url.to_string());
        } else {
            m.remove(url);
        }
    }

    /// Whether `url` last opened from the copy on this device.
    pub fn from_device(&self, url: &str) -> bool {
        self.from_device
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .contains(url)
    }

    /// Open and probe `url` now; `Err` says why it can't play here (the page then plays it).
    pub fn open_url(&self, url: &str) -> Result<Box<dyn FrameSource>, String> {
        if let Some(path) = self.local_copy(url) {
            match pm_audio::zone::FileSource::open(&path) {
                Ok(f) => {
                    // The library's file, as the node sent it: only where it plays from differs
                    let info = FrameSource::stream_info(&f).unwrap_or_default();
                    self.mark(url, true);
                    return Ok(Box::new(Kept { inner: f, info }));
                }
                Err(e) => {
                    tracing::warn!(path = %path.display(), "the copy on this device: {e}; from the network")
                }
            }
        }
        self.mark(url, false);
        let reader = RangeReader::open(&self.client, url)?;
        let ext = ext_for(reader.content_type(), url).map_err(|why| format!("{CANT_PLAY_HERE}{why}"))?;
        let bytes = reader.len();
        let inner =
            pm_audio::play::Source::from_media_source(Box::new(reader), ext, "the stream").map_err(|e| {
                match e {
                    // The link broke while the start was read: no refusal, it may pass
                    pm_audio::AudioError::Io(io) if io.kind() != std::io::ErrorKind::UnexpectedEof => {
                        format!("the stream broke off: {io}")
                    }
                    e => format!("{CANT_PLAY_HERE}{e}"),
                }
            })?;
        let mut info = inner.stream_info();
        if inner.is_lossy() {
            info.bitrate_kbps = match (bytes, inner.duration_ms()) {
                (Some(b), Some(ms)) if ms > 0 => Some((b * 8 / ms) as u32),
                _ => None,
            };
        }
        Ok(Box::new(UrlSource { inner, info }))
    }

    /// Keep `source` (opened for `url`) for the zone's open of the same URL, in place of any
    /// other kept one.
    pub fn ready(&self, url: &str, source: Box<dyn FrameSource>) {
        let mut ready = self.ready.lock().unwrap_or_else(|e| e.into_inner());
        ready.clear();
        ready.insert(url.to_string(), source);
    }
}

/// A file kept on this device, labelled as such.
struct Kept {
    inner: pm_audio::zone::FileSource,
    info: StreamInfo,
}

impl FrameSource for Kept {
    fn format(&self) -> AudioFormat {
        self.inner.format()
    }
    fn duration_ms(&self) -> Option<u64> {
        self.inner.duration_ms()
    }
    fn next_block(&mut self) -> Result<Option<AudioBlock>, SourceError> {
        self.inner.next_block()
    }
    fn seek_ms(&mut self, ms: u64) -> Result<(), SourceError> {
        self.inner.seek_ms(ms)
    }
    fn finalize(&mut self) -> Option<bool> {
        self.inner.finalize()
    }
    fn lossy_source(&self) -> bool {
        self.inner.lossy_source()
    }
    fn stream_info(&self) -> Option<StreamInfo> {
        Some(self.info.clone())
    }
}

impl SourceFactory for UrlSources {
    fn open(&self, item: &QueueItem) -> Result<Box<dyn FrameSource>, SourceError> {
        let ready = self
            .ready
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&item.uri);
        match ready {
            Some(s) => Ok(s),
            None => self.open_url(&item.uri).map_err(|reason| SourceError::Open {
                uri: "the stream".into(),
                reason,
            }),
        }
    }

    fn plan(
        &self,
        src: &dyn FrameSource,
        caps: &OutputCaps,
        _output: &OutputId,
    ) -> Result<Box<dyn Processor>, SourceError> {
        let source = src.format();
        let mut plan = Plan::negotiate_for(source, caps, &self.cfg, src.replay_gain()).map_err(|e| {
            SourceError::Negotiate {
                format: source,
                reason: e.to_string(),
            }
        })?;
        if src.lossy_source() {
            plan.mark_lossy_source();
        }
        Ok(Box::new(plan))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn content_types_and_names_pick_the_container_and_refuse_playlists() {
        assert_eq!(ext_for("audio/flac", "/x"), Ok(Some("flac")));
        assert_eq!(
            ext_for("audio/mp4", "/v1/sources/tidal/stream/1"),
            Ok(Some("m4a"))
        );
        assert_eq!(
            ext_for("application/octet-stream", "/a/b.FLAC?token=x"),
            Ok(Some("flac"))
        );
        assert_eq!(ext_for("", "/v1/library/tracks/3/file"), Ok(None));
        assert!(ext_for("application/vnd.apple.mpegurl", "/x").is_err());
        assert!(ext_for("application/dash+xml", "/x").is_err());
        assert!(ext_for("", "/live/radio.m3u8").is_err());
        let html = ext_for("text/html", "/x").unwrap_err();
        assert!(html.contains("text/html instead of audio"), "{html}");
    }
}
