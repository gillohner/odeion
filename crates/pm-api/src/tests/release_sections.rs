//! Artist release sections: an artist's whole catalogue on a service, each release with the type
//! the service gives it (album, EP, single; none when the service says nothing), newest first,
//! with an honest account of what was fetched (`catalogue`). Mock services only.

use super::*;

/// A synthetic TIDAL v2 artist catalogue, served the way the official API does: the artist's
/// album relationship in cursor pages of 20 (no filter, no page size, no total), then the albums
/// themselves by `filter[id]`, at most 20 per call.
#[derive(Clone)]
struct TidalCatalogue {
    /// Relationship order (TIDAL's own, not by date).
    ids: Vec<String>,
    albums: std::collections::HashMap<String, serde_json::Value>,
    /// A relationship page (0-based) that fails with 500 for this artist.
    broken_page: Option<usize>,
    /// The last id of each page is repeated at the start of the next (an overlapping cursor).
    overlap: bool,
    /// Page 0's `links.next`, as sent (`{base}` is this mock's own address): a link elsewhere.
    next_override: Option<String>,
    /// A relationship page that answers only after this many milliseconds.
    slow_page: Option<(usize, u64)>,
    /// From this page on, pages repeat page 0's ids and still say there is a next page.
    stuck_from: Option<usize>,
    /// Ids the relationship lists but `/albums` doesn't return.
    hide_details: Vec<String>,
    /// A cursor that never ends: one new id per page, always a next page.
    endless: bool,
    /// The first relationship call is answered 429 with `Retry-After: 120`.
    rate_limit_first: bool,
}

const PAGE: usize = 20;

/// `n` releases: every fifth an ALBUM, then an EP, two SINGLEs and one with a type only in the
/// deprecated `type` attribute (even tens) or none at all (odd tens). Dates are scrambled
/// against the relationship order. Titles hold the traps a title-based guess would fall into.
fn catalogue(n: usize) -> TidalCatalogue {
    let mut ids = Vec::new();
    let mut albums = std::collections::HashMap::new();
    for i in 0..n {
        let id = (9_000_000 + i).to_string();
        let k = (i * 37) % n.max(1);
        let date = format!("{}-{:02}-{:02}", 1990 + k / 12, k % 12 + 1, 1 + k % 28);
        let title = match i {
            0 => "Night Drive".to_string(),
            5 => "Night Drive".to_string(),
            10 => "Night Drive (Deluxe Edition)".to_string(),
            2 => "Night Drive (Remix)".to_string(),
            3 => "Album".to_string(),
            1 => "Greatest Hits".to_string(),
            _ => format!("Release {i}"),
        };
        let mut attrs = serde_json::json!({"title": title, "releaseDate": date, "explicit": i == 5,
            "numberOfItems": if i % 5 == 0 { 1 } else { 12 }, "barcodeId": format!("0000000{i:05}")});
        match i % 5 {
            0 => attrs["albumType"] = "ALBUM".into(),
            1 => attrs["albumType"] = "EP".into(),
            2 | 3 => attrs["albumType"] = "SINGLE".into(),
            _ if i % 10 == 4 => attrs["type"] = "EP".into(),
            _ => {}
        }
        albums.insert(
            id.clone(),
            serde_json::json!({"id": id, "type": "albums", "attributes": attrs,
                "relationships": {"artists": {"data": [{"id": "4805908", "type": "artists"}]}}}),
        );
        ids.push(id);
    }
    TidalCatalogue {
        ids,
        albums,
        broken_page: None,
        overlap: false,
        next_override: None,
        slow_page: None,
        stuck_from: None,
        hide_details: Vec::new(),
        endless: false,
        rate_limit_first: false,
    }
}

