//! One search, scoped, that never feels stuck (owner, 2026-10-21;
//! STEWARD-UX-HOME.md "Search").
//!
//! `GET /v1/search?q=&in=` asks every source of the scope in parallel and
//! streams one NDJSON line per source as soon as it answers: the library
//! first (local, instant), then Pubky (Pinakes) and each connected service.
//! A source that fails or passes its time limit sends a line saying so; the
//! others are not held back. Every hit has the same shape, so the UI renders
//! them with one row and one tile, and the same song from several sources is
//! one row (Pinakes group, ISRC or UPC). Answers are cached five minutes per
//! query and source.

use crate::state::NodeState;
use crate::streaming::{services, StreamingService};
use axum::body::Body;
use axum::extract::{Query, State};
use axum::http::header;
use axum::response::{IntoResponse, Response};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

/// How long one source may take before it is reported as not answering.
pub const SOURCE_TIMEOUT: Duration = Duration::from_secs(8);
const CACHE_TTL: Duration = Duration::from_secs(300);

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

#[derive(Debug, Clone, Deserialize)]
pub struct SearchParams {
    #[serde(default)]
    pub q: String,
    /// `everywhere` (default), `library`, `pubky`, or a service id.
    #[serde(default, rename = "in")]
    pub scope: Option<String>,
    #[serde(default)]
    pub limit: Option<u32>,
    /// Tag labels, comma-separated: the same as `#label` words in `q` (several: all of them).
    #[serde(default)]
    pub tags: Option<String>,
}

/// `q` with the `tags` parameter folded in as `#label` words, so one grammar serves both.
pub(crate) fn query_with_tags(p: &SearchParams) -> String {
    let mut q = p.q.trim().to_string();
    for label in p.tags.iter().flat_map(|t| t.split(',')).flat_map(tag_labels) {
        q.push_str(" #");
        q.push_str(&label);
    }
    q.trim().to_string()
}

/// The sources a scope asks.
pub fn sources_for(s: &NodeState, scope: &str) -> Vec<String> {
    let svcs: Vec<String> = services()
        .iter()
        .filter(|x| s.source_enabled(x.id()) && x.unified(s))
        .map(|x| x.id().to_string())
        .collect();
    // People and posts come from Nexus, which the node reaches through its discovery setup
    let social: Vec<String> = if s.discovery().is_some() {
        vec!["people".to_string(), "posts".to_string()]
    } else {
        Vec::new()
    };
    match scope {
        "" | "everywhere" => {
            let mut v = vec!["library".to_string(), "pubky".to_string()];
            v.extend(social);
            v.extend(svcs);
            v
        }
        "pubky" => {
            let mut v = vec!["pubky".to_string()];
            v.extend(social);
            v
        }
        "library" => vec![scope.to_string()],
        other if services().iter().any(|x| x.id() == other) && s.source_enabled(other) => {
            vec![other.to_string()]
        }
        _ => vec!["library".to_string()],
    }
}

/// Answers by (query, source), with when they were fetched.
/// Each node's answers, by query and source (on `NodeState`: one node's change never empties
/// another's, as two nodes in one process, or tests running side by side, would otherwise).
pub(crate) type AnswerCache = Mutex<HashMap<(String, String), (Instant, SourceAnswer)>>;

/// Drop this node's cached answers (a source was switched on or off).
pub(crate) fn forget_answers(s: &NodeState) {
    s.search_answers.lock().expect("search cache").clear();
}

/// Drops one person's cached answers (`Who::tag`): they were removed.
pub(crate) fn forget_answers_of(s: &NodeState, tag: &str) {
    let suffix = format!("|{tag}");
    s.search_answers
        .lock()
        .expect("search cache")
        .retain(|(_, src), _| !src.ends_with(&suffix));
}

/// `GET /v1/search?q=&in=`: NDJSON, one line per source as it answers.
pub(crate) async fn search(
    State(s): State<Arc<NodeState>>,
    axum::Extension(c): axum::Extension<crate::accounts::Caller>,
    Query(p): Query<SearchParams>,
) -> Response {
    let owner = c.role == crate::accounts::Role::Owner;
    let q = query_with_tags(&p);
    let scope = p.scope.clone().unwrap_or_default();
    let limit = p.limit.unwrap_or(20).clamp(1, 50);
    let sources = if q.is_empty() {
        Vec::new()
    } else if !hash_tags(&q).is_empty() {
        // `#tag`: tags only, yours and everyone's (posts carrying the tag too, unless the scope is
        // the library)
        let mut v = vec!["library".to_string(), "pubky".to_string()];
        if s.discovery().is_some() && matches!(scope.as_str(), "" | "everywhere" | "pubky") {
            v.push("posts".to_string());
        }
        v
    } else {
        sources_for(&s, &scope)
    };
    let announced = sources.join(",");
    let (tx, rx) = tokio::sync::mpsc::channel::<SourceAnswer>(sources.len().max(1));
    for src in sources {
        let (s, q, tx) = (s.clone(), q.clone(), tx.clone());
        crate::accounts::spawn_asking(async move {
            // Each person's answers apart: their own sign-ins (and, for a listener, no owner's
            // playlists) make them differ
            let who = crate::accounts::asking();
            let key = (
                q.to_lowercase(),
                if owner {
                    src.clone()
                } else {
                    format!("{src}|{}", who.tag())
                },
            );
            let hit = s
                .search_answers
                .lock()
                .expect("search cache")
                .get(&key)
                .filter(|(at, _)| at.elapsed() < CACHE_TTL)
                .map(|(_, a)| a.clone());
            let answer = match hit {
                Some(mut a) => {
                    a.cached = true;
                    a
                }
                None => {
                    let started = Instant::now();
                    let res =
                        tokio::time::timeout(SOURCE_TIMEOUT, one_source(&s, &src, &q, limit, owner)).await;
                    let ms = started.elapsed().as_millis() as u64;
                    let a = match res {
                        Ok(Ok((hits, counts))) => SourceAnswer {
                            source: src.clone(),
                            status: "ok".into(),
                            ms,
                            hits,
                            counts,
                            ..Default::default()
                        },
                        Ok(Err(e)) => SourceAnswer {
                            source: src.clone(),
                            status: "error".into(),
                            ms,
                            error: Some(e),
                            ..Default::default()
                        },
                        Err(_) => SourceAnswer {
                            source: src.clone(),
                            status: "timeout".into(),
                            ms,
                            error: Some(format!("no answer within {} s", SOURCE_TIMEOUT.as_secs())),
                            ..Default::default()
                        },
                    };
                    if a.status == "ok" {
                        s.search_answers
                            .lock()
                            .expect("search cache")
                            .insert(key, (Instant::now(), a.clone()));
                    }
                    a
                }
            };
            let _ = tx.send(answer).await;
        });
    }
    drop(tx);
    let stream = futures_util::stream::unfold(rx, |mut rx| async move {
        let a = rx.recv().await?;
        let mut line = serde_json::to_vec(&a).unwrap_or_default();
        line.push(b'\n');
        Some((Ok::<_, std::io::Error>(bytes::Bytes::from(line)), rx))
    });
    (
        [
            (header::CONTENT_TYPE, "application/x-ndjson".to_string()),
            (header::CACHE_CONTROL, "no-cache".to_string()),
            // Which sources will answer, so the UI can show them pending at once.
            (header::HeaderName::from_static("x-pm-sources"), announced),
        ],
        Body::from_stream(stream),
    )
        .into_response()
}

