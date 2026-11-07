//! An artist's releases on a service, sorted into what the service says they are.
//!
//! An artist page lists Albums, then EPs & singles, then everything else ("Other releases"),
//! from the type each service gives its releases. Never from a title or a track count: a single
//! called "Album", an EP called "Greatest Hits" or a one-track album stay what the service says.
//! A release the service gives no usable type is unknown, and listed under Other.
//!
//! The whole catalogue is fetched, page by page, up to [`MAX_RELEASES`]; a [`Catalogue`](crate::types::Catalogue) says
//! whether every page arrived, whether the bound cut it, and why a list is partial. Releases are
//! told apart only by the service's own id: editions, explicit and clean versions and remixes
//! with the same title are separate releases and all stay.

use crate::types::{ArtistRelease, ItemAlbum, ReleaseKind};
use serde_json::Value;

/// The most releases an artist page fetches from one service. TIDAL lists an artist's releases
/// 20 at a time with no type filter, so the whole list (and every album's details) has to be
/// fetched to sort it: 500 is 25 pages and 25 detail calls.
pub const MAX_RELEASES: usize = 500;

/// TIDAL's relationship pages have a size the server picks (20 when we looked); the page bound
/// assumes pages this small, so a smaller page size still reaches [`MAX_RELEASES`].
pub const TIDAL_PAGE_FLOOR: usize = 10;

/// The most pages a catalogue fetch asks for, whatever the pages hold: enough for
/// [`MAX_RELEASES`] at `page_size` a page, and a few more. Calls are bounded by this, not by how
/// many usable rows arrive (rows the node can't use, or repeats, never make it ask more).
pub const fn max_pages(page_size: usize) -> usize {
    MAX_RELEASES.div_ceil(page_size) + 5
}

/// A catalogue's `next` link, followed only where the first page was asked: the same scheme,
/// host and port as the service's API base and exactly the same list path (the query, the
/// cursor, may differ). A relative link is taken against the API base, as the service's own
/// calls are. Anything else is refused before any token is attached, with the reason.
pub fn checked_next(api_base: &str, next: &str, list_path: &str) -> Result<String, String> {
    let refused = |why: &str| format!("refused the service's next page link ({why})");
    let base = url::Url::parse(api_base).map_err(|_| refused("no API base"))?;
    let base_path = base.path().trim_end_matches('/').to_string();
    let url = if next.starts_with('/') && !next.starts_with("//") {
        base.join(&format!("{base_path}{next}"))
    } else {
        url::Url::parse(next)
    }
    .map_err(|_| refused("not a link"))?;
    if url.scheme() != base.scheme() {
        return Err(refused(&format!("{} instead of {}", url.scheme(), base.scheme())));
    }
    if url.host_str() != base.host_str() || url.port_or_known_default() != base.port_or_known_default() {
        return Err(refused("another address"));
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(refused("credentials in the link"));
    }
    if url.path() != format!("{base_path}{list_path}") {
        return Err(refused("another path"));
    }
    Ok(url.to_string())
}

fn kind(release_type: Option<&str>, released: Option<&str>, compilation: bool) -> ReleaseKind {
    ReleaseKind {
        release_type: release_type.map(str::to_string),
        released: released.filter(|d| !d.is_empty()).map(str::to_string),
        compilation,
        secondary_types: Vec::new(),
    }
}

/// TIDAL v2 (official API): `albumType` ALBUM|EP|SINGLE, else the deprecated `type` with the
/// same values (tidal-api-reference, `Albums_Attributes`).
pub fn tidal(attrs: &Value) -> ReleaseKind {
    let t = attrs["albumType"].as_str().or(attrs["type"].as_str());
    let rt = match t {
        Some("ALBUM") => Some("album"),
        Some("EP") => Some("ep"),
        Some("SINGLE") => Some("single"),
        _ => None,
    };
    kind(rt, attrs["releaseDate"].as_str(), false)
}

/// Spotify (Get Artist's Albums): `album_type` album|single|compilation. A compilation is a
/// trait, not an album/EP/single. `album_group` (removed for development-mode apps, February
/// 2026) is the artist's relationship to the album, never its type.
pub fn spotify(a: &Value) -> ReleaseKind {
    let (rt, comp) = match a["album_type"].as_str() {
        Some("album") => (Some("album"), false),
        Some("single") => (Some("single"), false),
        Some("compilation") => (None, true),
        _ => (None, false),
    };
    kind(rt, a["release_date"].as_str(), comp)
}