/// A mock TIDAL (token, artists, album relationship pages, albums by id) for several artists.
async fn mock_tidal(
    artists: Vec<(&'static str, TidalCatalogue)>,
) -> (String, Arc<std::sync::Mutex<Vec<String>>>) {
    use axum::extract::RawQuery;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());
    let seen = Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
    let cats: Arc<std::collections::HashMap<String, TidalCatalogue>> =
        Arc::new(artists.into_iter().map(|(a, c)| (a.to_string(), c)).collect());
    let pairs = |q: &Option<String>| -> Vec<(String, String)> {
        url::form_urlencoded::parse(q.as_deref().unwrap_or("").as_bytes())
            .into_owned()
            .collect()
    };
    let (c1, c2, c3) = (cats.clone(), cats.clone(), cats.clone());
    let (b1, s1, s2, s3) = (base.clone(), seen.clone(), seen.clone(), seen.clone());
    let mock = Router::new()
        .route(
            "/v1/oauth2/token",
            post(|| async { Json(serde_json::json!({"access_token": "app-token", "expires_in": 14400})) }),
        )
        .route(
            "/artists",
            get(move |RawQuery(q): RawQuery| {
                let cats = c1.clone();
                async move {
                    let data: Vec<serde_json::Value> = pairs(&q)
                        .into_iter()
                        .filter(|(k, v)| k == "filter[id]" && cats.contains_key(v))
                        .map(|(_, v)| serde_json::json!({"id": v, "type": "artists", "attributes": {"name": format!("Synthetic {v}")}}))
                        .collect();
                    Json(serde_json::json!({"data": data}))
                }
            }),
        )
        .route(
            "/artists/{id}/relationships/albums",
            get(move |Path(id): Path<String>, RawQuery(q): RawQuery| {
                let (cats, base, seen) = (c2.clone(), b1.clone(), s1.clone());
                async move {
                    let n_before = {
                        let mut seen = seen.lock().unwrap();
                        let n = seen.iter().filter(|x| x.starts_with(&format!("rel {id} "))).count();
                        seen.push(format!("rel {id} {}", q.clone().unwrap_or_default()));
                        n
                    };
                    let Some(c) = cats.get(&id) else {
                        return (StatusCode::NOT_FOUND, Json(serde_json::json!({"errors": [{"detail": "no artist"}]}))).into_response();
                    };
                    if c.rate_limit_first && n_before == 0 {
                        return (StatusCode::TOO_MANY_REQUESTS, [(axum::http::header::RETRY_AFTER, "120")], Json(serde_json::json!({"errors": [{"detail": "slow down"}]}))).into_response();
                    }
                    let q = pairs(&q);
                    assert!(
                        q.iter().all(|(k, _)| ["countryCode", "page[cursor]", "include"].contains(&k.as_str())),
                        "TIDAL rejects undocumented parameters with 400: {q:?}"
                    );
                    let page: usize = q
                        .iter()
                        .find(|(k, _)| k == "page[cursor]")
                        .map(|(_, v)| v.trim_start_matches("cur").parse().unwrap())
                        .unwrap_or(0);
                    if let Some((p, ms)) = c.slow_page {
                        if p == page {
                            tokio::time::sleep(std::time::Duration::from_millis(ms)).await;
                        }
                    }
                    if c.broken_page == Some(page) {
                        return (
                            StatusCode::INTERNAL_SERVER_ERROR,
                            Json(serde_json::json!({"errors": [{"detail": "upstream hiccup"}]})),
                        )
                            .into_response();
                    }
                    let next_of = |p: usize| {
                        format!("{base}/artists/{id}/relationships/albums?countryCode=US&page%5Bcursor%5D=cur{p}")
                    };
                    if c.endless {
                        let data = serde_json::json!([{"id": (8_000_000 + page).to_string(), "type": "albums"}]);
                        return Json(serde_json::json!({"data": data, "links": {"next": next_of(page + 1)}})).into_response();
                    }
                    if c.stuck_from.is_some_and(|k| page >= k) {
                        let data: Vec<serde_json::Value> = c.ids[..PAGE.min(c.ids.len())]
                            .iter()
                            .map(|i| serde_json::json!({"id": i, "type": "albums"}))
                            .collect();
                        return Json(serde_json::json!({"data": data, "links": {"next": next_of(page + 1)}})).into_response();
                    }
                    let start = (page * PAGE).saturating_sub(usize::from(c.overlap && page > 0));
                    let end = ((page + 1) * PAGE).min(c.ids.len());
                    let data: Vec<serde_json::Value> = c.ids[start.min(end)..end]
                        .iter()
                        .map(|i| serde_json::json!({"id": i, "type": "albums"}))
                        .collect();
                    let mut links = serde_json::json!({"self": format!("/artists/{id}/relationships/albums")});
                    if end < c.ids.len() {
                        links["next"] = next_of(page + 1).into();
                    }
                    if let (0, Some(o)) = (page, &c.next_override) {
                        links["next"] = o.replace("{base}", &base).into();
                    }
                    (StatusCode::OK, Json(serde_json::json!({"data": data, "links": links}))).into_response()
                }
            }),
        )
        .route(
            "/albums",
            get(move |RawQuery(q): RawQuery| {
                let (cats, seen) = (c3.clone(), s2.clone());
                async move {
                    let want: Vec<String> = pairs(&q)
                        .into_iter()
                        .filter(|(k, _)| k == "filter[id]")
                        .map(|(_, v)| v)
                        .collect();
                    seen.lock().unwrap().push(format!("albums {}", want.len()));
                    if want.is_empty() || want.len() > 20 {
                        return (
                            StatusCode::BAD_REQUEST,
                            Json(serde_json::json!({"errors": [{"detail": "filter[id] takes 1 to 20 ids"}]})),
                        );
                    }
                    let data: Vec<serde_json::Value> = want
                        .iter()
                        .filter(|w| !cats.values().any(|c| c.hide_details.contains(w)))
                        .filter_map(|w| {
                            cats.values().find_map(|c| c.albums.get(w).cloned()).or_else(|| {
                                w.starts_with("80").then(|| serde_json::json!({"id": w, "type": "albums",
                                    "attributes": {"title": format!("Endless {w}"), "albumType": "SINGLE", "releaseDate": "2001-01-01"}}))
                            })
                        })
                        .collect();
                    (StatusCode::OK, Json(serde_json::json!({"data": data})))
                }
            }),
        )
        .fallback(move |uri: axum::http::Uri| {
            let seen = s3.clone();
            async move {
                seen.lock().unwrap().push(format!("other {uri}"));
                StatusCode::NOT_FOUND
            }
        });
    tokio::spawn(async move { axum::serve(listener, mock).await.unwrap() });
    (base, seen)
}

/// Somewhere a catalogue's `next` link must never lead: it counts every request and every
/// `Authorization` header it receives.
async fn elsewhere() -> (String, Arc<std::sync::Mutex<Vec<String>>>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());
    let got = Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
    let g = got.clone();
    let app = Router::new().fallback(move |headers: axum::http::HeaderMap, uri: axum::http::Uri| {
        let g = g.clone();
        async move {
            let auth = headers
                .get(axum::http::header::AUTHORIZATION)
                .map(|v| v.to_str().unwrap_or("?").to_string())
                .unwrap_or_default();
            g.lock().unwrap().push(format!("{uri} auth={auth}"));
            Json(serde_json::json!({"data": [], "items": []}))
        }
    });
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (base, got)
}

async fn tidal_state(dir: &std::path::Path, base: &str) -> Arc<NodeState> {
    std::fs::create_dir_all(dir).unwrap();
    let s = state(dir, None).await;
    {
        let mut ep = s.tidal.write().unwrap();
        ep.api_base = base.to_string();
        ep.auth_base = base.to_string();
        ep.login_base = base.to_string();
        ep.creds = tidal::TidalCredentials {
            client_id: "cid".into(),
            client_secret: "sec".into(),
            scopes: String::new(),
        };
    }
    s
}

