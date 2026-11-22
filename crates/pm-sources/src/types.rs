//! What the providers answer, in the shapes the node's `/v1` routes send (`pm-api` re-exports
//! these in its `types`).

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SourceInfo {
    pub id: String,
    pub name: String,
    pub description: String,
    /// `ready`, or a reason it is not.
    pub status: String,
}

/// One search result of a streaming source: an album-like item or a single track.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SourceHit {
    pub id: String,
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub creator: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub year: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub downloads: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image: Option<String>,
    /// `item` (several tracks) or `track`.
    #[serde(default = "default_kind")]
    pub kind: String,
    /// Track hits: length, so a queue item knows it before the stream does.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
    /// Artist hits: the library artist with the same name, when there is one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub library_artist_id: Option<i64>,
    /// Identity the service exposes, for matching against the library.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub isrc: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub upc: Option<String>,
    /// The library track or album with the same identity, when you already have it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub library_track_id: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub library_album_id: Option<i64>,
    /// Queue URI of a track hit (`tidal:track:…`, `deezer:track:…`, a stream URL).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub uri: Option<String>,
    /// The source's own page for it.
    pub url: String,
}

fn default_kind() -> String {
    "item".into()
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SourceResults {
    pub source: String,
    pub items: Vec<SourceHit>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SourceTrack {
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub track_no: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
    /// The archive's format label (`Flac`, `VBR MP3`, …).
    pub format: String,
    /// Direct stream URL.
    pub url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub artist: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SourceItem {
    pub source: String,
    pub id: String,
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub creator: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub year: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub licence: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image: Option<String>,
    pub url: String,
    pub tracks: Vec<SourceTrack>,
    /// The library album with the same UPC, when you already have it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub library_album_id: Option<i64>,
}

/// An artist on a streaming service with their albums.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SourceArtist {
    pub source: String,
    pub id: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image: Option<String>,
    pub url: String,
    /// The artist's releases on the service, newest first, each with the type the service gives
    /// it (bounded: see [`crate::releases::MAX_RELEASES`] and `catalogue`).
    pub albums: Vec<ArtistRelease>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub library_artist_id: Option<i64>,
    /// What `albums` is out of the artist's whole catalogue on the service.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub catalogue: Option<Catalogue>,
}

/// One of an artist's releases on a service: the hit, and what the service says it is.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ArtistRelease {
    #[serde(flatten)]
    pub hit: SourceHit,
    #[serde(flatten)]
    pub release: ReleaseKind,
}

/// What a service says a release is. Only ever the service's own metadata: never guessed from
/// a title or a track count. Nothing known is `None`/`false` (the page lists it under "Other
/// releases").
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct ReleaseKind {
    /// `album`, `ep` or `single`, as the service types the release; absent when it says nothing
    /// (or something else).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub release_type: Option<String>,
    /// The release date as the service gives it (`YYYY`, `YYYY-MM` or `YYYY-MM-DD`): newest first.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub released: Option<String>,
    /// The service marks it a compilation (MusicBrainz's secondary type, Spotify's and Qobuz's
    /// compilation type).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub compilation: bool,
    /// MusicBrainz's secondary types, as it names them (`Live`, `Remix`, `Compilation`…).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub secondary_types: Vec<String>,
}

/// How much of an artist's catalogue a list holds, honestly: whether every page arrived,
/// whether the fetch stopped at its bound, the service's own count when it gives one, and why a
/// list is partial or missing.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Catalogue {
    /// The service the list comes from (`tidal`, `deezer`, `spotify`, `qobuz`, `musicbrainz`).
    pub source: String,
    /// Every page arrived: the list is the artist's whole catalogue on that service.
    pub complete: bool,
    /// The fetch stopped at its bound and the service lists more.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub truncated: bool,
    /// The service's own count of the artist's releases, when it gives one (TIDAL doesn't).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub total: Option<u64>,
    /// Why the list is partial or missing (a page failed). Such a list is never cached.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Where else a library album or track can be streamed from (Roon's "Versions").
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Version {
    pub source: String,
    /// Source item id (`album:<id>` / `track:<id>`), for `/v1/sources/{source}/items/{id}`.
    pub id: String,
    pub label: String,
    pub url: String,
    /// Queue URI when the version is a single track.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub uri: Option<String>,
    /// This instance's own item page id (`lib-1`, `tidal-1`, `u-…`), when it has one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub page: Option<String>,
}

/// An album tile on an artist's item page.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ItemAlbum {
    /// The album's page id.
    #[serde(default)]
    pub id: String,
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub year: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image: Option<String>,
    pub page: String,
    /// What the service says the release is (artist pages).
    #[serde(flatten)]
    pub release: ReleaseKind,
}

/// What a check of a client or app answered (Settings' Check buttons).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CheckResult {
    pub ok: bool,
    pub detail: String,
}