/// Deezer: `record_type` album|ep|single (any case). Other values (Deezer's compilation value is
/// not in its public reference) stay unknown, with no trait claimed.
pub fn deezer(a: &Value) -> ReleaseKind {
    let rt = a["record_type"]
        .as_str()
        .map(str::to_ascii_lowercase)
        .and_then(|t| match t.as_str() {
            "album" => Some("album"),
            "ep" => Some("ep"),
            "single" => Some("single"),
            _ => None,
        });
    kind(rt, a["release_date"].as_str(), false)
}

/// Qobuz: `product_type`, else `release_type`, album|single; `product_type` compilation is a
/// trait. Other values stay unknown.
pub fn qobuz(a: &Value) -> ReleaseKind {
    let comp = a["product_type"].as_str() == Some("compilation");
    let rt = [a["product_type"].as_str(), a["release_type"].as_str()]
        .into_iter()
        .flatten()
        .find_map(|t| match t {
            "album" => Some("album"),
            "single" => Some("single"),
            _ => None,
        });
    kind(rt, a["release_date_original"].as_str(), comp)
}

/// MusicBrainz release group (musicbrainz.org/doc/Release_Group/Type): primary type Album, EP,
/// Single (Broadcast, Other and none stay unknown); the Compilation secondary type is a trait,
/// and every secondary type (Live, Remix, Soundtrack…) is kept as MusicBrainz names it.
pub fn musicbrainz(g: &Value) -> ReleaseKind {
    let rt = match g["primary-type"].as_str() {
        Some("Album") => Some("album"),
        Some("EP") => Some("ep"),
        Some("Single") => Some("single"),
        _ => None,
    };
    let comp = g["secondary-types"]
        .as_array()
        .is_some_and(|a| a.iter().any(|t| t.as_str() == Some("Compilation")));
    ReleaseKind {
        secondary_types: g["secondary-types"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|t| t.as_str().map(str::to_string))
            .collect(),
        ..kind(rt, g["first-release-date"].as_str(), comp)
    }
}

/// Keeps the first of each service id: a cursor that repeats an id at a page edge counts it once.
/// Nothing else is merged: two releases with the same title are two releases.
pub fn dedupe(releases: &mut Vec<ArtistRelease>) {
    let mut seen = std::collections::HashSet::new();
    releases.retain(|r| seen.insert(r.hit.id.clone()));
}

/// Newest first: by the release date where the service gives one, else the year; undated last.
/// Stable, so equal dates keep the service's order.
fn newest_first_key(released: Option<&str>, year: Option<i32>) -> Option<String> {
    released
        .map(str::to_string)
        .or_else(|| year.map(|y| format!("{y:04}")))
}

pub fn sort_releases(releases: &mut [ArtistRelease]) {
    releases.sort_by(|a, b| {
        newest_first_key(b.release.released.as_deref(), b.hit.year)
            .cmp(&newest_first_key(a.release.released.as_deref(), a.hit.year))
    });
}

