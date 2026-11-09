//! Automatic Qobuz app discovery from its fixed web-player origin.
//! Same method as qobuz-dl / spoofbuz; independently implemented in Rust.

use super::{api_base, app_refused, file_url_query, now, App, CHECK_TRACK};
use crate::ApiError;
use axum::http::StatusCode;
use base64::Engine;
use std::time::Duration;

const PLAYER_ORIGIN: &str = "https://play.qobuz.com";
const HTML_LIMIT: usize = 512 * 1024;
const BUNDLE_LIMIT: usize = 12 * 1024 * 1024;
const MAX_CANDIDATES: usize = 16;

#[cfg(test)]
static PLAYER: std::sync::RwLock<Option<String>> = std::sync::RwLock::new(None);

#[cfg(test)]
pub(crate) fn set_player(url: &str) {
    *PLAYER.write().expect("Qobuz player") = Some(url.trim_end_matches('/').into());
}

/// One fixed browser-like User-Agent for every request to Qobuz (the web player and the API), as
/// qobuz-dl: Qobuz never sees the node's own User-Agent or its operator's contact.
pub(super) const USER_AGENT: &str = "Mozilla/5.0 (X11; Linux x86_64; rv:83.0) Gecko/20100101 Firefox/83.0";

/// Why automatic setup did not work.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Fail {
    /// A transport error, a timeout or an unexpected answer: nothing is wrong with the app values.
    Unreachable,
    /// The web player is not what discovery expects, or Qobuz refused every candidate.
    Changed,
    /// The last look at the web player was cut off a moment ago and found nothing yet.
    Interrupted,
    /// Qobuz answered this person's sign-in with a 400 that is not about the app values: only
    /// this caller fails, nothing is refused for anyone else.
    Declined,
    /// Qobuz took this person's sign-in but answered the signed stream check with a 400 that is
    /// not about the app values: only this caller fails, nothing is refused for anyone else.
    NoStream,
}

impl Fail {
    pub(super) fn message(self) -> &'static str {
        match self {
            Self::Unreachable => "Could not reach Qobuz. Try again later.",
            Self::Changed => {
                "Qobuz: Qobuz changed its web player, so automatic setup failed. Try again later"
            }
            Self::Interrupted => "Qobuz setup was interrupted. Try again in a minute.",
            Self::Declined => "Qobuz refused this sign-in. Check your email address and try again.",
            Self::NoStream => "Qobuz would not stream for this account. Try again later.",
        }
    }
}

impl From<Fail> for ApiError {
    fn from(f: Fail) -> Self {
        let status = match f {
            Fail::Interrupted => StatusCode::SERVICE_UNAVAILABLE,
            Fail::Declined => StatusCode::BAD_REQUEST,
            Fail::Unreachable | Fail::Changed | Fail::NoStream => StatusCode::BAD_GATEWAY,
        };
        ApiError::new(status, f.message())
    }
}

pub(super) fn discovery_failed() -> ApiError {
    Fail::Changed.into()
}

pub(super) fn client() -> Result<reqwest::Client, Fail> {
    client_with_timeout(Duration::from_secs(20))
}

fn client_with_timeout(timeout: Duration) -> Result<reqwest::Client, Fail> {
    reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .user_agent(USER_AGENT)
        .connect_timeout(Duration::from_secs(5))
        .timeout(timeout)
        .build()
        .map_err(|_| Fail::Unreachable)
}

fn player_origin() -> String {
    #[cfg(test)]
    if let Some(origin) = PLAYER.read().expect("Qobuz player").clone() {
        return origin;
    }
    PLAYER_ORIGIN.into()
}

/// The longest start of `s` whose bytes all pass `ok`, and the rest (`ok` accepts ASCII only).
fn run(s: &str, ok: impl Fn(u8) -> bool) -> (&str, &str) {
    s.split_at(s.bytes().position(|b| !ok(b)).unwrap_or(s.len()))
}