type Found = (Vec<SearchHit>, Vec<(String, i64)>);

async fn one_source(
    s: &Arc<NodeState>,
    src: &str,
    q: &str,
    limit: u32,
    owner: bool,
) -> Result<Found, String> {
    match src {
        "library" => Ok((
            library(s, q, limit, owner).map_err(|e| e.to_string())?,
            Vec::new(),
        )),
        "pubky" => pubky(s, q, limit, owner).await,
        "people" => people(s, q, limit).await,
        "posts" => posts(s, q, limit).await,
        other => {
            let svc = services()
                .into_iter()
                .find(|x| x.id() == other)
                .ok_or_else(|| format!("no source {other}"))?;
            service(s, svc.as_ref(), q, limit).await
        }
    }
}

/// A library song as a search row.
pub(crate) fn song_hit(t: &pm_model::Track) -> Option<SearchHit> {
    let tid = t.id?;
    let mut keys = Vec::new();
    if let Some(i) = &t.isrc {
        keys.push(format!("isrc:{i}"));
    }
    if let Some(m) = t.recording_mbid {
        keys.push(format!("mb:{m}"));
    }
    Some(SearchHit {
        kind: "song".into(),
        title: t.title.clone(),
        subtitle: Some(
            t.artists
                .iter()
                .map(|a| a.name.as_str())
                .collect::<Vec<_>>()
                .join(", "),
        )
        .filter(|x| !x.is_empty()),
        image: t
            .album
            .as_ref()
            .and_then(|a| a.id)
            .map(|i| format!("/v1/library/albums/{i}/cover")),
        id: crate::item::track_page_id(t),
        track_id: Some(tid.0),
        artist_id: t.artists.first().and_then(|a| a.id).map(|i| format!("lib-{i}")),
        album: t.album.as_ref().map(|a| a.title.clone()),
        album_id: t.album.as_ref().and_then(|a| a.id).map(|i| format!("lib-{i}")),
        duration_ms: t.duration_ms,
        keys,
        format: Some(file_format(&t.file)),
        quality: Some(file_quality(&t.file).into()),
        ..Default::default()
    })
}

/// A library artist as a result.
pub(crate) fn artist_hit(a: &pm_library::ArtistSummary) -> SearchHit {
    SearchHit {
        kind: "artist".into(),
        title: a.name.clone(),
        subtitle: Some(format!("{} · your library", crate::plural(a.albums, "album"))),
        id: format!("lib-{}", a.id),
        keys: a.mbid.map(|m| vec![format!("mb:{m}")]).unwrap_or_default(),
        ..Default::default()
    }
}

/// A library album as a result.
pub(crate) fn album_hit(a: &pm_model::Album) -> Option<SearchHit> {
    let id = a.id?;
    Some(SearchHit {
        kind: "album".into(),
        title: a.title.clone(),
        subtitle: a.album_artist.clone(),
        image: Some(format!("/v1/library/albums/{id}/cover")),
        id: format!("lib-{id}"),
        year: a.year,
        keys: a
            .upc
            .as_ref()
            .map(|u| vec![format!("upc:{u}")])
            .unwrap_or_default(),
        ..Default::default()
    })
}

/// A file's format in words: `FLAC 16/44.1`, `MP3 320 kbps` (no bitrate known: `MP3`), `DSD64`.
pub(crate) fn file_format(f: &pm_model::AudioFile) -> String {
    let fm = &f.format;
    if matches!(fm.kind, pm_model::SampleKind::Dsd) {
        return format!("DSD{}", (fm.rate_hz / 44_100).max(1));
    }
    let codec = f.container.to_uppercase();
    if file_quality(f) == "lossy" {
        return codec;
    }
    let k = fm.rate_hz as f64 / 1000.0;
    let rate = format!("{:.1}", k).trim_end_matches(".0").to_string();
    format!("{codec} {}/{rate}", fm.bits)
}