pub fn sort_albums(albums: &mut [ItemAlbum]) {
    albums.sort_by(|a, b| {
        newest_first_key(b.release.released.as_deref(), b.year)
            .cmp(&newest_first_key(a.release.released.as_deref(), a.year))
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn tidal_types_come_from_album_type_then_the_deprecated_type() {
        let t = |v: Value| tidal(&v).release_type;
        assert_eq!(t(json!({"albumType": "ALBUM"})).as_deref(), Some("album"));
        assert_eq!(t(json!({"albumType": "EP"})).as_deref(), Some("ep"));
        assert_eq!(t(json!({"albumType": "SINGLE"})).as_deref(), Some("single"));
        assert_eq!(t(json!({"type": "EP"})).as_deref(), Some("ep"));
        assert_eq!(
            t(json!({"albumType": "SINGLE", "type": "ALBUM"})).as_deref(),
            Some("single")
        );
        // Unknown or absent: nothing made up, whatever the title or track count say
        assert_eq!(t(json!({"albumType": "COMPILATION"})), None);
        assert_eq!(t(json!({"title": "Album", "numberOfItems": 14})), None);
        assert_eq!(t(json!({"title": "X (Single)", "numberOfItems": 1})), None);
        assert_eq!(
            tidal(&json!({"releaseDate": "2013-05-17"})).released.as_deref(),
            Some("2013-05-17")
        );
    }

    #[test]
    fn other_services_types_are_their_own_fields_only() {
        let t = |k: ReleaseKind| (k.release_type, k.compilation);
        let none = (None, false);
        // Spotify: album_type; compilation is a trait, not album/EP/single
        assert_eq!(
            t(spotify(&json!({"album_type": "album"}))),
            (Some("album".into()), false)
        );
        assert_eq!(
            t(spotify(&json!({"album_type": "single", "total_tracks": 6}))),
            (Some("single".into()), false)
        );
        assert_eq!(t(spotify(&json!({"album_type": "compilation"}))), (None, true));
        assert_eq!(
            t(spotify(&json!({"album_group": "album", "name": "Album"}))),
            none,
            "album_group is not a type"
        );
        assert_eq!(
            spotify(&json!({"release_date": "2019-05"})).released.as_deref(),
            Some("2019-05")
        );
        // Deezer: record_type album|ep|single, any case; other values unknown
        assert_eq!(
            t(deezer(&json!({"record_type": "ep"}))),
            (Some("ep".into()), false)
        );
        assert_eq!(
            t(deezer(&json!({"record_type": "EP"}))),
            (Some("ep".into()), false)
        );
        assert_eq!(
            t(deezer(&json!({"record_type": "single"}))),
            (Some("single".into()), false)
        );
        assert_eq!(
            t(deezer(&json!({"record_type": "album"}))),
            (Some("album".into()), false)
        );
        assert_eq!(t(deezer(&json!({"record_type": "compile"}))), none);
        assert_eq!(t(deezer(&json!({"title": "Single", "nb_tracks": 1}))), none);
        assert_eq!(
            deezer(&json!({"release_date": "2001-02-03"})).released.as_deref(),
            Some("2001-02-03")
        );
        // Qobuz: product_type, then release_type; product_type compilation is a trait
        assert_eq!(
            t(qobuz(&json!({"product_type": "single"}))),
            (Some("single".into()), false)
        );
        assert_eq!(
            t(qobuz(&json!({"release_type": "album"}))),
            (Some("album".into()), false)
        );
        assert_eq!(t(qobuz(&json!({"product_type": "compilation"}))), (None, true));
        assert_eq!(
            t(qobuz(&json!({"release_type": "epmini"}))),
            none,
            "unverified values stay unknown"
        );
        // MusicBrainz: primary type; the Compilation secondary type kept
        assert_eq!(
            t(musicbrainz(
                &json!({"primary-type": "Album", "secondary-types": ["Compilation", "Live"]})
            )),
            (Some("album".into()), true)
        );
        assert_eq!(
            t(musicbrainz(&json!({"primary-type": "EP"}))),
            (Some("ep".into()), false)
        );
        assert_eq!(
            t(musicbrainz(
                &json!({"primary-type": "Single", "secondary-types": ["Remix"]})
            )),
            (Some("single".into()), false)
        );
        assert_eq!(t(musicbrainz(&json!({"primary-type": "Broadcast"}))), none);
        assert_eq!(t(musicbrainz(&json!({"primary-type": null}))), none);
        assert_eq!(
            musicbrainz(&json!({"first-release-date": "1999"}))
                .released
                .as_deref(),
            Some("1999")
        );
    }

    #[test]
    fn a_next_link_is_followed_only_to_the_same_list() {
        let list = "/artists/7/relationships/albums";
        let base = "https://openapi.tidal.com/v2";
        let ok = |n: &str| checked_next(base, n, list);
        assert_eq!(
            ok("https://openapi.tidal.com/v2/artists/7/relationships/albums?page%5Bcursor%5D=x").as_deref(),
            Ok("https://openapi.tidal.com/v2/artists/7/relationships/albums?page%5Bcursor%5D=x")
        );
        assert_eq!(
            ok("/artists/7/relationships/albums?page%5Bcursor%5D=x").as_deref(),
            Ok("https://openapi.tidal.com/v2/artists/7/relationships/albums?page%5Bcursor%5D=x"),
            "relative to the API base, as get_json takes it"
        );
        for bad in [
            "http://openapi.tidal.com/v2/artists/7/relationships/albums",
            "https://evil.example/v2/artists/7/relationships/albums",
            "https://openapi.tidal.com:8443/v2/artists/7/relationships/albums",
            "https://openapi.tidal.com.evil.example/v2/artists/7/relationships/albums",
            "https://user:pw@openapi.tidal.com/v2/artists/7/relationships/albums",
            "https://openapi.tidal.com/v2/artists/8/relationships/albums",
            "https://openapi.tidal.com/v2/artists/7/relationships/albumsX",
            "https://openapi.tidal.com/v2/artists/7/relationships/albums/../../../users/me",
            "https://openapi.tidal.com/users/me",
            "//evil.example/v2/artists/7/relationships/albums",
            "/../users/me",
            "javascript:alert(1)",
            "",
        ] {
            assert!(ok(bad).is_err(), "{bad} must be refused");
        }
    }
}