/// The item page once the service's list has arrived (the page answers `pending` past its
/// budget and the fetch goes on in the background).
async fn settled_page(app: &Router, path: &str) -> serde_json::Value {
    for _ in 0..300 {
        let (st, pg) = call(app, "GET", path, None).await;
        assert_eq!(st, StatusCode::OK, "{pg}");
        if pg["pending"] != true {
            return pg;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    panic!("{path} stayed pending");
}

fn types_of(albums: &serde_json::Value) -> std::collections::BTreeMap<String, usize> {
    let mut m = std::collections::BTreeMap::new();
    for a in albums.as_array().unwrap() {
        let t = a["release_type"].as_str().unwrap_or("unknown").to_string();
        *m.entry(t).or_insert(0) += 1;
    }
    m
}

fn assert_newest_first(albums: &serde_json::Value) {
    let dates: Vec<&str> = albums
        .as_array()
        .unwrap()
        .iter()
        .map(|a| a["released"].as_str().unwrap_or(""))
        .collect();
    for w in dates.windows(2) {
        assert!(w[0] >= w[1], "newest first: {} before {}", w[0], w[1]);
    }
}

/// The owner's report (/artist/tidal-4805908): "Albums 60" mixed every single and remix in. The
/// whole catalogue arrives (75 releases over four cursor pages, a repeated id at a page edge
/// counted once), each with TIDAL's own `albumType` (the deprecated `type` when that is all
/// there is; nothing when TIDAL says nothing), newest first. Same-titled editions, an explicit
/// twin, a remix single and titles like "Album" or "Greatest Hits" stay what TIDAL says they are.
#[tokio::test]
async fn a_tidal_artist_lists_its_whole_catalogue_by_release_type() {
    let mut c = catalogue(75);
    c.overlap = true;
    let (base, seen) = mock_tidal(vec![("4805908", c)]).await;
    let dir = tempfile::tempdir().unwrap();
    let s = tidal_state(dir.path(), &base).await;
    let app = router(s.clone());

    let (st, a) = call(&app, "GET", "/v1/sources/tidal/artists/artist:4805908", None).await;
    assert_eq!(st, StatusCode::OK, "{a}");
    let albums = &a["albums"];
    assert_eq!(
        albums.as_array().unwrap().len(),
        75,
        "every release, past the first 60: {a}"
    );
    let want: std::collections::BTreeMap<String, usize> =
        [("album", 15), ("ep", 23), ("single", 30), ("unknown", 7)]
            .into_iter()
            .map(|(k, v)| (k.to_string(), v))
            .collect();
    assert_eq!(types_of(albums), want);
    assert_newest_first(albums);
    let by_id = |id: &str| {
        albums
            .as_array()
            .unwrap()
            .iter()
            .find(|x| x["id"] == format!("album:{id}"))
            .cloned()
            .unwrap_or_else(|| panic!("album:{id} missing"))
    };
    // Same title, different releases: both kept (no edition/explicit de-duplication)
    assert_eq!(by_id("9000000")["title"], "Night Drive");
    assert_eq!(by_id("9000005")["title"], "Night Drive");
    assert_eq!(by_id("9000000")["release_type"], "album");
    assert_eq!(
        by_id("9000010")["release_type"],
        "album",
        "a deluxe edition is still TIDAL's album"
    );
    assert_eq!(by_id("9000002")["release_type"], "single", "a remix single");
    assert_eq!(
        by_id("9000003")["release_type"],
        "single",
        "a single titled \"Album\""
    );
    assert_eq!(
        by_id("9000001")["release_type"],
        "ep",
        "an EP titled \"Greatest Hits\""
    );
    assert_eq!(
        by_id("9000004")["release_type"],
        "ep",
        "the deprecated `type` when albumType is absent"
    );
    assert!(
        by_id("9000009").get("release_type").is_none(),
        "no type from TIDAL: none made up"
    );
    assert!(
        by_id("9000000").get("compilation").is_none(),
        "TIDAL supplies no compilation trait"
    );
    assert_eq!(by_id("9000000")["released"], "1990-01-01");
    assert_eq!(a["catalogue"]["source"], "tidal");
    assert_eq!(a["catalogue"]["complete"], true, "{}", a["catalogue"]);
    assert!(a["catalogue"].get("truncated").is_none_or(|t| t == false));
    assert!(a["catalogue"].get("error").is_none());
    let calls = seen.lock().unwrap().clone();
    assert_eq!(
        calls.iter().filter(|c| c.starts_with("rel 4805908")).count(),
        4,
        "{calls:?}"
    );
    assert!(calls
        .iter()
        .filter(|c| c.starts_with("albums "))
        .all(|c| c != "albums 0"));

    // The item page carries the same: every release, its type and date, newest first
    let pg = settled_page(&app, "/v1/item/artist/tidal-4805908").await;
    assert_eq!(pg["albums"].as_array().unwrap().len(), 75, "{pg}");
    assert_eq!(types_of(&pg["albums"]), want);
    assert_newest_first(&pg["albums"]);
    let night: Vec<&serde_json::Value> = pg["albums"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|x| x["title"] == "Night Drive")
        .collect();
    assert_eq!(night.len(), 2, "both Night Drive releases");
    assert!(night.iter().all(|x| x["release_type"] == "album"));
    assert_eq!(pg["catalogue"]["complete"], true, "{}", pg["catalogue"]);
    assert_eq!(pg["catalogue"]["source"], "tidal");
    // And the cached page (a second look) is the same list
    let again = settled_page(&app, "/v1/item/artist/tidal-4805908").await;
    assert_eq!(again["albums"], pg["albums"]);
    assert_eq!(again["catalogue"], pg["catalogue"]);
}

/// A page that fails part-way: what arrived is listed, the page says it is incomplete and why.
/// The partial list is kept only briefly (polls don't crawl the service again; a later look
/// does), never for the day a complete list is kept.
#[tokio::test]
async fn a_failed_catalogue_page_is_reported_and_kept_only_briefly() {
    let mut c = catalogue(75);
    c.broken_page = Some(2);
    let (base, seen) = mock_tidal(vec![("4805909", c)]).await;
    let dir = tempfile::tempdir().unwrap();
    let s = tidal_state(dir.path(), &base).await;
    let app = router(s.clone());
    let (st, a) = call(&app, "GET", "/v1/sources/tidal/artists/artist:4805909", None).await;
    assert_eq!(st, StatusCode::OK, "{a}");
    assert_eq!(
        a["albums"].as_array().unwrap().len(),
        40,
        "the two pages that arrived: {a}"
    );
    assert_eq!(a["catalogue"]["complete"], false);
    assert!(
        a["catalogue"]["error"].as_str().is_some_and(|e| !e.is_empty()),
        "{}",
        a["catalogue"]
    );

    let pg = settled_page(&app, "/v1/item/artist/tidal-4805909").await;
    assert_eq!(pg["albums"].as_array().unwrap().len(), 40, "{pg}");
    assert_eq!(pg["catalogue"]["complete"], false);
    assert!(pg["catalogue"]["error"].is_string());
    let key = "item.contents.https://tidal.com/artist/4805909";
    let kept = s
        .library
        .lock()
        .unwrap()
        .get_setting(key)
        .unwrap()
        .expect("kept briefly");
    let (at, c): (i64, serde_json::Value) = serde_json::from_str(&kept).unwrap();
    assert!(c["catalogue"]["error"].is_string(), "{c}");
    let rels = |seen: &Arc<std::sync::Mutex<Vec<String>>>| {
        seen.lock()
            .unwrap()
            .iter()
            .filter(|x| x.starts_with("rel 4805909 "))
            .count()
    };
    let before = rels(&seen);
    let again = settled_page(&app, "/v1/item/artist/tidal-4805909").await;
    assert_eq!(again["catalogue"], pg["catalogue"]);
    assert_eq!(
        rels(&seen),
        before,
        "a look within the short time is answered from the cache"
    );
    // Past the short time it is fetched again (a complete list would still be kept for the day)
    let old = serde_json::json!([at - crate::discover::ERROR_TTL_SECS - 1, c]);
    s.library
        .lock()
        .unwrap()
        .set_setting(key, &old.to_string())
        .unwrap();
    settled_page(&app, "/v1/item/artist/tidal-4805909").await;
    assert!(rels(&seen) > before, "an old error is not kept");
}

/// The fetch is bounded: past the bound the page lists the newest of what it fetched and says
/// the service has more, never that the list is complete.
#[tokio::test]
async fn a_catalogue_past_the_bound_says_it_is_truncated() {
    let n = crate::releases::MAX_RELEASES + 7;
    let (base, _) = mock_tidal(vec![("4805910", catalogue(n))]).await;
    let dir = tempfile::tempdir().unwrap();
    let s = tidal_state(dir.path(), &base).await;
    let app = router(s.clone());
    let (st, a) = call(&app, "GET", "/v1/sources/tidal/artists/artist:4805910", None).await;
    assert_eq!(st, StatusCode::OK, "{a}");
    assert_eq!(
        a["albums"].as_array().unwrap().len(),
        crate::releases::MAX_RELEASES
    );
    assert_eq!(a["catalogue"]["complete"], false, "{}", a["catalogue"]);
    assert_eq!(a["catalogue"]["truncated"], true);
    assert!(a["catalogue"].get("error").is_none());
}

/// Contents cached before release types existed (no `catalogue`) are not shown: the page fetches
/// the catalogue again instead of showing an untyped, cut-off list for a day.
#[tokio::test]
async fn an_artist_cached_without_a_catalogue_is_fetched_again() {
    let (base, _) = mock_tidal(vec![("4805911", catalogue(25))]).await;
    let dir = tempfile::tempdir().unwrap();
    let s = tidal_state(dir.path(), &base).await;
    let old = serde_json::json!([pm_library::db::now_unix(), {"kind": "artist", "total_ms": 1000,
        "tracks": [{"title": "Old top song", "duration_ms": 1000}],
        "albums": [{"id": "tidal-1", "title": "Old untyped", "page": "https://tidal.com/album/1"}]}]);
    s.library
        .lock()
        .unwrap()
        .set_setting("item.contents.https://tidal.com/artist/4805911", &old.to_string())
        .unwrap();
    let app = router(s.clone());
    let pg = settled_page(&app, "/v1/item/artist/tidal-4805911").await;
    assert_eq!(pg["albums"].as_array().unwrap().len(), 25, "{pg}");
    assert!(pg["albums"]
        .as_array()
        .unwrap()
        .iter()
        .all(|a| a["title"] != "Old untyped"));
    assert_eq!(pg["catalogue"]["complete"], true);
}

/// Spotify's artist albums (developer.spotify.com, Get Artist's Albums): `limit` 0–10, paged by
/// `next` with a `total`; `album_type` album|single|compilation. `album_group` was removed for
/// development-mode apps in February 2026, so nothing may depend on it. Every page is followed
/// and the type is Spotify's own.
#[tokio::test]
async fn a_spotify_artist_pages_ten_at_a_time_and_keeps_album_type() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());
    let b2 = base.clone();
    let kinds = ["album", "single", "compilation"];
    let mock = Router::new()
        .route(
            "/api/token",
            post(|| async { Json(serde_json::json!({"access_token": "app", "expires_in": 3600})) }),
        )
        .route(
            "/artists/{id}",
            get(|Path(id): Path<String>| async move { Json(serde_json::json!({"id": id, "name": "Synthetic Spotify"})) }),
        )
        .route(
            "/artists/{id}/albums",
            get(move |Path(id): Path<String>, Query(q): Query<std::collections::HashMap<String, String>>| {
                let base = b2.clone();
                async move {
                    let limit: usize = q.get("limit").and_then(|l| l.parse().ok()).unwrap_or(5);
                    if limit > 10 {
                        return (StatusCode::BAD_REQUEST, Json(serde_json::json!({"error": {"status": 400, "message": "Invalid limit"}})));
                    }
                    assert_eq!(q.get("include_groups").map(String::as_str), Some("album,single"), "{q:?}");
                    let offset: usize = q.get("offset").and_then(|o| o.parse().ok()).unwrap_or(0);
                    let total = 23;
                    let items: Vec<serde_json::Value> = (offset..(offset + limit).min(total))
                        .map(|i| {
                            let date = match i % 3 { 0 => format!("{}", 2000 + i), 1 => format!("{}-0{}", 2000 + i, 1 + i % 9), _ => format!("{}-05-17", 2000 + i) };
                            serde_json::json!({"id": format!("sp{i:020}"), "name": if i == 4 { "Album".to_string() } else { format!("Spot {i}") },
                                "album_type": kinds[i % 3], "release_date": date, "release_date_precision": "day", "total_tracks": 1, "artists": [{"name": "Synthetic Spotify"}], "images": []})
                        })
                        .collect();
                    let next = (offset + limit < total)
                        .then(|| format!("{base}/artists/{id}/albums?offset={}&limit={limit}&include_groups=album,single", offset + limit));
                    (StatusCode::OK, Json(serde_json::json!({"items": items, "total": total, "limit": limit, "offset": offset, "next": next})))
                }
            }),
        );
    tokio::spawn(async move { axum::serve(listener, mock).await.unwrap() });
    let dir = tempfile::tempdir().unwrap();
    let s = state(dir.path(), None).await;
    {
        let mut ep = s.spotify.write().unwrap();
        ep.api_base = base.clone();
        ep.accounts_base = base.clone();
        ep.creds = spotify::SpotifyCredentials {
            client_id: "cid".into(),
            client_secret: "sec".into(),
        };
    }
    let app = router(s.clone());
    let (st, a) = call(
        &app,
        "GET",
        "/v1/sources/spotify/artists/artist:4Z8W4fKeB5YxbusRsdQVPb",
        None,
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{a}");
    let albums = a["albums"].as_array().unwrap();
    assert_eq!(albums.len(), 23, "every page: {a}");
    let want: std::collections::BTreeMap<String, usize> = [("album", 8), ("single", 8), ("unknown", 7)]
        .into_iter()
        .map(|(k, v)| (k.to_string(), v))
        .collect();
    assert_eq!(types_of(&a["albums"]), want);
    let comp = albums.iter().filter(|x| x["compilation"] == true).count();
    assert_eq!(comp, 7, "Spotify's compilation type is kept as a trait");
    let titled = albums.iter().find(|x| x["title"] == "Album").unwrap();
    assert_eq!(titled["release_type"], "single", "a single titled Album");
    assert_newest_first(&a["albums"]);
    assert_eq!(a["catalogue"]["source"], "spotify");
    assert_eq!(a["catalogue"]["complete"], true, "{}", a["catalogue"]);
    assert_eq!(a["catalogue"]["total"], 23);
}

/// Deezer's artist albums: `index`/`limit` pages with a `total`; `record_type` on each album.
/// Only album, ep and single are types; anything else stays unknown.
#[tokio::test]
async fn a_deezer_artist_pages_past_one_hundred_and_keeps_record_type() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let types = ["album", "ep", "single", "compile"];
    let mock = Router::new()
        .route(
            "/artist/{id}",
            get(|Path(id): Path<u64>| async move { Json(serde_json::json!({"id": id, "name": "Synthetic Deezer"})) }),
        )
        .route(
            "/artist/{id}/albums",
            get(move |Query(q): Query<std::collections::HashMap<String, String>>| async move {
                let limit: usize = q.get("limit").and_then(|l| l.parse().ok()).unwrap_or(25);
                let index: usize = q.get("index").and_then(|o| o.parse().ok()).unwrap_or(0);
                let total = 130;
                let data: Vec<serde_json::Value> = (index..(index + limit).min(total))
                    .map(|i| serde_json::json!({"id": 700_000 + i, "title": format!("Dz {i}"), "record_type": types[i % 4],
                        "release_date": format!("{}-01-{:02}", 1990 + i / 28, 1 + i % 28)}))
                    .collect();
                let next = (index + limit < total).then(|| format!("https://api.deezer.com/artist/1/albums?index={}", index + limit));
                Json(serde_json::json!({"data": data, "total": total, "next": next}))
            }),
        );
    tokio::spawn(async move { axum::serve(listener, mock).await.unwrap() });
    let dir = tempfile::tempdir().unwrap();
    let s = state(dir.path(), None).await;
    *s.deezer_base.write().unwrap() = format!("http://127.0.0.1:{port}");
    let app = router(s.clone());
    let (st, a) = call(&app, "GET", "/v1/sources/deezer/artists/artist:27", None).await;
    assert_eq!(st, StatusCode::OK, "{a}");
    assert_eq!(
        a["albums"].as_array().unwrap().len(),
        130,
        "past the first page of 100: {a}"
    );
    let want: std::collections::BTreeMap<String, usize> =
        [("album", 33), ("ep", 33), ("single", 32), ("unknown", 32)]
            .into_iter()
            .map(|(k, v)| (k.to_string(), v))
            .collect();
    assert_eq!(types_of(&a["albums"]), want);
    assert!(
        a["albums"]
            .as_array()
            .unwrap()
            .iter()
            .all(|x| x.get("compilation").is_none()),
        "no trait from an unverified value"
    );
    assert_newest_first(&a["albums"]);
    assert_eq!(a["catalogue"]["source"], "deezer");
    assert_eq!(a["catalogue"]["complete"], true, "{}", a["catalogue"]);
    assert_eq!(a["catalogue"]["total"], 130);
}