/// `hi_res`, `lossless` or `lossy`, from the container and the sample format.
pub(crate) fn file_quality(f: &pm_model::AudioFile) -> &'static str {
    let fm = &f.format;
    let lossy = pm_library::tags::is_lossy_container(&f.container.to_lowercase());
    if lossy {
        "lossy"
    } else if matches!(fm.kind, pm_model::SampleKind::Dsd) || fm.bits > 16 || fm.rate_hz > 48_000 {
        "hi_res"
    } else {
        "lossless"
    }
}

/// `#chill #90s` in the library: what you tagged with all of them (your tags table, offline).
fn library_tagged(
    lib: &pm_library::Library,
    labels: &[String],
    limit: u32,
) -> pm_library::Result<Vec<SearchHit>> {
    let mut by_uri: HashMap<String, Vec<String>> = HashMap::new();
    for t in lib
        .list_tags(None)?
        .into_iter()
        .filter(|t| labels.contains(&t.label))
    {
        by_uri.entry(t.uri).or_default().push(t.label);
    }
    let shown = labels
        .iter()
        .map(|l| format!("#{l}"))
        .collect::<Vec<_>>()
        .join(" ");
    let mut out = Vec::new();
    for (uri, got) in by_uri {
        if !labels.iter().all(|l| got.contains(l)) {
            continue;
        }
        let Ok(m) = pm_model::MusicUri::parse(&uri) else {
            continue;
        };
        if let pm_model::MusicUri::Release { mbid } = &m {
            if let Some(a) = lib.album_by_release_mbid(*mbid)? {
                if let Some(id) = a.id {
                    out.push(SearchHit {
                        kind: "album".into(),
                        title: a.title.clone(),
                        subtitle: a.album_artist.clone(),
                        image: Some(format!("/v1/library/albums/{id}/cover")),
                        id: format!("lib-{id}"),
                        tagged: Some(shown.clone()),
                        ..Default::default()
                    });
                }
            }
            continue;
        }
        for t in lib.tracks_by_canonical(&m)?.into_iter().take(1) {
            if let Some(mut h) = song_hit(&t) {
                h.tagged = Some(shown.clone());
                out.push(h);
            }
        }
        if out.len() >= limit as usize {
            break;
        }
    }
    Ok(out)
}

/// `owner`: the node's own playlists are the owner's; a listener's search leaves them out.
fn library(s: &NodeState, q: &str, limit: u32, owner: bool) -> pm_library::Result<Vec<SearchHit>> {
    let lib = s.library.lock().expect("library poisoned");
    let hashed = hash_tags(q);
    if !hashed.is_empty() {
        return library_tagged(&lib, &hashed, limit);
    }
    let needle = q.to_lowercase();
    let mut out = Vec::new();
    for a in lib
        .list_artists()?
        .into_iter()
        .filter(|a| a.name.to_lowercase().contains(&needle))
        .take(8)
    {
        out.push(artist_hit(&a));
    }
    let own_playlists = if owner {
        crate::playlists::summaries_named(&lib, &needle, 6).unwrap_or_default()
    } else {
        Vec::new()
    };
    for pl in own_playlists {
        out.push(SearchHit {
            kind: "playlist".into(),
            title: pl.name.clone(),
            subtitle: Some(format!(
                "{} · your playlist",
                crate::plural(pl.items as u64, "song")
            )),
            image: pl.image.clone(),
            id: pl.id.clone(),
            ..Default::default()
        });
    }
    for a in lib.search_albums(q, 12)? {
        out.extend(album_hit(&a));
    }
    for t in lib.search_tracks(q, limit)? {
        out.extend(song_hit(&t));
    }
    Ok(out)
}

fn kind_of_type(t: Option<&str>) -> Option<&'static str> {
    Some(match t? {
        "MusicRecording" | "MusicComposition" => "song",
        "MusicAlbum" | "MusicRelease" => "album",
        "MusicGroup" | "Person" => "artist",
        "MusicPlaylist" => "playlist",
        "PodcastSeries" | "PodcastEpisode" => "podcast",
        "MusicEvent" => "concert",
        _ => return None,
    })
}

/// A search term as pubky.app tag labels: lowercase, no spaces or commas, 1–20 characters; a
/// multi-word term also hyphenated ("bohemian rhapsody" → `bohemian-rhapsody`).
pub(crate) fn tag_labels(term: &str) -> Vec<String> {
    let clean = |x: &str| -> String {
        x.trim()
            .trim_start_matches('#')
            .to_lowercase()
            .chars()
            .filter(|c| !c.is_whitespace() && *c != ',')
            .collect()
    };
    let words: Vec<&str> = term.split_whitespace().collect();
    let mut out = Vec::new();
    for l in [clean(term), clean(&words.join("-"))] {
        if !l.is_empty() && l.chars().count() <= 20 && !out.contains(&l) {
            out.push(l);
        }
    }
    out
}

/// `#chill #90s` (or `tag:chill tag:90s`) → `["chill", "90s"]`; none when the term has no tag word.
pub(crate) fn hash_tags(q: &str) -> Vec<String> {
    q.split_whitespace()
        .filter_map(|w| {
            if w.starts_with('#') && w.len() > 1 {
                Some(w)
            } else {
                w.strip_prefix("tag:").filter(|r| !r.is_empty())
            }
        })
        .flat_map(tag_labels)
        .collect()
}

/// The words of `q` that are not tags (`#x` or `tag:x`): what text search gets.
pub(crate) fn text_words(q: &str) -> String {
    q.split_whitespace()
        .filter(|w| !(w.starts_with('#') && w.len() > 1) && !w.starts_with("tag:"))
        .collect::<Vec<_>>()
        .join(" ")
}

