//! Search answers in the node's wire shape: one line per source (`SourceAnswer`, NDJSON), each
//! with its hits (`SearchHit`). The phone answers the same lines from its own sources.

use crate::types::SourceHit;
use serde::{Deserialize, Serialize};

/// One result, whatever the source.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct SearchHit {
    /// `song`, `album`, `artist`, `playlist`, `podcast`, `concert`, `genre`.
    pub kind: String,
    pub title: String,
    /// Artist (songs, albums), date and place (concerts).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subtitle: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image: Option<String>,
    /// The item page id (`lib-<n>`, `mb-…`, `isrc-…`, `u-…`) or the view arg.
    pub id: String,
    /// The service page or canonical URL.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub page: Option<String>,
    /// Queue URI when it streams.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub uri: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub track_id: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub artist_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub album: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub album_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub year: Option<i32>,
    /// Identity for merging rows across sources: `g:<pinakes group>`,
    /// `isrc:<ISRC>`, `upc:<UPC>`, `mb:<mbid>`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub keys: Vec<String>,
    /// Found through a Pubky tag: the label (the row says "tagged #chill").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tagged: Option<String>,
    /// A library file's format in words: `FLAC 16/44.1`, `MP3 320 kbps`, `DSD64`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub format: Option<String>,
    /// `hi_res`, `lossless` or `lossy` (library files); a facet.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quality: Option<String>,
    /// The site a Pubky card is from (`tidal.com`, `open.spotify.com`); a facet and a badge.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub site: Option<String>,
    /// Found by an exact identifier or a pasted link (an ISRC, a barcode, an MBID, a service
    /// page): the result, not one of several matches.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub exact: bool,
    /// The other sites the same thing is on (an exact hit's group): "Also on".
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub also: Vec<String>,
}

/// One source's answer (one NDJSON line).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct SourceAnswer {
    /// `library`, `pubky`, `tidal`, `deezer`, `spotify`, …
    pub source: String,
    /// `ok`, `error` or `timeout`.
    pub status: String,
    pub ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(default)]
    pub hits: Vec<SearchHit>,
    /// Counts by kind over the whole match (Pinakes's `facet=type`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub counts: Vec<(String, i64)>,
    /// Served from the five-minute cache.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub cached: bool,
}

/// A service's hit as a search row: songs, albums, artists and playlists (anything else is not
/// shown); `lib-<n>` when the library has it, else the page id of its link (`page_id`).
pub fn hit_of(h: &SourceHit, page_id: impl Fn(&str) -> String) -> Option<SearchHit> {
    let kind = match h.kind.as_str() {
        "track" => "song",
        "album" | "item" => "album",
        "artist" => "artist",
        "playlist" => "playlist",
        _ => return None,
    };
    let mut keys = Vec::new();
    if let Some(i) = &h.isrc {
        keys.push(format!("isrc:{i}"));
    }
    if let Some(u) = &h.upc {
        keys.push(format!("upc:{u}"));
    }
    let id = match (kind, h.library_track_id, h.library_album_id) {
        ("song", Some(t), _) => format!("lib-{t}"),
        ("album", _, Some(a)) => format!("lib-{a}"),
        _ => page_id(&h.url),
    };
    Some(SearchHit {
        kind: kind.into(),
        title: h.title.clone(),
        subtitle: h.creator.clone(),
        image: h.image.clone(),
        id,
        page: Some(h.url.clone()),
        uri: h.uri.clone(),
        track_id: h.library_track_id,
        duration_ms: h.duration_ms,
        year: h.year,
        keys,
        ..Default::default()
    })
}