/// MusicBrainz (no service lists the artist): every release group, of every type, page by page
/// (`limit=100&offset=`, `release-group-count`), typed by its primary type; the Compilation
/// secondary type is kept. Before, only primary-type Album without secondary types, one page.
#[tokio::test]
async fn a_musicbrainz_artist_pages_every_release_group_type() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let asked = Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
    let a2 = asked.clone();
    let prim = [Some("Album"), Some("EP"), Some("Single"), Some("Broadcast"), None];
    let mock = Router::new().route(
        "/ws/2/release-group",
        get(move |Query(q): Query<std::collections::HashMap<String, String>>| {
            let asked = a2.clone();
            async move {
                asked.lock().unwrap().push(format!("{q:?}"));
                let offset: usize = q.get("offset").and_then(|o| o.parse().ok()).unwrap_or(0);
                let limit: usize = q.get("limit").and_then(|o| o.parse().ok()).unwrap_or(25).min(100);
                let total = 230;
                let groups: Vec<serde_json::Value> = (offset..(offset + limit).min(total))
                    .map(|i| serde_json::json!({"id": format!("00000000-0000-0000-0000-{i:012}"), "title": format!("Rg {i}"),
                        "primary-type": prim[i % 5], "secondary-types": if i % 7 == 0 { vec!["Compilation"] } else if i % 7 == 1 { vec!["Live"] } else { vec![] },
                        "first-release-date": format!("{}-03-01", 1960 + i / 4)}))
                    .collect();
                Json(serde_json::json!({"release-group-count": total, "release-group-offset": offset, "release-groups": groups}))
            }
        }),
    );
    tokio::spawn(async move { axum::serve(listener, mock).await.unwrap() });
    let dir = tempfile::tempdir().unwrap();
    let s = state(dir.path(), None).await;
    s.set_contact("test@example.org").unwrap();
    *s.mb_base.write().unwrap() = format!("http://127.0.0.1:{port}/ws/2");
    let (_, albums, cat) =
        crate::discover::musicbrainz_contents(&s, "artist", "b10bbbfc-cf9e-42e0-be17-e2c3e1d2600d").await;
    assert_eq!(albums.len(), 230, "every page, every type");
    let mut types = std::collections::BTreeMap::new();
    for a in &albums {
        *types
            .entry(a.release.release_type.clone().unwrap_or("unknown".into()))
            .or_insert(0) += 1;
    }
    let want: std::collections::BTreeMap<String, usize> =
        [("album", 46), ("ep", 46), ("single", 46), ("unknown", 92)]
            .into_iter()
            .map(|(k, v)| (k.to_string(), v))
            .collect();
    assert_eq!(types, want);
    assert_eq!(albums.iter().filter(|a| a.release.compilation).count(), 33);
    // Every secondary type is kept as MusicBrainz names it (a live album is still an album)
    let live: Vec<_> = albums
        .iter()
        .filter(|a| a.release.secondary_types == ["Live"])
        .collect();
    assert_eq!(live.len(), 33);
    assert!(albums
        .iter()
        .filter(|a| a.release.compilation)
        .all(|a| a.release.secondary_types == ["Compilation"]));
    assert_eq!(
        albums
            .iter()
            .filter(|a| a.release.secondary_types.is_empty())
            .count(),
        230 - 66
    );
    let cat = cat.expect("a catalogue");
    assert_eq!(
        (cat.source.as_str(), cat.complete, cat.total),
        ("musicbrainz", true, Some(230))
    );
    let asked = asked.lock().unwrap().clone();
    assert!(
        asked.iter().all(|q| !q.contains("\"type\"")),
        "no type filter: {asked:?}"
    );
    for w in albums.windows(2) {
        assert!(w[0].release.released >= w[1].release.released, "newest first");
    }
}