pub(crate) fn card_hit(c: &pm_pubky::pinakes::Card) -> Option<SearchHit> {
    if !c.is_ok() {
        return None;
    }
    let kind = kind_of_type(c.type_.as_deref())?;
    let mut keys = Vec::new();
    if let Some(g) = &c.group {
        keys.push(format!("g:{}", g.id));
    }
    let subtitle = if kind == "concert" {
        c.start.clone().map(|st| st.get(..10).unwrap_or(&st).to_string())
    } else {
        c.artist_hint().or_else(|| c.site.clone())
    };
    Some(SearchHit {
        kind: kind.into(),
        title: c
            .name
            .clone()
            .unwrap_or_else(|| format!("Untitled on {}", c.site.clone().unwrap_or_default())),
        subtitle,
        image: c.image.clone(),
        id: crate::discover::page_id(&c.uri),
        uri: crate::discover::playable_uri(&c.uri),
        page: Some(c.uri.clone()),
        keys,
        site: c.site.clone(),
        ..Default::default()
    })
}

/// People on Pubky whose name starts with the term (Nexus).
/// Until when Nexus asked the node to wait (a 429): people and posts aren't asked meanwhile, so a
/// search per keystroke doesn't keep it busy.
fn nexus_wait() -> &'static Mutex<Option<Instant>> {
    static WAIT: OnceLock<Mutex<Option<Instant>>> = OnceLock::new();
    WAIT.get_or_init(|| Mutex::new(None))
}

const NEXUS_BUSY: &str = "Pubky search is busy: try again in a few seconds";

fn nexus_ready() -> Result<(), String> {
    match *nexus_wait().lock().expect("nexus wait") {
        Some(until) if Instant::now() < until => Err(NEXUS_BUSY.into()),
        _ => Ok(()),
    }
}

fn nexus_error(e: pm_pubky::nexus::NexusError) -> String {
    if let pm_pubky::nexus::NexusError::RateLimited(after) = e {
        let wait = after
            .unwrap_or(Duration::from_secs(10))
            .min(Duration::from_secs(120));
        *nexus_wait().lock().expect("nexus wait") = Some(Instant::now() + wait);
        return NEXUS_BUSY.into();
    }
    e.to_string()
}

async fn people(s: &Arc<NodeState>, q: &str, limit: u32) -> Result<Found, String> {
    nexus_ready()?;
    let d = s
        .discovery()
        .ok_or_else(|| "People on Pubky need the upstream contact (Settings)".to_string())?;
    let term = text_words(q);
    if term.is_empty() {
        return Ok((Vec::new(), Vec::new()));
    }
    if term.chars().count() < 2 {
        return Ok((Vec::new(), Vec::new()));
    }
    let users = match d.nexus().users_by_name(&term, limit.min(10)).await {
        Ok(u) => u,
        Err(pm_pubky::nexus::NexusError::NotFound) => Vec::new(),
        Err(e) => return Err(nexus_error(e)),
    };
    let hits = users
        .into_iter()
        .map(|u| {
            let id = u.details.id.clone();
            let bio = u.details.bio.clone().unwrap_or_default();
            let bio = bio.trim();
            SearchHit {
                kind: "person".into(),
                title: u.details.name.clone(),
                subtitle: Some(if bio.is_empty() {
                    "on Pubky".to_string()
                } else {
                    bio.chars().take(80).collect()
                }),
                image: u
                    .details
                    .image
                    .as_ref()
                    .map(|_| format!("/v1/profiles/{id}/avatar")),
                id: id.clone(),
                page: Some(format!("https://pubky.app/profile/{id}")),
                uri: Some(format!("pubky://{id}")),
                ..Default::default()
            }
        })
        .collect();
    Ok((hits, Vec::new()))
}