/// `\w`
fn is_word(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

/// `[\w=]`
fn is_encoded(b: u8) -> bool {
    is_word(b) || b == b'='
}

/// `production:{api:{appId:"(\d{9})",appSecret:"\w{32}`: the first occurrence that matches.
fn app_id(bundle: &str) -> Option<String> {
    bundle
        .split("production:{api:{appId:\"")
        .skip(1)
        .find_map(|tail| {
            let (id, tail) = run(tail, |b| b.is_ascii_digit());
            let secret = tail.strip_prefix("\",appSecret:\"")?;
            (id.len() == 9 && run(secret, is_word).0.len() >= 32).then(|| id.to_string())
        })
}

/// `\d+` at the start of `s`, and what follows it.
fn digits(s: &str) -> Option<&str> {
    let (d, rest) = run(s, |b| b.is_ascii_digit());
    (!d.is_empty()).then_some(rest)
}

/// `<script src="(/resources/\d+\.\d+\.\d+-[a-z]\d{3}/bundle\.js)">`: a path on the player's own
/// origin, nothing else.
fn bundle_path(html: &str) -> Option<String> {
    html.split("<script src=\"").skip(1).find_map(|tail| {
        let rest = tail.strip_prefix("/resources/")?;
        let rest = digits(rest)?.strip_prefix('.')?;
        let rest = digits(rest)?.strip_prefix('.')?;
        let rest = digits(rest)?.strip_prefix('-')?;
        let rest = rest.strip_prefix(|c: char| c.is_ascii_lowercase())?;
        let (three, rest) = run(rest, |b| b.is_ascii_digit());
        let rest = rest.strip_prefix("/bundle.js\">").filter(|_| three.len() == 3)?;
        Some(tail[..tail.len() - rest.len() - "\">".len()].to_string())
    })
}

/// Every `[a-z]\.initialSeed\("([\w=]+)",window\.utimezone\.([a-z]+)\)`, timezone → seed in
/// order of first appearance (a repeated timezone keeps its place and takes the later seed).
fn seeds(bundle: &str) -> Result<Vec<(String, String)>, Fail> {
    let mut seeds: Vec<(String, String)> = Vec::new();
    for (at, _) in bundle.match_indices(".initialSeed(\"") {
        if !bundle[..at]
            .bytes()
            .next_back()
            .is_some_and(|b| b.is_ascii_lowercase())
        {
            continue;
        }
        let (seed, tail) = run(&bundle[at + ".initialSeed(\"".len()..], is_encoded);
        let Some(tail) = tail.strip_prefix("\",window.utimezone.") else {
            continue;
        };
        let (zone, tail) = run(tail, |b| b.is_ascii_lowercase());
        if seed.is_empty() || zone.is_empty() || !tail.starts_with(')') {
            continue;
        }
        if let Some((_, old)) = seeds.iter_mut().find(|(z, _)| z == zone) {
            *old = seed.to_string();
        } else {
            seeds.push((zone.to_string(), seed.to_string()));
        }
        if seeds.len() > MAX_CANDIDATES {
            return Err(Fail::Changed);
        }
    }
    Ok(seeds)
}

/// One `name:"\w+/(Zone)",info:"([\w=]+)",extras:"([\w=]+)"` at the start of `tail` (just after
/// `name:"`): the zone, info and extras.
fn info_extras(tail: &str) -> Option<(&str, &str, &str)> {
    let (region, tail) = run(tail, is_word);
    let tail = tail.strip_prefix('/').filter(|_| !region.is_empty())?;
    let (zone, tail) = run(tail, |b| b.is_ascii_alphabetic());
    let (info, tail) = run(tail.strip_prefix("\",info:\"")?, is_encoded);
    let (extras, _) = run(tail.strip_prefix("\",extras:\"")?, is_encoded);
    (!zone.is_empty() && !info.is_empty() && !extras.is_empty()).then_some((zone, info, extras))
}

fn capitalized(zone: &str) -> String {
    let mut chars = zone.chars();
    chars
        .next()
        .map(|c| c.to_ascii_uppercase().to_string() + chars.as_str())
        .unwrap_or_default()
}

fn parse_bundle(bundle: &str, bundle_url: &str) -> Result<Vec<App>, Fail> {
    let id = app_id(bundle).ok_or(Fail::Changed)?;
    let mut seeds = seeds(bundle)?;
    if seeds.len() < 2 {
        return Err(Fail::Changed);
    }
    let second = seeds.remove(1);
    seeds.insert(0, second);
    let zones: Vec<String> = seeds.iter().map(|(zone, _)| capitalized(zone)).collect();
    for (at, _) in bundle.match_indices("name:\"") {
        let Some((zone, info, extras)) = info_extras(&bundle[at + "name:\"".len()..]) else {
            continue;
        };
        if let Some(i) = zones.iter().position(|z| z == zone) {
            seeds[i].1.push_str(info);
            seeds[i].1.push_str(extras);
        }
    }
    let apps: Vec<App> = seeds
        .into_iter()
        .filter_map(|(_, joined)| {
            let encoded = joined.get(..joined.len().saturating_sub(44))?;
            let decoded = base64::engine::general_purpose::STANDARD.decode(encoded).ok()?;
            String::from_utf8(decoded).ok().filter(|s| !s.is_empty())
        })
        .map(|secret| App {
            id: id.clone(),
            secret,
            from: "web".into(),
            bundle_url: Some(bundle_url.to_string()),
        })
        .collect();
    if apps.is_empty() {
        Err(Fail::Changed)
    } else {
        Ok(apps)
    }
}

/// A body up to `limit` bytes, also when it comes without a length.
pub(super) async fn bounded(mut response: reqwest::Response, limit: usize) -> Result<Vec<u8>, Fail> {
    if response.content_length().is_some_and(|n| n > limit as u64) {
        return Err(Fail::Changed);
    }
    let mut out = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|_| Fail::Unreachable)? {
        if out.len().saturating_add(chunk.len()) > limit {
            return Err(Fail::Changed);
        }
        out.extend_from_slice(&chunk);
    }
    Ok(out)
}