/// A section's "View all" page has a path of its own: a reload or a shared link opens it.
#[tokio::test]
async fn a_release_section_path_serves_the_web_app() {
    let dir = tempfile::tempdir().unwrap();
    let s = state(dir.path(), None).await;
    let app = router(s);
    for path in [
        "/artist/tidal-4805908/releases/albums",
        "/artist/lib-3/releases/eps-singles?page=2",
        "/artist/mb-b10bbbfc-cf9e-42e0-be17-e2c3e1d2600d/releases/other",
    ] {
        let req = Request::builder().uri(path).body(Body::empty()).unwrap();
        let resp = app.clone().oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK, "{path}");
        let body = resp.into_body().collect().await.unwrap().to_bytes();
        assert!(
            String::from_utf8_lossy(&body).contains("<html"),
            "{path} serves the app"
        );
    }
}

/// H1: a catalogue's `next` link is followed only to the same service, the same scheme and the
/// same artist's list. A link elsewhere, a plain-http link or another path gets no request and
/// no token; the list says it stopped and why.
#[tokio::test]
async fn a_catalogue_next_link_elsewhere_gets_no_request_and_no_token() {
    let (bad, got) = elsewhere().await;
    let mk = |next: String| {
        let mut c = catalogue(45);
        c.next_override = Some(next);
        c
    };
    let (base, seen) = mock_tidal(vec![
        (
            "4805930",
            mk(format!(
                "{bad}/artists/4805930/relationships/albums?page%5Bcursor%5D=cur1"
            )),
        ),
        ("4805931", mk("{base}/users/me?page%5Bcursor%5D=cur1".to_string())),
        (
            "4805932",
            mk("{base}/artists/4805999/relationships/albums?page%5Bcursor%5D=cur1".to_string()),
        ),
        (
            "4805933",
            mk("https://127.0.0.1:1/artists/4805933/relationships/albums?page%5Bcursor%5D=cur1".to_string()),
        ),
    ])
    .await;
    let dir = tempfile::tempdir().unwrap();
    let s = tidal_state(dir.path(), &base).await;
    let app = router(s.clone());
    for id in ["4805930", "4805931", "4805932", "4805933"] {
        let (st, a) = call(
            &app,
            "GET",
            &format!("/v1/sources/tidal/artists/artist:{id}"),
            None,
        )
        .await;
        assert_eq!(st, StatusCode::OK, "{a}");
        assert_eq!(
            a["albums"].as_array().unwrap().len(),
            20,
            "only the first page: {id} {a}"
        );
        assert_eq!(a["catalogue"]["complete"], false, "{id}");
        assert!(
            a["catalogue"]["error"]
                .as_str()
                .is_some_and(|e| e.contains("next")),
            "{id}: {}",
            a["catalogue"]
        );
    }
    assert!(
        got.lock().unwrap().is_empty(),
        "the other host got requests: {:?}",
        got.lock().unwrap()
    );
    let others: Vec<String> = seen
        .lock()
        .unwrap()
        .iter()
        .filter(|x| x.starts_with("other ") || x.contains("4805999"))
        .cloned()
        .collect();
    assert!(others.is_empty(), "requests off the artist's list: {others:?}");

    // Spotify's `next` likewise
    let (sp, sp_seen) = mock_spotify("elsewhere", &bad).await;
    let s2 = spotify_state(dir.path().join("sp").as_path(), &sp).await;
    let app2 = router(s2);
    let (st, a) = call(
        &app2,
        "GET",
        "/v1/sources/spotify/artists/artist:4Z8W4fKeB5YxbusRsdQVPb",
        None,
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{a}");
    assert_eq!(a["albums"].as_array().unwrap().len(), 10);
    assert_eq!(a["catalogue"]["complete"], false);
    assert!(
        a["catalogue"]["error"]
            .as_str()
            .is_some_and(|e| e.contains("next")),
        "{}",
        a["catalogue"]
    );
    assert!(
        got.lock().unwrap().is_empty(),
        "the other host got requests: {:?}",
        got.lock().unwrap()
    );
    assert_eq!(
        sp_seen
            .lock()
            .unwrap()
            .iter()
            .filter(|x| x.starts_with("albums"))
            .count(),
        1
    );
}

/// H2: a list that takes longer than the page waits (the usual case for a big TIDAL artist) and
/// then fails is still shown with its error on the next poll, without crawling TIDAL again; an
/// explicit Retry (`?refresh=1`) does fetch it again.
#[tokio::test]
async fn a_slow_failed_catalogue_reaches_the_page_without_a_second_crawl() {
    let mut c = catalogue(75);
    c.slow_page = Some((1, crate::discover::CONTENTS_BUDGET.as_millis() as u64 + 1200));
    c.broken_page = Some(1);
    let (base, seen) = mock_tidal(vec![("4805934", c)]).await;
    let dir = tempfile::tempdir().unwrap();
    let s = tidal_state(dir.path(), &base).await;
    let app = router(s.clone());
    let rels = || {
        seen.lock()
            .unwrap()
            .iter()
            .filter(|x| x.starts_with("rel 4805934 "))
            .count()
    };
    let (_, first) = call(&app, "GET", "/v1/item/artist/tidal-4805934", None).await;
    assert_eq!(first["pending"], true, "slower than the page waits: {first}");
    let pg = settled_page(&app, "/v1/item/artist/tidal-4805934").await;
    assert_eq!(pg["albums"].as_array().unwrap().len(), 20, "{pg}");
    assert!(
        pg["catalogue"]["error"]
            .as_str()
            .is_some_and(|e| e.contains("500")),
        "{}",
        pg["catalogue"]
    );
    assert_eq!(rels(), 2, "page 0 and the failing page 1, once each");
    for _ in 0..3 {
        let (_, again) = call(&app, "GET", "/v1/item/artist/tidal-4805934", None).await;
        assert_eq!(again["catalogue"], pg["catalogue"], "polls see the error");
    }
    assert_eq!(rels(), 2, "polls don't crawl TIDAL again");
    // Retry: fetched again, now
    let (_, r) = call(&app, "GET", "/v1/item/artist/tidal-4805934?refresh=1", None).await;
    assert_eq!(r["pending"], true, "{r}");
    settled_page(&app, "/v1/item/artist/tidal-4805934").await;
    assert_eq!(rels(), 4, "Retry crawls once more");
}

/// Completeness is claimed only when the end was reached: a page that brings nothing new while
/// saying there is more, ids TIDAL lists without details, or a cursor that never ends.
#[tokio::test]
async fn tidal_paging_is_complete_only_at_the_real_end() {
    let mut stuck = catalogue(75);
    stuck.stuck_from = Some(1);
    let mut hidden = catalogue(30);
    hidden.hide_details = vec!["9000003".into(), "9000005".into()];
    let mut endless = catalogue(0);
    endless.endless = true;
    // The hidden ids are shared with the other synthetic artists: a mock of its own
    let (base, seen) = mock_tidal(vec![("4805935", stuck), ("4805937", endless)]).await;
    let (hbase, _) = mock_tidal(vec![("4805936", hidden)]).await;
    let dir = tempfile::tempdir().unwrap();
    let s = tidal_state(dir.path(), &base).await;
    let app = router(s.clone());
    let hs = tidal_state(dir.path().join("h").as_path(), &hbase).await;
    let happ = router(hs);

    let (_, a) = call(&app, "GET", "/v1/sources/tidal/artists/artist:4805935", None).await;
    assert_eq!(a["albums"].as_array().unwrap().len(), 20, "{a}");
    assert_eq!(a["catalogue"]["complete"], false, "{}", a["catalogue"]);
    assert!(
        a["catalogue"]["error"]
            .as_str()
            .is_some_and(|e| e.contains("nothing new")),
        "{}",
        a["catalogue"]
    );
    assert_eq!(
        seen.lock()
            .unwrap()
            .iter()
            .filter(|x| x.starts_with("rel 4805935 "))
            .count(),
        2
    );

    let (_, a) = call(&happ, "GET", "/v1/sources/tidal/artists/artist:4805936", None).await;
    assert_eq!(a["albums"].as_array().unwrap().len(), 28, "{a}");
    assert_eq!(a["catalogue"]["complete"], false);
    assert!(
        a["catalogue"]["error"].as_str().is_some_and(|e| e.contains("2 ")),
        "says how many are missing: {}",
        a["catalogue"]
    );

    let (_, a) = call(&app, "GET", "/v1/sources/tidal/artists/artist:4805937", None).await;
    let pages = seen
        .lock()
        .unwrap()
        .iter()
        .filter(|x| x.starts_with("rel 4805937 "))
        .count();
    assert_eq!(
        pages,
        crate::releases::max_pages(crate::releases::TIDAL_PAGE_FLOOR),
        "bounded by calls, not by rows"
    );
    assert_eq!(a["catalogue"]["truncated"], true, "{}", a["catalogue"]);
    assert_eq!(a["catalogue"]["complete"], false);
}

/// TIDAL's `Retry-After` is waited out, but never for longer than a few seconds.
#[tokio::test]
async fn tidal_rate_limit_waits_are_capped() {
    let mut c = catalogue(5);
    c.rate_limit_first = true;
    let (base, _) = mock_tidal(vec![("4805938", c)]).await;
    let dir = tempfile::tempdir().unwrap();
    let s = tidal_state(dir.path(), &base).await;
    let app = router(s.clone());
    let t = std::time::Instant::now();
    let r = tokio::time::timeout(
        std::time::Duration::from_secs(20),
        call(&app, "GET", "/v1/sources/tidal/artists/artist:4805938", None),
    )
    .await;
    let (_, a) = r.expect("a 120 s Retry-After is not waited out in full");
    assert!(t.elapsed() < std::time::Duration::from_secs(15));
    assert_eq!(a["albums"].as_array().unwrap().len(), 5, "{a}");
    assert_eq!(a["catalogue"]["complete"], true);
}

/// An artist with releases but no top songs is kept like any other (it was fetched on every look).
#[tokio::test]
async fn an_artist_with_only_releases_is_cached() {
    let (base, seen) = mock_tidal(vec![("4805939", catalogue(25))]).await;
    let dir = tempfile::tempdir().unwrap();
    let s = tidal_state(dir.path(), &base).await;
    let app = router(s.clone());
    let pg = settled_page(&app, "/v1/item/artist/tidal-4805939").await;
    assert_eq!(pg["albums"].as_array().unwrap().len(), 25);
    assert!(
        pg["songs"].as_array().is_none_or(|x| x.is_empty()),
        "the mock has no top songs"
    );
    let n = seen
        .lock()
        .unwrap()
        .iter()
        .filter(|x| x.starts_with("rel 4805939 "))
        .count();
    settled_page(&app, "/v1/item/artist/tidal-4805939").await;
    assert_eq!(
        seen.lock()
            .unwrap()
            .iter()
            .filter(|x| x.starts_with("rel 4805939 "))
            .count(),
        n
    );
}

/// A mock Spotify for one artist: 10 a page with a `total`, misbehaving as `mode` says.
async fn mock_spotify(mode: &'static str, elsewhere: &str) -> (String, Arc<std::sync::Mutex<Vec<String>>>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());
    let seen = Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
    let (b2, s2, bad) = (base.clone(), seen.clone(), elsewhere.to_string());
    let mock = Router::new()
        .route("/api/token", post(|| async { Json(serde_json::json!({"access_token": "app", "expires_in": 3600})) }))
        .route("/artists/{id}", get(|Path(id): Path<String>| async move { Json(serde_json::json!({"id": id, "name": "Synthetic Spotify"})) }))
        .route(
            "/artists/{id}/albums",
            get(move |Path(id): Path<String>, Query(q): Query<std::collections::HashMap<String, String>>| {
                let (base, seen, bad) = (b2.clone(), s2.clone(), bad.clone());
                async move {
                    seen.lock().unwrap().push(format!("albums {q:?}"));
                    let offset: usize = q.get("offset").and_then(|o| o.parse().ok()).unwrap_or(0);
                    let total = if mode == "endless" { 10_000 } else { 23 };
                    let rows = |from: usize| -> Vec<serde_json::Value> {
                        (from..(from + 10).min(total))
                            .map(|i| serde_json::json!({"id": format!("sp{i:020}"), "name": format!("Spot {i}"), "album_type": "album", "release_date": "2001", "artists": [], "images": []}))
                            .collect()
                    };
                    let next = |o: usize| format!("{base}/artists/{id}/albums?offset={o}&limit=10&include_groups=album,single");
                    let (items, next) = match (mode, offset) {
                        ("elsewhere", 0) => (rows(0), Some(format!("{bad}/artists/{id}/albums?offset=10&limit=10"))),
                        ("empty", 10) => (vec![], Some(next(20))),
                        ("short", 0) => (rows(0), None),
                        ("cycle", 10) => (rows(10), Some(next(0))),
                        (_, o) => (rows(o), (o + 10 < total).then(|| next(o + 10))),
                    };
                    Json(serde_json::json!({"items": items, "total": total, "limit": 10, "offset": offset, "next": next}))
                }
            }),
        );
    tokio::spawn(async move { axum::serve(listener, mock).await.unwrap() });
    (base, seen)
}

async fn spotify_state(dir: &std::path::Path, base: &str) -> Arc<NodeState> {
    std::fs::create_dir_all(dir).unwrap();
    let s = state(dir, None).await;
    let mut ep = s.spotify.write().unwrap();
    ep.api_base = base.to_string();
    ep.accounts_base = base.to_string();
    ep.creds = spotify::SpotifyCredentials {
        client_id: "cid".into(),
        client_secret: "sec".into(),
    };
    drop(ep);
    s
}

/// Spotify: an empty page that says there is more, a list that ends before its own `total`, a
/// `next` that leads back, and a list longer than the bound.
#[tokio::test]
async fn spotify_paging_is_complete_only_at_the_real_end() {
    let dir = tempfile::tempdir().unwrap();
    for (mode, rows, err, truncated) in [
        ("empty", 10, Some("nothing new"), false),
        ("short", 10, Some("of its 23"), false),
        ("cycle", 20, Some("nothing new"), false),
        ("endless", crate::releases::MAX_RELEASES, None, true),
    ] {
        let (base, seen) = mock_spotify(mode, "http://127.0.0.1:9").await;
        let s = spotify_state(dir.path().join(mode).as_path(), &base).await;
        let app = router(s);
        let (st, a) = call(
            &app,
            "GET",
            "/v1/sources/spotify/artists/artist:4Z8W4fKeB5YxbusRsdQVPb",
            None,
        )
        .await;
        assert_eq!(st, StatusCode::OK, "{mode}: {a}");
        assert_eq!(
            a["albums"].as_array().unwrap().len(),
            rows,
            "{mode}: {}",
            a["catalogue"]
        );
        assert_eq!(a["catalogue"]["complete"], false, "{mode}");
        match err {
            Some(e) => assert!(
                a["catalogue"]["error"].as_str().is_some_and(|x| x.contains(e)),
                "{mode}: {}",
                a["catalogue"]
            ),
            None => assert!(
                a["catalogue"].get("error").is_none(),
                "{mode}: {}",
                a["catalogue"]
            ),
        }
        assert_eq!(
            a["catalogue"]["truncated"] == true,
            truncated,
            "{mode}: {}",
            a["catalogue"]
        );
        let calls = seen.lock().unwrap().len();
        assert!(calls <= crate::releases::max_pages(10), "{mode}: {calls} calls");
    }
}

/// Deezer: a list that ends before its own `total`, and rows the node can't use (no title) with
/// a `total` that never runs out: bounded by calls and said, never "complete".
#[tokio::test]
async fn deezer_paging_is_complete_only_at_the_real_end() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let calls = Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
    let c2 = calls.clone();
    let mock = Router::new()
        .route("/artist/{id}", get(|Path(id): Path<u64>| async move { Json(serde_json::json!({"id": id, "name": "Synthetic Deezer"})) }))
        .route(
            "/artist/{id}/albums",
            get(move |Path(id): Path<u64>, Query(q): Query<std::collections::HashMap<String, String>>| {
                let calls = c2.clone();
                async move {
                    calls.lock().unwrap().push(format!("{id} {q:?}"));
                    let index: usize = q.get("index").and_then(|o| o.parse().ok()).unwrap_or(0);
                    if id == 31 {
                        // total 130, but no next after the first page
                        let data: Vec<serde_json::Value> = (0..100).map(|i| serde_json::json!({"id": 1000 + i, "title": format!("Dz {i}"), "record_type": "album"})).collect();
                        return Json(serde_json::json!({"data": data, "total": 130}));
                    }
                    let data: Vec<serde_json::Value> = (index..index + 100).map(|i| serde_json::json!({"id": 5000 + i, "record_type": "album"})).collect();
                    Json(serde_json::json!({"data": data, "total": 1_000_000, "next": format!("https://api.deezer.com/artist/{id}/albums?index={}", index + 100)}))
                }
            }),
        );
    tokio::spawn(async move { axum::serve(listener, mock).await.unwrap() });
    let dir = tempfile::tempdir().unwrap();
    let s = state(dir.path(), None).await;
    *s.deezer_base.write().unwrap() = format!("http://127.0.0.1:{port}");
    let app = router(s.clone());
    let (_, a) = call(&app, "GET", "/v1/sources/deezer/artists/artist:31", None).await;
    assert_eq!(a["albums"].as_array().unwrap().len(), 100, "{a}");
    assert_eq!(a["catalogue"]["complete"], false);
    assert!(
        a["catalogue"]["error"]
            .as_str()
            .is_some_and(|e| e.contains("of its 130")),
        "{}",
        a["catalogue"]
    );
    let (_, a) = call(&app, "GET", "/v1/sources/deezer/artists/artist:32", None).await;
    assert_eq!(a["albums"].as_array().unwrap().len(), 0, "{a}");
    assert_eq!(a["catalogue"]["complete"], false);
    assert!(
        a["catalogue"]["error"]
            .as_str()
            .is_some_and(|e| e.contains("without")),
        "{}",
        a["catalogue"]
    );
    let n = calls
        .lock()
        .unwrap()
        .iter()
        .filter(|c| c.starts_with("32 "))
        .count();
    assert!(n <= crate::releases::max_pages(100), "bounded by calls: {n}");
}