/// Posts on Pubky whose text matches the term, or that carry the `#tag` (Nexus).
async fn posts(s: &Arc<NodeState>, q: &str, limit: u32) -> Result<Found, String> {
    nexus_ready()?;
    let d = s
        .discovery()
        .ok_or_else(|| "Posts on Pubky need the upstream contact (Settings)".to_string())?;
    let nexus = d.nexus();
    let tags = hash_tags(q);
    let found = match tags.first() {
        // One tag: Nexus' own list. Several: posts carrying all of them (each tag's list, kept
        // where they meet, in the first tag's order)
        Some(label) if tags.len() == 1 => nexus.posts_by_tag(label, limit.min(10)).await,
        Some(_) => {
            let lists =
                futures_util::future::join_all(tags.iter().take(5).map(|l| nexus.posts_by_tag(l, 100))).await;
            let mut lists = lists.into_iter();
            match lists.next() {
                Some(Ok(first)) => {
                    let mut rest = Vec::new();
                    let mut failed = None;
                    for l in lists {
                        match l {
                            Ok(k) => rest.push(k.into_iter().collect::<std::collections::HashSet<_>>()),
                            Err(e) => {
                                failed = Some(e);
                                break;
                            }
                        }
                    }
                    match failed {
                        Some(e) => Err(e),
                        None => Ok(first
                            .into_iter()
                            .filter(|k| rest.iter().all(|r| r.contains(k)))
                            .take(limit.min(10) as usize)
                            .collect()),
                    }
                }
                Some(Err(e)) => Err(e),
                None => Ok(Vec::new()),
            }
        }
        None => {
            // Nexus searches 2 to 30 characters and at most 4 words
            let term: String = text_words(q)
                .split_whitespace()
                .take(4)
                .collect::<Vec<_>>()
                .join(" ")
                .chars()
                .take(30)
                .collect();
            if term.trim().chars().count() < 2 {
                return Ok((Vec::new(), Vec::new()));
            }
            nexus.posts_by_content(term.trim(), limit.min(10)).await
        }
    };
    let keys = match found {
        Ok(k) => k,
        Err(pm_pubky::nexus::NexusError::NotFound) => Vec::new(),
        Err(e) => return Err(nexus_error(e)),
    };
    let views = nexus.posts_by_ids(&keys).await.map_err(|e| e.to_string())?;
    let authors: Vec<String> = {
        let mut a: Vec<String> = views.iter().map(|p| p.details.author.clone()).collect();
        a.sort();
        a.dedup();
        a
    };
    let names: HashMap<String, String> = nexus
        .users_by_ids(&authors, None)
        .await
        .unwrap_or_default()
        .into_iter()
        .map(|u| (u.details.id.clone(), u.details.name.clone()))
        .collect();
    let hits = views
        .into_iter()
        .map(|p| {
            let d = p.details;
            let first = d
                .content
                .lines()
                .find(|l| !l.trim().is_empty())
                .unwrap_or("")
                .trim();
            let title: String = if first.is_empty() {
                "Untitled post".to_string()
            } else if first.chars().count() > 120 {
                format!("{}…", first.chars().take(119).collect::<String>())
            } else {
                first.to_string()
            };
            let by = names.get(&d.author).cloned().unwrap_or_else(|| "someone".into());
            SearchHit {
                kind: "post".into(),
                title,
                // The kind when it says more than "a post" (Nexus' social kind is "" when unknown)
                subtitle: Some(match d.kind.as_str() {
                    "" | "short" | "long" => by,
                    k => format!("{by} · {k}"),
                }),
                id: format!("post-{}:{}", d.author, d.id),
                page: Some(format!("https://pubky.app/post/{}/{}", d.author, d.id)),
                uri: Some(d.uri.clone()).filter(|u| !u.is_empty()),
                ..Default::default()
            }
        })
        .collect();
    Ok((hits, Vec::new()))
}

/// A search term that is an exact identifier, not words.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Ident {
    /// A Pinakes `key`: `isrc:<ISRC>`, `upc:<barcode>` or `mbid:<uuid>`.
    Key(String),
    /// A pasted page of a music service or MusicBrainz.
    Link(String),
}

/// An ISRC, bare or hyphenated: two letters, three letters or digits, seven digits.
fn isrc_of(t: &str) -> Option<String> {
    let s: String = t
        .chars()
        .filter(|c| *c != '-')
        .collect::<String>()
        .to_ascii_uppercase();
    let b = s.as_bytes();
    let ok = b.len() == 12
        && b[..2].iter().all(|c| c.is_ascii_uppercase())
        && b[2..5]
            .iter()
            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit())
        && b[5..].iter().all(|c| c.is_ascii_digit());
    ok.then_some(s)
}

/// A barcode's last digit is its GS1 check digit (UPC-A, EAN-13).
fn gs1_ok(digits: &str) -> bool {
    if digits.len() < 2 || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return false;
    }
    let v: Vec<u32> = digits.bytes().map(|b| u32::from(b - b'0')).collect();
    let (body, check) = v.split_at(v.len() - 1);
    let sum: u32 = body
        .iter()
        .rev()
        .enumerate()
        .map(|(i, x)| x * if i % 2 == 0 { 3 } else { 1 })
        .sum();
    (10 - sum % 10) % 10 == check[0]
}

/// The music services whose pasted pages are looked up as links.
fn music_link_host(host: &str) -> bool {
    let h = host.trim_start_matches("www.");
    matches!(
        h,
        "open.spotify.com"
            | "deezer.com"
            | "tidal.com"
            | "listen.tidal.com"
            | "music.apple.com"
            | "music.youtube.com"
            | "musicbrainz.org"
            | "bandcamp.com"
    ) || h.ends_with(".bandcamp.com")
}

/// What `q` is when it is one exact identifier: an ISRC, a UPC/EAN with a valid check digit, an
/// MBID (each bare or prefixed `isrc:`, `upc:`/`ean:`, `mbid:`), or a pasted page of Spotify,
/// Deezer, TIDAL, Apple Music, Bandcamp, YouTube Music or MusicBrainz. Anything else is text.
pub(crate) fn identifier(q: &str) -> Option<Ident> {
    let t = q.trim();
    if t.is_empty() || t.chars().any(char::is_whitespace) {
        return None;
    }
    let lower = t.to_ascii_lowercase();
    if lower.starts_with("https://") || lower.starts_with("http://") {
        let u = url::Url::parse(t).ok()?;
        let host = u.host_str()?.to_ascii_lowercase();
        // A service's front page is not a thing to look up
        return (music_link_host(&host) && u.path().len() > 1).then(|| Ident::Link(t.to_string()));
    }
    if let Some(rest) = lower.strip_prefix("isrc:") {
        return isrc_of(rest).map(|i| Ident::Key(format!("isrc:{i}")));
    }
    if let Some(rest) = lower.strip_prefix("upc:").or_else(|| lower.strip_prefix("ean:")) {
        let ok = (8..=14).contains(&rest.len()) && rest.bytes().all(|b| b.is_ascii_digit());
        return ok.then(|| Ident::Key(format!("upc:{rest}")));
    }
    if let Some(rest) = lower.strip_prefix("mbid:") {
        return pm_model::Mbid::parse(rest)
            .is_ok()
            .then(|| Ident::Key(format!("mbid:{rest}")));
    }
    if lower.len() == 36 && pm_model::Mbid::parse(&lower).is_ok() {
        return Some(Ident::Key(format!("mbid:{lower}")));
    }
    if (t.len() == 12 || t.len() == 13) && gs1_ok(t) {
        return Some(Ident::Key(format!("upc:{t}")));
    }
    // A hyphenated ISRC is written CC-XXX-YY-NNNNN; other hyphenated words are text
    let hyphens = t.matches('-').count();
    if (hyphens == 0 && t.len() == 12) || (hyphens == 3 && t.len() == 15) {
        return isrc_of(t).map(|i| Ident::Key(format!("isrc:{i}")));
    }
    None
}