/// A web-player answer that is not 2xx: busy or timed out is [`Fail::Unreachable`]; anything else
/// (gone, refused, moved: redirects are not followed) is [`Fail::Changed`].
fn player_refused(status: StatusCode) -> Fail {
    if status.is_server_error()
        || status == StatusCode::TOO_MANY_REQUESTS
        || status == StatusCode::REQUEST_TIMEOUT
    {
        Fail::Unreachable
    } else {
        Fail::Changed
    }
}

async fn text(client: &reqwest::Client, url: &str, limit: usize) -> Result<String, Fail> {
    let response = client.get(url).send().await.map_err(|_| Fail::Unreachable)?;
    if !response.status().is_success() {
        return Err(player_refused(response.status()));
    }
    String::from_utf8(bounded(response, limit).await?).map_err(|_| Fail::Changed)
}

/// The web player's app ID with every candidate secret, in qobuz-dl's order.
pub(super) async fn discover(client: &reqwest::Client) -> Result<Vec<App>, Fail> {
    let origin = player_origin();
    let html = text(client, &format!("{origin}/login"), HTML_LIMIT).await?;
    let path = bundle_path(&html).ok_or(Fail::Changed)?;
    let bundle_url = format!("{origin}{path}");
    let bundle = text(client, &bundle_url, BUNDLE_LIMIT).await?;
    parse_bundle(&bundle, &bundle_url)
}

/// An error answer's JSON, `null` when it has none (only its message is read).
pub(super) async fn json_of(response: reqwest::Response) -> serde_json::Value {
    match bounded(response, 64 * 1024).await {
        Ok(bytes) => serde_json::from_slice(&bytes).unwrap_or_default(),
        Err(_) => serde_json::Value::Null,
    }
}