/// MusicBrainz: a first page that fails says so (it isn't "no releases"); a browse without a
/// count ends at its first short page.
#[tokio::test]
async fn musicbrainz_errors_and_missing_counts_are_honest() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let mock = Router::new().route(
        "/ws/2/release-group",
        get(|Query(q): Query<std::collections::HashMap<String, String>>| async move {
            if q.get("artist").is_some_and(|a| a.starts_with("aaaaaaaa")) {
                return (StatusCode::BAD_REQUEST, Json(serde_json::json!({"error": "bad"})));
            }
            let groups: Vec<serde_json::Value> = (0..30).map(|i| serde_json::json!({"id": format!("00000000-0000-0000-0000-{i:012}"), "title": format!("Rg {i}"), "primary-type": "Album"})).collect();
            (StatusCode::OK, Json(serde_json::json!({"release-groups": groups})))
        }),
    );
    tokio::spawn(async move { axum::serve(listener, mock).await.unwrap() });
    let dir = tempfile::tempdir().unwrap();
    let s = state(dir.path(), None).await;
    s.set_contact("test@example.org").unwrap();
    *s.mb_base.write().unwrap() = format!("http://127.0.0.1:{port}/ws/2");
    let (_, albums, cat) =
        crate::discover::musicbrainz_contents(&s, "artist", "aaaaaaaa-cf9e-42e0-be17-e2c3e1d2600d").await;
    assert!(albums.is_empty());
    let cat = cat.expect("a failure is said");
    assert!(!cat.complete && cat.error.is_some(), "{cat:?}");
    let (_, albums, cat) =
        crate::discover::musicbrainz_contents(&s, "artist", "b10bbbfc-cf9e-42e0-be17-e2c3e1d2600d").await;
    assert_eq!(albums.len(), 30);
    let cat = cat.unwrap();
    assert!(
        cat.complete && cat.total.is_none() && cat.error.is_none(),
        "{cat:?}"
    );
}