/// Pinakes' `facet=type` counts as Odeion's kinds: types of one kind add up (albums are
/// `MusicAlbum` and `MusicRelease`), other types are left out.
pub(crate) fn kind_counts(types: &[pm_pubky::pinakes::TypeFacet]) -> Vec<(String, i64)> {
    let mut out: Vec<(String, i64)> = Vec::new();
    for t in types {
        let Some(kind) = kind_of_type(Some(&t.type_)) else {
            continue;
        };
        match out.iter_mut().find(|(k, _)| k == kind) {
            Some(c) => c.1 += t.count,
            None => out.push((kind.to_string(), t.count)),
        }
    }
    out
}

/// Pinakes instances that refused `boost=wot_2` (their Nexus serves no reach set), by base URL,
/// with when: not asked again for an hour.
fn no_boost() -> &'static Mutex<HashMap<String, Instant>> {
    static NO_BOOST: OnceLock<Mutex<HashMap<String, Instant>>> = OnceLock::new();
    NO_BOOST.get_or_init(Default::default)
}
const NO_BOOST_TTL: Duration = Duration::from_secs(3600);

/// The text query over music cards, one row per group, with counts by type. With a `viewer`
/// what their network referenced ranks higher (`boost=wot_2`) where the instance serves it; one
/// that refuses is remembered and the query repeated once without, so nothing is lost.
pub(crate) async fn text_search(
    p: &pm_pubky::pinakes::PinakesClient,
    q: &str,
    limit: u32,
    viewer: Option<String>,
) -> Result<pm_pubky::pinakes::SearchResponse, pm_pubky::pinakes::PinakesError> {
    let query = |viewer: Option<String>| pm_pubky::pinakes::SearchQuery {
        q: Some(q.to_string()),
        type_: Some(pm_pubky::pinakes::music_types()),
        collapse: Some("group".into()),
        facet: Some("type".into()),
        limit: Some(limit as usize),
        boost: viewer.as_ref().map(|_| "wot_2".to_string()),
        viewer,
        ..Default::default()
    };
    let refused = no_boost()
        .lock()
        .expect("boost memo")
        .get(p.base())
        .is_some_and(|at| at.elapsed() < NO_BOOST_TTL);
    let Some(viewer) = viewer.filter(|_| !refused) else {
        return p.search(&query(None)).await;
    };
    match p.search(&query(Some(viewer))).await {
        Err(
            pm_pubky::pinakes::PinakesError::BadRequest(_)
            | pm_pubky::pinakes::PinakesError::Status { status: 501, .. },
        ) => {
            no_boost()
                .lock()
                .expect("boost memo")
                .insert(p.base().to_string(), Instant::now());
            p.search(&query(None)).await
        }
        r => r,
    }
}

/// The sites of an "Also on" list, each once, without `own`.
fn also_sites(also: &[pm_pubky::pinakes::AlsoOn], own: Option<&str>) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for a in also {
        let Some(site) = a.site.as_deref().filter(|x| !x.is_empty()) else {
            continue;
        };
        if Some(site) != own && !out.iter().any(|o| o == site) {
            out.push(site.to_string());
        }
    }
    out
}