/// Whether Qobuz takes `app`'s secret: a signed probe answered 2xx is yes, a 400 about the
/// signature or the app ID is no, any other 400 is [`Fail::NoStream`] (this person's alone), and
/// anything else (another status, a transport error, a timeout) is [`Fail::Unreachable`].
pub(super) async fn secret_works(client: &reqwest::Client, app: &App, token: &str) -> Result<bool, Fail> {
    let response = client
        .get(format!("{}/track/getFileUrl", api_base()))
        .header("X-App-Id", &app.id)
        .header("X-User-Auth-Token", token)
        .query(&file_url_query(app, CHECK_TRACK, 5, now()))
        .send()
        .await
        .map_err(|_| Fail::Unreachable)?;
    match response.status() {
        s if s.is_success() => Ok(true),
        StatusCode::BAD_REQUEST if app_refused(400, &json_of(response).await) => Ok(false),
        StatusCode::BAD_REQUEST => Err(Fail::NoStream),
        _ => Err(Fail::Unreachable),
    }
}

/// The first candidate whose secret works; a 400 about the app values moves on to the next,
/// anything else stops. [`Fail::Changed`] means Qobuz refused every one of them.
pub(super) async fn select_secret(
    client: &reqwest::Client,
    candidates: &[App],
    token: &str,
) -> Result<App, Fail> {
    for app in candidates {
        if secret_works(client, app, token).await? {
            return Ok(app.clone());
        }
    }
    Err(Fail::Changed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{response::IntoResponse, routing::get, Json, Router};
    use std::sync::Arc;

    fn piece(secret: &str, zone: &str) -> String {
        let encoded = base64::engine::general_purpose::STANDARD.encode(secret);
        let padded = format!("{encoded}{}", "A".repeat(44));
        let (seed, rest) = padded.split_at(8);
        let (info, extras) = rest.split_at(8);
        format!(
            "a.initialSeed(\"{seed}\",window.utimezone.{});name:\"Region/{}\",info:\"{info}\",extras:\"{extras}\";",
            zone.to_ascii_lowercase(),
            zone
        )
    }

    fn bundle(secrets: &[&str]) -> String {
        let zones = ["First", "Second", "Third", "Fourth"];
        format!(
            "production:{{api:{{appId:\"100000001\",appSecret:\"{}\"}}}};{}",
            "a".repeat(32),
            secrets
                .iter()
                .zip(zones)
                .map(|(secret, zone)| piece(secret, zone))
                .collect::<String>()
        )
    }

    #[test]
    fn discovers_good_bundle_and_moves_the_second_timezone_first() {
        let apps = parse_bundle(
            &bundle(&[
                "MockSecretOne123456789012345678901",
                "MockSecretTwo123456789012345678901",
            ]),
            "https://play.qobuz.com/resources/1.2.3-a004/bundle.js",
        )
        .unwrap();
        assert_eq!(apps.len(), 2);
        assert_eq!(apps[0].secret, "MockSecretTwo123456789012345678901");
        assert_eq!(apps[1].secret, "MockSecretOne123456789012345678901");
        assert_eq!(apps[0].id, "100000001");
        assert_eq!(
            apps[0].bundle_url.as_deref(),
            Some("https://play.qobuz.com/resources/1.2.3-a004/bundle.js")
        );
    }

    #[test]
    fn changed_bundle_changes_only_discovered_values() {
        let first = parse_bundle(
            &bundle(&[
                "MockOldOne1234567890123456789012",
                "MockOldTwo1234567890123456789012",
            ]),
            "old",
        )
        .unwrap();
        let second = parse_bundle(
            &bundle(&[
                "MockNewOne1234567890123456789012",
                "MockNewTwo1234567890123456789012",
            ]),
            "new",
        )
        .unwrap();
        assert_ne!(first[0].secret, second[0].secret);
        assert_eq!(second[0].bundle_url.as_deref(), Some("new"));
    }

    #[test]
    fn malformed_missing_app_id_and_missing_secrets_are_refused() {
        for bad in [
            String::new(),
            "production:{api:{appId:\"123\",appSecret:\"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\"}}".into(),
            piece("MockOnlyOne1234567890123456789012", "First"),
            bundle(&[
                "MockOne123456789012345678901234",
                "MockTwo123456789012345678901234",
            ])
            .replace("initialSeed", "notInitialSeed"),
            bundle(&[
                "MockOne123456789012345678901234",
                "MockTwo123456789012345678901234",
            ])
            .replace("info:", "notInfo:"),
        ] {
            assert!(parse_bundle(&bad, "mock").is_err());
        }
    }

    #[test]
    fn only_the_exact_versioned_same_origin_bundle_path_is_used() {
        assert_eq!(
            bundle_path("<script src=\"/resources/1.22.333-b004/bundle.js\"></script>").as_deref(),
            Some("/resources/1.22.333-b004/bundle.js")
        );
        for bad in [
            "<script src=\"https://evil.invalid/resources/1.2.3-a004/bundle.js\"></script>",
            "<script src=\"/resources/1.2-a004/bundle.js\"></script>",
            "<script src=\"/resources/1.2.3-a04/bundle.js\"></script>",
            "<script src=\"/resources/../1.2.3-a004/bundle.js\"></script>",
            "<script src=\"/resources/1.2.3-a004/other.js\"></script>",
        ] {
            assert!(bundle_path(bad).is_none(), "{bad}");
        }
    }

    #[tokio::test]
    async fn oversize_and_timeout_are_bounded() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let large = "x".repeat(1024);
        let server = tokio::spawn(async move {
            axum::serve(
                listener,
                Router::new()
                    .route("/large", get(move || async move { large }))
                    .route(
                        "/chunked",
                        get(|| async {
                            let chunks = (0..10).map(|_| Ok::<_, std::io::Error>("y".repeat(16)));
                            axum::body::Body::from_stream(futures_util::stream::iter(chunks))
                        }),
                    )
                    .route(
                        "/slow",
                        get(|| async {
                            tokio::time::sleep(Duration::from_secs(2)).await;
                            "late"
                        }),
                    ),
            )
            .await
            .unwrap();
        });
        assert_eq!(
            text(&client().unwrap(), &format!("{base}/large"), 64).await,
            Err(Fail::Changed)
        );
        // Without a Content-Length, the cap holds chunk by chunk
        let response = client()
            .unwrap()
            .get(format!("{base}/chunked"))
            .send()
            .await
            .unwrap();
        assert!(response.content_length().is_none());
        assert_eq!(bounded(response, 64).await, Err(Fail::Changed));
        assert_eq!(
            text(&client().unwrap(), &format!("{base}/chunked"), 4096)
                .await
                .map(|t| t.len()),
            Ok(160)
        );
        let short = client_with_timeout(Duration::from_millis(30)).unwrap();
        assert_eq!(
            text(&short, &format!("{base}/slow"), 64).await,
            Err(Fail::Unreachable)
        );
        server.abort();
    }

    #[tokio::test]
    async fn first_400_secret_is_skipped_and_second_works() {
        let _guard = super::super::API_TEST_LOCK.lock().await;
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let calls = Arc::new(std::sync::Mutex::new(Vec::new()));
        let seen = calls.clone();
        let server = tokio::spawn(async move {
            axum::serve(
                listener,
                Router::new().route(
                    "/track/getFileUrl",
                    get(
                        move |headers: axum::http::HeaderMap,
                              axum::extract::Query(q): axum::extract::Query<
                            std::collections::HashMap<String, String>,
                        >| {
                            let seen = seen.clone();
                            async move {
                                assert_eq!(headers.get("x-user-auth-token").unwrap(), "mock-token");
                                assert_eq!(q["track_id"], "5966783");
                                assert_eq!(q["format_id"], "5");
                                seen.lock().unwrap().push(q["request_sig"].clone());
                                if seen.lock().unwrap().len() == 1 {
                                    (
                                        StatusCode::BAD_REQUEST,
                                        Json(serde_json::json!({"message": "bad signature"})),
                                    )
                                } else {
                                    (
                                        StatusCode::OK,
                                        Json(serde_json::json!({"url": "https://stream.invalid/mock"})),
                                    )
                                }
                            }
                        },
                    ),
                ),
            )
            .await
            .unwrap();
        });
        super::super::set_api_base(&base);
        let candidates: Vec<App> = [
            "MockBadSecret123456789012345678901",
            "MockGoodSecret12345678901234567890",
        ]
        .into_iter()
        .map(|secret| App {
            id: "100000001".into(),
            secret: secret.into(),
            from: "web".into(),
            bundle_url: Some("mock".into()),
        })
        .collect();
        let chosen = select_secret(&client().unwrap(), &candidates, "mock-token")
            .await
            .unwrap();
        assert_eq!(chosen.secret, "MockGoodSecret12345678901234567890");
        assert_eq!(calls.lock().unwrap().len(), 2);
        server.abort();
    }

    #[tokio::test]
    async fn only_a_probe_400_about_the_app_values_moves_on() {
        let _guard = super::super::API_TEST_LOCK.lock().await;
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let seen = calls.clone();
        let server = tokio::spawn(async move {
            axum::serve(
                listener,
                Router::new().route(
                    "/track/getFileUrl",
                    get(move |headers: axum::http::HeaderMap| {
                        seen.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                        async move {
                            let message = match headers["x-app-id"].to_str().unwrap() {
                                "100000001" => "Invalid app_id parameter",
                                _ => "Something about this account",
                            };
                            (
                                StatusCode::BAD_REQUEST,
                                Json(serde_json::json!({"message": message})),
                            )
                        }
                    }),
                ),
            )
            .await
            .unwrap();
        });
        super::super::set_api_base(&base);
        let candidate = |id: &str| App {
            id: id.into(),
            secret: "MockSecret".into(),
            from: "web".into(),
            bundle_url: None,
        };
        let c = client().unwrap();
        // A 400 about the app ID moves on; one about anything else stops, for this caller only
        assert_eq!(
            select_secret(
                &c,
                &[candidate("100000001"), candidate("100000002")],
                "mock-token"
            )
            .await,
            Err(Fail::NoStream)
        );
        assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 2);
        // Every candidate refused about the app ID: the candidates are what changed
        assert_eq!(
            select_secret(&c, &[candidate("100000001")], "mock-token").await,
            Err(Fail::Changed)
        );
        assert_eq!(ApiError::from(Fail::Declined).status, StatusCode::BAD_REQUEST);
        assert_eq!(ApiError::from(Fail::NoStream).status, StatusCode::BAD_GATEWAY);
        server.abort();
    }

    #[tokio::test]
    async fn a_probe_answered_5xx_or_429_stops_selection_without_a_choice() {
        let _guard = super::super::API_TEST_LOCK.lock().await;
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let answer = Arc::new(std::sync::atomic::AtomicU16::new(503));
        let (seen, status) = (calls.clone(), answer.clone());
        let server = tokio::spawn(async move {
            axum::serve(
                listener,
                Router::new().route(
                    "/track/getFileUrl",
                    get(move || {
                        seen.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                        let code = status.load(std::sync::atomic::Ordering::SeqCst);
                        async move { StatusCode::from_u16(code).unwrap() }
                    }),
                ),
            )
            .await
            .unwrap();
        });
        super::super::set_api_base(&base);
        let candidates: Vec<App> = ["MockFirst", "MockSecond"]
            .into_iter()
            .map(|secret| App {
                id: "100000001".into(),
                secret: secret.into(),
                from: "web".into(),
                bundle_url: None,
            })
            .collect();
        for (round, code) in [503, 429].into_iter().enumerate() {
            answer.store(code, std::sync::atomic::Ordering::SeqCst);
            assert_eq!(
                select_secret(&client().unwrap(), &candidates, "mock-token").await,
                Err(Fail::Unreachable),
                "{code}"
            );
            // The first answer ends it: the second candidate is never asked
            assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), round + 1);
        }
        assert_eq!(
            Fail::Unreachable.message(),
            "Could not reach Qobuz. Try again later."
        );
        server.abort();
    }

    #[tokio::test]
    async fn a_busy_player_is_unreachable_and_a_gone_one_has_changed() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            axum::serve(
                listener,
                Router::new().route(
                    "/status/{code}",
                    get(|axum::extract::Path(code): axum::extract::Path<u16>| async move {
                        let status = StatusCode::from_u16(code).unwrap();
                        if status.is_redirection() {
                            (status, [("location", "/elsewhere")]).into_response()
                        } else {
                            status.into_response()
                        }
                    }),
                ),
            )
            .await
            .unwrap();
        });
        for (code, want) in [
            (503, Fail::Unreachable),
            (500, Fail::Unreachable),
            (429, Fail::Unreachable),
            (408, Fail::Unreachable),
            (404, Fail::Changed),
            (403, Fail::Changed),
            (302, Fail::Changed),
        ] {
            assert_eq!(
                text(&client().unwrap(), &format!("{base}/status/{code}"), 64).await,
                Err(want),
                "{code}"
            );
        }
        server.abort();
    }

    #[test]
    fn a_broken_entry_is_skipped_and_scanning_goes_on() {
        let good = bundle(&[
            "MockSecretOne123456789012345678901",
            "MockSecretTwo123456789012345678901",
        ]);
        // An entry for a known timezone without extras right after it, and one that is not a
        // timezone at all: neither matches, neither borrows text from further on
        let noisy = good.replacen(
            "a.initialSeed",
            "name:\"Region/First\",info:\"AAAA\",other:\"x\";name:\"Region/Nowhere\",info:\"AAAA\",extras:\"AAAA\";a.initialSeed",
            1,
        );
        let apps = parse_bundle(&noisy, "mock").unwrap();
        assert_eq!(apps.len(), 2);
        assert_eq!(apps[1].secret, "MockSecretOne123456789012345678901");
        // A candidate that does not decode is dropped; the others stay
        let broken = format!(
            "{good}{}",
            "b.initialSeed(\"%%%%\",window.utimezone.third);".replace("%%%%", "A")
        );
        let apps = parse_bundle(&broken, "mock").unwrap();
        assert_eq!(apps.len(), 2);
        let undecodable = good.replacen("info:\"", "info:\"A", 1);
        let apps = parse_bundle(&undecodable, "mock").unwrap();
        assert_eq!(apps.len(), 1);
        assert_eq!(apps[0].secret, "MockSecretTwo123456789012345678901");
    }

    #[test]
    fn the_app_id_and_bundle_path_follow_the_qobuz_dl_patterns() {
        let secret = "a".repeat(32);
        assert_eq!(
            app_id(&format!(
                "production:{{api:{{appId:\"12345\",appSecret:\"{secret}\"}}}};production:{{api:{{appId:\"100000002\",appSecret:\"{secret}b,"
            ))
            .as_deref(),
            Some("100000002")
        );
        assert!(app_id(&format!(
            "production:{{api:{{appId:\"100000002\",appSecret:\"{}\"",
            "a".repeat(31)
        ))
        .is_none());
        assert_eq!(
            bundle_path("<script src=\"/resources/1.2.3-a004/bundle.js\" defer>").as_deref(),
            None
        );
        assert_eq!(
            bundle_path("<script src=\"/x.js\"><script src=\"/resources/1.2.3-a004/bundle.js\">").as_deref(),
            Some("/resources/1.2.3-a004/bundle.js")
        );
    }

    #[tokio::test]
    async fn one_look_per_five_minutes_and_found_candidates_last_ten() {
        use super::super::{age_discovery, candidates, set_player};
        let _guard = super::super::API_TEST_LOCK.lock().await;
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let looks = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let down = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let (seen, off) = (looks.clone(), down.clone());
        let fixture = bundle(&[
            "MockSecretOne123456789012345678901",
            "MockSecretTwo123456789012345678901",
        ]);
        let server = tokio::spawn(async move {
            axum::serve(
                listener,
                Router::new()
                    .route(
                        "/login",
                        get(move || {
                            seen.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                            async { "<script src=\"/resources/1.2.3-a004/bundle.js\"></script>" }
                        }),
                    )
                    .route(
                        "/resources/1.2.3-a004/bundle.js",
                        get(move || {
                            let down = off.load(std::sync::atomic::Ordering::SeqCst);
                            let fixture = fixture.clone();
                            async move {
                                if down {
                                    Err(StatusCode::SERVICE_UNAVAILABLE)
                                } else {
                                    Ok(fixture)
                                }
                            }
                        }),
                    ),
            )
            .await
            .unwrap();
        });
        set_player(&base);
        let looked = || looks.load(std::sync::atomic::Ordering::SeqCst);
        let set_down = |on: bool| down.store(on, std::sync::atomic::Ordering::SeqCst);
        let c = client().unwrap();
        // A retried sign-in reuses what was found; a refused app waits for the rate limit
        assert_eq!(candidates(&c, false).await.unwrap().len(), 2);
        assert!(candidates(&c, false).await.is_ok());
        assert!(candidates(&c, true).await.is_ok());
        assert_eq!(looked(), 1);
        age_discovery(Duration::from_secs(6 * 60));
        assert!(candidates(&c, false).await.is_ok());
        assert_eq!(looked(), 1);
        assert!(candidates(&c, true).await.is_ok());
        assert_eq!(looked(), 2);
        age_discovery(Duration::from_secs(11 * 60));
        assert!(candidates(&c, false).await.is_ok());
        assert_eq!(looked(), 3);
        // A look that could not reach Qobuz keeps the candidates found before for a retried
        // sign-in, but one whose values were refused hears that Qobuz could not be reached, and
        // the next look is a minute later
        set_down(true);
        age_discovery(Duration::from_secs(6 * 60));
        assert_eq!(candidates(&c, true).await, Err(Fail::Unreachable));
        assert_eq!(looked(), 4);
        assert_eq!(candidates(&c, false).await.unwrap().len(), 2);
        assert_eq!(candidates(&c, true).await, Err(Fail::Unreachable));
        assert_eq!(looked(), 4);
        age_discovery(Duration::from_secs(61));
        assert_eq!(candidates(&c, true).await, Err(Fail::Unreachable));
        assert_eq!(looked(), 5);
        // Ten minutes after their look, the candidates are not offered to anyone any more
        age_discovery(Duration::from_secs(3 * 60));
        assert_eq!(candidates(&c, false).await, Err(Fail::Unreachable));
        assert_eq!(looked(), 6);
        assert_eq!(candidates(&c, false).await, Err(Fail::Unreachable));
        assert_eq!(looked(), 6);
        set_down(false);
        age_discovery(Duration::from_secs(61));
        let found = candidates(&c, true).await.unwrap();
        assert_eq!(looked(), 7);
        // Inside the wait, a look that found candidates offers them also to a refused caller
        assert_eq!(candidates(&c, true).await.as_ref(), Ok(&found));
        // Candidates Qobuz refused are not offered again: until the next look, that is the answer
        super::super::refuse(&found);
        assert_eq!(candidates(&c, false).await, Err(Fail::Changed));
        assert_eq!(candidates(&c, true).await, Err(Fail::Changed));
        assert_eq!(looked(), 7);
        age_discovery(Duration::from_secs(6 * 60));
        assert!(candidates(&c, false).await.is_ok());
        assert_eq!(looked(), 8);
        server.abort();
    }

    #[tokio::test]
    #[ignore = "live public unauthenticated discovery probe; opt in explicitly"]
    async fn public_player_discovery_probe() {
        let _guard = super::super::API_TEST_LOCK.lock().await;
        *PLAYER.write().unwrap() = None;
        let apps = discover(&client().unwrap()).await.unwrap();
        println!(
            "Qobuz public discovery returned {} candidate(s); values redacted",
            apps.len()
        );
    }
}