/// Qobuz (unofficial API, one page of 50 asked for): what the page holds against Qobuz's own
/// count, never more; no count → incomplete with the reason; a skipped row is said.
#[test]
fn qobuz_one_page_is_honest_about_what_it_holds() {
    let item = |i: usize| serde_json::json!({"id": format!("q{i}"), "title": format!("Q {i}"), "product_type": "album"});
    let page = |n: usize, total: Option<u64>| {
        let mut albums = serde_json::json!({"items": (0..n).map(item).collect::<Vec<_>>()});
        if let Some(t) = total {
            albums["total"] = t.into();
        }
        serde_json::json!({"albums": albums})
    };
    let (r, c) = crate::qobuz::artist_releases(&page(12, Some(12)));
    assert_eq!(
        (r.len(), c.complete, c.truncated, c.total, c.error.is_none()),
        (12, true, false, Some(12), true)
    );
    let (r, c) = crate::qobuz::artist_releases(&page(50, Some(80)));
    assert_eq!(
        (r.len(), c.complete, c.truncated, c.total),
        (50, false, true, Some(80)),
        "{c:?}"
    );
    let (_, c) = crate::qobuz::artist_releases(&page(12, None));
    assert!(!c.complete && !c.truncated && c.total.is_none(), "{c:?}");
    assert!(
        c.error.as_deref().is_some_and(|e| e.contains("how many")),
        "{c:?}"
    );
    // A row without a title: not "the service lists more", but a row the node couldn't use
    let mut p = page(12, Some(12));
    p["albums"]["items"][3]["title"] = serde_json::Value::Null;
    let (r, c) = crate::qobuz::artist_releases(&p);
    assert_eq!(r.len(), 11);
    assert!(!c.complete && !c.truncated, "{c:?}");
    assert!(c.error.as_deref().is_some_and(|e| e.contains("1 ")), "{c:?}");
}

#[test]
fn musicbrainz_secondary_types_are_kept_verbatim() {
    let k = crate::releases::musicbrainz(
        &serde_json::json!({"primary-type": "Album", "secondary-types": ["Live", "Remix", "DJ-mix"]}),
    );
    assert_eq!(k.release_type.as_deref(), Some("album"));
    assert_eq!(k.secondary_types, ["Live", "Remix", "DJ-mix"]);
    assert!(!k.compilation);
    let v = serde_json::to_value(&k).unwrap();
    assert_eq!(
        v["secondary_types"],
        serde_json::json!(["Live", "Remix", "DJ-mix"])
    );
    let plain = serde_json::to_value(crate::releases::musicbrainz(
        &serde_json::json!({"primary-type": "EP"}),
    ))
    .unwrap();
    assert!(
        plain.get("secondary_types").is_none(),
        "absent when there are none: {plain}"
    );
}