/// Pubky, music cards only: the text query and, in parallel, the term as tags
/// (STEWARD-FIX-search-tags.md) and concerts near the node's saved place; a `#tag` term searches
/// tags only (several: all of them). A term that is an identifier or a pasted service page
/// (`identifier`) is looked up exactly instead, in one request, and comes first.
async fn pubky(s: &Arc<NodeState>, q: &str, limit: u32, owner: bool) -> Result<Found, String> {
    let d = s
        .discovery()
        .ok_or_else(|| "Pubky search needs the upstream contact (Settings)".to_string())?;
    let hashed = hash_tags(q);
    let tags_only = !hashed.is_empty();
    let ident = if tags_only { None } else { identifier(q) };
    let words = !tags_only && ident.is_none();
    let labels = if tags_only {
        hashed
    } else if words {
        tag_labels(q)
    } else {
        Vec::new()
    };
    let types = pm_pubky::pinakes::music_types();
    // The owner's own network; a listener's search is not boosted
    let viewer = if owner { crate::home::owner(s) } else { None };
    let exact = async {
        let found: Vec<(pm_pubky::pinakes::Card, Vec<String>)> = match &ident {
            Some(Ident::Key(key)) => d
                .pinakes()
                .search(&pm_pubky::pinakes::SearchQuery {
                    type_: Some(types.clone()),
                    collapse: Some("group".into()),
                    limit: Some(limit as usize),
                    ..pm_pubky::pinakes::SearchQuery::key(key)
                })
                .await?
                .items
                .into_iter()
                .map(|c| {
                    let also = also_sites(&c.also_on, c.site.as_deref());
                    (c, also)
                })
                .collect(),
            // Read only: a search never makes Pinakes fetch a page
            Some(Ident::Link(uri)) => d
                .pinakes()
                .resource_by_uri(uri)
                .await?
                .map(|r| {
                    let also = also_sites(r.also_on(), r.card.site.as_deref());
                    (r.card, also)
                })
                .into_iter()
                .collect(),
            None => Vec::new(),
        };
        Ok::<_, pm_pubky::pinakes::PinakesError>(found)
    };
    let text = async {
        if !words {
            return Ok(None);
        }
        text_search(d.pinakes(), q, limit, viewer).await.map(Some)
    };
    let by_tag = async {
        if labels.is_empty() {
            return Ok(None);
        }
        d.pinakes()
            .search(&pm_pubky::pinakes::SearchQuery {
                tags: Some(labels.join(",")),
                match_: (tags_only && labels.len() > 1).then(|| "all".to_string()),
                type_: Some(types.clone()),
                collapse: Some("group".into()),
                facet: tags_only.then(|| "type".to_string()),
                limit: Some(limit as usize),
                ..Default::default()
            })
            .await
            .map(Some)
    };
    // Concerts matching the words, from now on, near the node's saved place (Home's Concerts row)
    let near = async {
        let loc = crate::home::location(s).filter(|_| words)?;
        d.pinakes()
            .search(&pm_pubky::pinakes::SearchQuery {
                q: Some(q.to_string()),
                type_: Some("MusicEvent".into()),
                near: Some(format!("{:.4},{:.4}", loc.lat, loc.lon)),
                radius: Some(crate::home::CONCERT_RADIUS_M.to_string()),
                from: Some("now".into()),
                collapse: Some("group".into()),
                limit: Some(limit.min(10) as usize),
                ..Default::default()
            })
            .await
            .ok()
    };
    let (exact, text, by_tag, near) = tokio::join!(exact, text, by_tag, near);
    // The exact lookup or the text query decides failure; a tag query that fails only loses the
    // tag hits, a concerts query that fails only the concerts near
    let exact = exact.map_err(|e| e.to_string())?;
    let text = text.map_err(|e| e.to_string())?;
    let by_tag = match by_tag {
        Ok(p) => p,
        Err(e) if tags_only => return Err(e.to_string()),
        Err(_) => None,
    };
    let facet_page = text.as_ref().or(by_tag.as_ref());
    let counts = facet_page
        .and_then(|p| p.facets.as_ref())
        .map(|f| kind_counts(&f.types))
        .unwrap_or_default();
    let mut hits: Vec<SearchHit> = exact
        .iter()
        .filter_map(|(card, also)| {
            let mut h = card_hit(card)?;
            h.exact = true;
            h.also = also.clone();
            Some(h)
        })
        .collect();
    // Concerts near come before concerts anywhere; the same page is one row
    let later = near
        .iter()
        .chain(text.iter())
        .flat_map(|p| p.items.iter().filter_map(card_hit));
    for h in later {
        if !hits.iter().any(|x| x.page == h.page) {
            hits.push(h);
        }
    }
    let shown = if tags_only {
        labels
            .iter()
            .map(|l| format!("#{l}"))
            .collect::<Vec<_>>()
            .join(" ")
    } else {
        format!("#{}", labels.first().cloned().unwrap_or_default())
    };
    for mut h in by_tag
        .as_ref()
        .map(|p| p.items.iter().filter_map(card_hit).collect::<Vec<_>>())
        .unwrap_or_default()
    {
        // One row per group: a text hit that is also tagged says so
        if let Some(prev) = hits
            .iter_mut()
            .find(|x| x.page == h.page || (!h.keys.is_empty() && x.keys.iter().any(|k| h.keys.contains(k))))
        {
            prev.tagged.get_or_insert(shown.clone());
            continue;
        }
        h.tagged = Some(shown.clone());
        hits.push(h);
    }
    Ok((hits, counts))
}

/// `GET /v1/search/tags?prefix=`: tag labels as you type: yours first (with how many items you
/// tagged), then everyone's on Pubky (Nexus).
pub(crate) async fn tag_suggestions(
    State(s): State<Arc<NodeState>>,
    Query(p): Query<std::collections::HashMap<String, String>>,
) -> axum::Json<serde_json::Value> {
    let prefix: String = p
        .get("prefix")
        .map(|x| x.trim().trim_start_matches('#').to_lowercase())
        .unwrap_or_default();
    if prefix.is_empty() {
        return axum::Json(serde_json::json!({"mine": [], "others": []}));
    }
    let mut mine: Vec<(String, usize)> = Vec::new();
    if let Ok(rows) = s.library.lock().expect("library poisoned").list_tags(None) {
        for t in rows.into_iter().filter(|t| t.label.starts_with(&prefix)) {
            match mine.iter_mut().find(|(l, _)| *l == t.label) {
                Some(m) => m.1 += 1,
                None => mine.push((t.label, 1)),
            }
        }
    }
    mine.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    let others: Vec<String> = match s.discovery() {
        Some(d) => tokio::time::timeout(
            std::time::Duration::from_secs(2),
            d.nexus().tags_by_prefix(&prefix, 8),
        )
        .await
        .ok()
        .and_then(|r| r.ok())
        .unwrap_or_default()
        .into_iter()
        .filter(|l| !mine.iter().any(|(m, _)| m == l))
        .collect(),
        None => Vec::new(),
    };
    axum::Json(serde_json::json!({
        "mine": mine.into_iter().take(8).map(|(label, count)| serde_json::json!({"label": label, "count": count})).collect::<Vec<_>>(),
        "others": others,
    }))
}

