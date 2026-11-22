//! Item page ids, the same on the node and the phone: `mb-<mbid>`, `isrc-<ISRC>`, a streaming
//! service's own pages as `tidal-<id>` / `deezer-<id>` / `spotify-<id>` / `youtube-<id>`, and any
//! other link as `u-<base64url of the link>` (the web UI makes the same ids).

use base64::Engine;

/// A page id for a link (mirrors the web UI): `mb-<mbid>` for MusicBrainz
/// recordings, artists and release groups, `isrc-<ISRC>`, else
/// `u-<base64url of the link>`.
pub fn page_id(url: &str) -> String {
    for prefix in [
        "https://musicbrainz.org/recording/",
        "https://musicbrainz.org/artist/",
        "https://musicbrainz.org/release-group/",
    ] {
        if let Some(m) = url.strip_prefix(prefix) {
            if pm_model::Mbid::parse(m.trim_end_matches('/')).is_ok() {
                return format!("mb-{}", m.trim_end_matches('/'));
            }
        }
    }
    if let Some(i) = url.strip_prefix("https://musicbrainz.org/isrc/") {
        if pm_model::Isrc::parse(i).is_ok() {
            return format!("isrc-{}", i.to_ascii_uppercase());
        }
    }
    if let Some((svc, id)) = service_page(url) {
        return format!("{svc}-{id}");
    }
    format!(
        "u-{}",
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(url)
    )
}

/// A streaming service's own page of one song, album or artist, as `(service, id)`:
/// `https://tidal.com/browse/track/1` → `("tidal", "1")`. Those pages get readable ids
/// (`/song/tidal-1`); the kind comes from the page (song → track).
pub fn service_page(url: &str) -> Option<(&'static str, String)> {
    let u = url::Url::parse(url).ok()?;
    // A YouTube video: exactly `https://www.youtube.com/watch?v=<11 characters>`
    if u.host_str() == Some("www.youtube.com") && u.path() == "/watch" && u.fragment().is_none() {
        let pairs: Vec<(String, String)> = u
            .query_pairs()
            .map(|(k, v)| (k.into_owned(), v.into_owned()))
            .collect();
        if let [(k, v)] = pairs.as_slice() {
            let ok = k == "v"
                && v.len() == 11
                && v.bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_');
            if ok && url == format!("https://www.youtube.com/watch?v={v}") {
                return Some(("youtube", v.clone()));
            }
        }
        return None;
    }
    if u.query().is_some() || u.fragment().is_some() {
        return None;
    }
    let svc = match u.host_str()? {
        "tidal.com" | "listen.tidal.com" | "www.tidal.com" => "tidal",
        "www.deezer.com" | "deezer.com" => "deezer",
        "open.spotify.com" => "spotify",
        _ => return None,
    };
    let segs: Vec<&str> = u.path_segments()?.filter(|x| !x.is_empty()).collect();
    // Optional `browse/` (TIDAL) or a language (`en/`, Deezer) before `<kind>/<id>`
    let (kind, id) = match segs.as_slice() {
        [k, id] => (*k, *id),
        [_, k, id] => (*k, *id),
        _ => return None,
    };
    if !matches!(kind, "track" | "album" | "artist") {
        return None;
    }
    let ok = match svc {
        "spotify" => id.len() == 22 && id.bytes().all(|b| b.is_ascii_alphanumeric()),
        _ => !id.is_empty() && id.len() <= 20 && id.bytes().all(|b| b.is_ascii_digit()),
    };
    ok.then(|| (svc, id.to_string()))
}

/// The link a page id names (`mb-`, `isrc-`, `tidal-`, `deezer-`, `spotify-`, `u-`); `kind`
/// picks the MusicBrainz entity for `mb-` and the service's kind for the others.
pub fn url_of_page(kind: &str, id: &str) -> Option<String> {
    if let Some(m) = id.strip_prefix("mb-") {
        pm_model::Mbid::parse(m).ok()?;
        let entity = match kind {
            "artist" => "artist",
            "album" => "release-group",
            _ => "recording",
        };
        return Some(format!("https://musicbrainz.org/{entity}/{m}"));
    }
    if let Some(i) = id.strip_prefix("isrc-") {
        pm_model::Isrc::parse(i).ok()?;
        return Some(format!("https://musicbrainz.org/isrc/{i}"));
    }
    let skind = match kind {
        "artist" => "artist",
        "album" => "album",
        _ => "track",
    };
    if let Some(v) = id.strip_prefix("youtube-") {
        let url = format!("https://www.youtube.com/watch?v={v}");
        return (page_id(&url) == id).then_some(url);
    }
    for (svc, base) in [
        ("tidal-", "https://tidal.com"),
        ("deezer-", "https://www.deezer.com"),
        ("spotify-", "https://open.spotify.com"),
    ] {
        if let Some(n) = id.strip_prefix(svc) {
            let url = format!("{base}/{skind}/{n}");
            // Only ids that map back to themselves (no paths or odd characters)
            return (page_id(&url) == id).then_some(url);
        }
    }
    let b = id.strip_prefix("u-")?;
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(b).ok()?;
    let u = String::from_utf8(bytes).ok()?;
    url::Url::parse(&u).ok().map(|_| u)
}

/// The service page of a queue URI (`tidal:track:1` → `https://tidal.com/track/1`).
pub fn page_of_queue_uri(uri: &str) -> Option<String> {
    let (svc, rest) = uri.split_once(':')?;
    let (kind, id) = rest.split_once(':')?;
    Some(match svc {
        "tidal" => format!("https://tidal.com/{kind}/{id}"),
        "deezer" => format!("https://www.deezer.com/{kind}/{id}"),
        "spotify" => format!("https://open.spotify.com/{kind}/{id}"),
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn service_pages_get_readable_ids() {
        assert_eq!(page_id("https://tidal.com/browse/track/12345"), "tidal-12345");
        assert_eq!(page_id("https://tidal.com/track/12345"), "tidal-12345");
        assert_eq!(page_id("https://www.deezer.com/en/track/987"), "deezer-987");
        assert_eq!(
            page_id("https://open.spotify.com/track/4uLU6hMCjMI75M1A2tKUQC"),
            "spotify-4uLU6hMCjMI75M1A2tKUQC"
        );
        assert_eq!(
            url_of_page("song", "tidal-12345").as_deref(),
            Some("https://tidal.com/track/12345")
        );
        assert_eq!(
            url_of_page("album", "deezer-55").as_deref(),
            Some("https://www.deezer.com/album/55")
        );
        assert_eq!(url_of_page("song", "tidal-1/../x"), None);
        assert_eq!(
            page_id("https://www.youtube.com/watch?v=bSnlKl_PoQU"),
            "youtube-bSnlKl_PoQU"
        );
        assert_eq!(
            url_of_page("song", "youtube-bSnlKl_PoQU").as_deref(),
            Some("https://www.youtube.com/watch?v=bSnlKl_PoQU")
        );
        assert!(page_id("https://www.youtube.com/watch?v=bSnlKl_PoQU&t=1").starts_with("u-"));
        // Anything else keeps the base64 id
        assert!(page_id("https://tidal.com/track/1?x=1").starts_with("u-"));
        assert!(page_id("https://example.org/track/1").starts_with("u-"));
    }
}