async fn service(
    s: &Arc<NodeState>,
    svc: &dyn StreamingService,
    q: &str,
    limit: u32,
) -> Result<Found, String> {
    let hits = svc.search(s, q, limit).await.map_err(|e| e.message)?;
    let lib = s.library.lock().expect("library poisoned");
    let mut out = Vec::new();
    for mut h in hits {
        crate::streaming::mark_library(&lib, &mut h);
        let kind = match h.kind.as_str() {
            "track" => "song",
            "album" | "item" => "album",
            "artist" => "artist",
            "playlist" => "playlist",
            _ => continue,
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
            _ => crate::discover::page_id(&h.url),
        };
        out.push(SearchHit {
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
        });
    }
    Ok((out, Vec::new()))
}

#[cfg(test)]
mod ident_tests {
    use super::*;

    fn key(k: &str) -> Option<Ident> {
        Some(Ident::Key(k.to_string()))
    }

    #[test]
    fn identifiers_are_recognised_and_near_misses_stay_text() {
        // ISRC: bare, hyphenated, lower case, prefixed
        assert_eq!(identifier("USQX91300108"), key("isrc:USQX91300108"));
        assert_eq!(identifier(" usqx91300108 "), key("isrc:USQX91300108"));
        assert_eq!(identifier("US-QX9-13-00108"), key("isrc:USQX91300108"));
        assert_eq!(identifier("isrc:GBUM71029604"), key("isrc:GBUM71029604"));
        assert_eq!(identifier("ISRC:gb-um7-10-29604"), key("isrc:GBUM71029604"));
        // Near misses: a 12-letter word, 11 or 13 characters, letters where digits belong,
        // other hyphenations
        assert_eq!(identifier("unbelievable"), None);
        assert_eq!(identifier("USQX9130010"), None);
        assert_eq!(identifier("USQX913001088"), None);
        assert_eq!(identifier("USQX9130010X"), None);
        assert_eq!(identifier("1SQX91300108"), None);
        assert_eq!(identifier("rock-n-roll-1"), None);
        assert_eq!(identifier("US-QX91300108"), None);
        assert_eq!(identifier("isrc:nope"), None);
        // UPC-A and EAN-13 with a valid check digit; a wrong one, or another length, is text
        assert_eq!(identifier("036000291452"), key("upc:036000291452"));
        assert_eq!(identifier("4006381333931"), key("upc:4006381333931"));
        assert_eq!(identifier("036000291453"), None);
        assert_eq!(identifier("4006381333930"), None);
        assert_eq!(identifier("03600029145"), None);
        assert_eq!(identifier("1999"), None);
        assert_eq!(identifier("upc:724384960650"), key("upc:724384960650"));
        assert_eq!(identifier("ean:0724384960650"), key("upc:0724384960650"));
        assert_eq!(identifier("upc:12ab"), None);
        // MBID
        let mbid = "b1a9c0e9-d987-4042-ae91-78d6a3267d69";
        assert_eq!(identifier(mbid), key(&format!("mbid:{mbid}")));
        assert_eq!(identifier(&mbid.to_uppercase()), key(&format!("mbid:{mbid}")));
        assert_eq!(identifier(&format!("mbid:{mbid}")), key(&format!("mbid:{mbid}")));
        assert_eq!(identifier("b1a9c0e9-d987-4042-ae91-78d6a3267d6"), None);
        assert_eq!(identifier("mbid:not-a-uuid"), None);
        // Pasted pages of music services; other sites, front pages and words are text
        for link in [
            "https://open.spotify.com/track/2Foc5Q5nqNiosCNqttzHof",
            "https://www.deezer.com/en/track/66609426",
            "https://tidal.com/browse/album/20115564",
            "https://listen.tidal.com/album/20115564",
            "https://music.apple.com/us/album/random-access-memories/617154241",
            "https://daftpunk.bandcamp.com/album/homework",
            "https://music.youtube.com/watch?v=5NV6Rdv1a3I",
            "https://musicbrainz.org/recording/b1a9c0e9-d987-4042-ae91-78d6a3267d69",
        ] {
            assert_eq!(identifier(link), Some(Ident::Link(link.to_string())), "{link}");
        }
        assert_eq!(identifier("https://example.com/track/1"), None);
        assert_eq!(identifier("https://open.spotify.com/"), None);
        assert_eq!(identifier("https://notbandcamp.com/album/x"), None);
        assert_eq!(identifier("https://evil.example/open.spotify.com/track/1"), None);
        assert_eq!(identifier("daft punk"), None);
        assert_eq!(identifier("get lucky USQX91300108"), None);
        assert_eq!(identifier("#USQX91300108"), None);
        assert_eq!(identifier(""), None);
    }

    #[test]
    fn type_counts_add_up_per_kind_and_leave_other_types_out() {
        let f = |t: &str, count: i64| pm_pubky::pinakes::TypeFacet {
            type_: t.to_string(),
            count,
        };
        let counts = kind_counts(&[
            f("MusicGroup", 9),
            f("MusicAlbum", 3),
            f("WebPage", 40),
            f("MusicRelease", 2),
            f("MusicRecording", 3),
            f("Person", 1),
            f("MusicEvent", 4),
            f("PodcastEpisode", 5),
            f("VideoObject", 1),
        ]);
        assert_eq!(
            counts,
            vec![
                ("artist".to_string(), 10),
                ("album".to_string(), 5),
                ("song".to_string(), 3),
                ("concert".to_string(), 4),
                ("podcast".to_string(), 5),
            ]
        );
    }

    #[test]
    fn the_music_types_fit_one_query_and_are_all_shown() {
        assert!(pm_pubky::pinakes::MUSIC_TYPES.len() <= 10);
        for t in pm_pubky::pinakes::MUSIC_TYPES {
            assert!(kind_of_type(Some(t)).is_some(), "{t} has no kind");
        }
        assert_eq!(pm_pubky::pinakes::music_types().split(',').count(), 8);
    }
}
