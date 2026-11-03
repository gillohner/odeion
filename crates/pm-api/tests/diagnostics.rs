//! Opt-in local diagnostics (diagnostics.rs): the owner's node switch, each owner browser's own
//! Start, a closed event schema, limits, and an export that never carries who anyone is.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use pm_api::{router, NodeSettings, NodeState};
use pm_audio::PipelineConfig;
use pm_library::Library;
use pm_zones::OutputRegistry;
use serde_json::{json, Value};
use std::sync::Arc;
use tower::ServiceExt;

const OWNER: &str = "owner-token-CANARY-7f3a";
const KEY: &str = "o3ftkuxjd9k6zgnc4hhbhbg5jusubr4zqyb1dtzi8hhm3hf5wbmy";

async fn call(
    app: &axum::Router,
    token: &str,
    method: &str,
    uri: &str,
    body: Option<Value>,
) -> (StatusCode, String) {
    let req = Request::builder()
        .method(method)
        .uri(uri)
        .header("authorization", format!("Bearer {token}"))
        .header("content-type", "application/json")
        .body(
            body.map(|b| Body::from(b.to_string()))
                .unwrap_or_else(Body::empty),
        )
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    (status, String::from_utf8_lossy(&bytes).into_owned())
}

async fn raw(app: &axum::Router, token: &str, uri: &str, body: String) -> StatusCode {
    let req = Request::builder()
        .method("POST")
        .uri(uri)
        .header("authorization", format!("Bearer {token}"))
        .header("content-type", "application/json")
        .body(Body::from(body))
        .unwrap();
    app.clone().oneshot(req).await.unwrap().status()
}

fn node(dir: &std::path::Path) -> Arc<NodeState> {
    node_with(dir, Some(OWNER))
}

fn node_with(dir: &std::path::Path, token: Option<&str>) -> Arc<NodeState> {
    let db = dir.join("lib.sqlite");
    NodeState::new(
        NodeSettings {
            name: "t".into(),
            version: "9.9.9-test".into(),
            db_path: db.clone(),
            scan_folders: vec![],
            art_dir: dir.join("art"),
            mirror_dir: dir.join("homeserver"),
            pipeline: PipelineConfig::default(),
            token: token.map(str::to_string),
            subsonic_user: "pubky".into(),
            pubky: Default::default(),
            public_url: "http://127.0.0.1:7788".into(),
            upnp: false,
            cast: false,
            slimproto_port: 0,
            session_secret_path: None,
            tidal: Default::default(),
            spotify: Default::default(),
            acoustid_key: String::new(),
            jamendo: Default::default(),
        },
        Library::open(&db).unwrap(),
        OutputRegistry::new(),
    )
}

fn listener(s: &Arc<NodeState>) -> String {
    let roster = json!({"people": [{"key": KEY, "access": "music", "music_enabled": true, "added_at": 0}]});
    s.library
        .lock()
        .unwrap()
        .set_setting("people", &roster.to_string())
        .unwrap();
    pm_api::accounts::mint(s, KEY).unwrap()
}

fn nav(view: &str) -> Value {
    json!({"kind": "nav", "view": view})
}

#[tokio::test]
async fn off_by_default_and_nothing_is_taken_while_off() {
    let dir = tempfile::tempdir().unwrap();
    let app = router(node(dir.path()));
    let (st, body) = call(&app, OWNER, "GET", "/v1/node/diagnostics", None).await;
    assert_eq!(st, StatusCode::OK, "{body}");
    let v: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(v["enabled"], false, "{body}");
    assert_eq!(v["events"], 0, "{body}");
    // No session can start while the node switch is off
    let (st, body) = call(
        &app,
        OWNER,
        "POST",
        "/v1/node/diagnostics/sessions",
        Some(start_body("desktop", &v)),
    )
    .await;
    assert_eq!(st, StatusCode::CONFLICT, "{body}");
    // Nor can events come in
    let (st, _) = call(
        &app,
        OWNER,
        "POST",
        "/v1/node/diagnostics/events",
        Some(json!({"session": "0123456789abcdef", "events": [nav("home")]})),
    )
    .await;
    assert_eq!(st, StatusCode::CONFLICT);
}

#[tokio::test]
async fn listeners_are_refused_every_diagnostics_route_reads_too() {
    let dir = tempfile::tempdir().unwrap();
    let s = node(dir.path());
    let who = listener(&s);
    let app = router(s);
    switch(&app, OWNER, true).await;
    for (m, uri, body) in [
        ("GET", "/v1/node/diagnostics", None),
        ("GET", "/v1/node/diagnostics/export", None),
        ("PUT", "/v1/node/diagnostics", Some(json!({"enabled": false}))),
        (
            "POST",
            "/v1/node/diagnostics/sessions",
            Some(json!({"viewport": "phone"})),
        ),
        ("DELETE", "/v1/node/diagnostics/sessions/0123456789abcdef", None),
        (
            "POST",
            "/v1/node/diagnostics/events",
            Some(json!({"session": "0123456789abcdef", "events": [nav("home")]})),
        ),
        ("POST", "/v1/node/diagnostics/clear", None),
    ] {
        let (st, out) = call(&app, &who, m, uri, body).await;
        assert_eq!(st, StatusCode::FORBIDDEN, "{m} {uri}: {out}");
        // And a party guest's token is refused by the guest rule as well
        assert!(
            pm_api::party::guest_refusal(&m.parse().unwrap(), uri).is_some(),
            "{m} {uri}"
        );
    }
    // No token at all: not signed in
    let (st, _) = call(&app, "wrong", "GET", "/v1/node/diagnostics", None).await;
    assert_eq!(st, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn an_owner_session_records_closed_events_and_exports_them_without_identities() {
    let dir = tempfile::tempdir().unwrap();
    let app = router(node(dir.path()));
    switch(&app, OWNER, true).await;
    let sid = begin(&app, OWNER).await;
    assert_eq!(sid.len(), 16, "{sid}");

    let events = json!([
        nav("settings"),
        {"kind": "load", "view": "albums", "outcome": "http_5xx", "ms": "lt3000"},
        {"kind": "api", "area": "library", "outcome": "ok", "ms": "lt300"},
        {"kind": "exception", "class": "type_error"},
    ]);
    let (st, body) = call(
        &app,
        OWNER,
        "POST",
        "/v1/node/diagnostics/events",
        Some(json!({"session": sid, "events": events})),
    )
    .await;
    assert_eq!(st, StatusCode::NO_CONTENT, "{body}");

    // Anything outside the schema is refused whole: a free-text field, an unknown enum value, an extra key
    for bad in [
        json!({"session": sid, "events": [{"kind": "nav", "view": "home", "title": "CANARY-title"}]}),
        json!({"session": sid, "events": [{"kind": "nav", "view": "CANARY-view"}]}),
        json!({"session": sid, "events": [{"kind": "exception", "class": "type_error", "message": "CANARY-msg"}]}),
        json!({"session": sid, "events": [nav("home")], "url": "CANARY-url"}),
        json!({"session": "CANARY-session-id", "events": [nav("home")]}),
    ] {
        let (st, out) = call(
            &app,
            OWNER,
            "POST",
            "/v1/node/diagnostics/events",
            Some(bad.clone()),
        )
        .await;
        assert!(st.is_client_error(), "{bad}: {st} {out}");
        assert!(!out.contains("CANARY"), "the refusal does not echo input: {out}");
    }
    // Over the batch size
    let many: Vec<Value> = (0..33).map(|_| nav("home")).collect();
    let (st, _) = call(
        &app,
        OWNER,
        "POST",
        "/v1/node/diagnostics/events",
        Some(json!({"session": sid, "events": many})),
    )
    .await;
    assert_eq!(st, StatusCode::PAYLOAD_TOO_LARGE);
    // Over 16 KiB of body
    let big = format!(
        "{{\"session\":\"{sid}\",\"events\":[],\"pad\":\"{}\"}}",
        "x".repeat(17 * 1024)
    );
    assert_eq!(
        raw(&app, OWNER, "/v1/node/diagnostics/events", big).await,
        StatusCode::PAYLOAD_TOO_LARGE
    );

    let (st, body) = call(&app, OWNER, "GET", "/v1/node/diagnostics/export", None).await;
    assert_eq!(st, StatusCode::OK, "{body}");
    for secret in ["CANARY", OWNER, KEY, &sid, "owner-token"] {
        assert!(!body.contains(secret), "export leaks {secret}: {body}");
    }
    let v: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(v["manifest"]["schema"], "pm-diagnostics/1", "{body}");
    assert_eq!(v["manifest"]["app_version"], "9.9.9-test", "{body}");
    assert_eq!(v["events"].as_array().unwrap().len(), 4, "{body}");
    assert_eq!(v["events"][0]["session"], "s1", "{body}");
    assert_eq!(v["events"][1]["outcome"], "http_5xx", "{body}");
    assert_eq!(v["aggregates"]["exceptions"]["type_error"], 1, "{body}");
    assert_eq!(v["aggregates"]["loads"]["albums"]["http_5xx"], 1, "{body}");
    assert_eq!(v["sessions"][0]["viewport"], "desktop", "{body}");

    // Clear empties the ring but leaves the switch on; Off is separate
    let (st, _) = call(&app, OWNER, "POST", "/v1/node/diagnostics/clear", None).await;
    assert_eq!(st, StatusCode::NO_CONTENT);
    let (_, body) = call(&app, OWNER, "GET", "/v1/node/diagnostics", None).await;
    let v: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(
        (v["enabled"].clone(), v["events"].clone()),
        (json!(true), json!(0)),
        "{body}"
    );

    // Off ends every session at once: no replay of a queued batch after turning it on again
    switch(&app, OWNER, false).await;
    switch(&app, OWNER, true).await;
    let (st, _) = call(
        &app,
        OWNER,
        "POST",
        "/v1/node/diagnostics/events",
        Some(json!({"session": sid, "events": [nav("home")]})),
    )
    .await;
    assert_eq!(st, StatusCode::GONE);

    // Stop for this browser
    let sid2 = begin(&app, OWNER).await;
    let (st, _) = call(
        &app,
        OWNER,
        "DELETE",
        &format!("/v1/node/diagnostics/sessions/{sid2}"),
        None,
    )
    .await;
    assert_eq!(st, StatusCode::NO_CONTENT);
    let (st, _) = call(
        &app,
        OWNER,
        "POST",
        "/v1/node/diagnostics/events",
        Some(json!({"session": sid2, "events": [nav("home")]})),
    )
    .await;
    assert_eq!(st, StatusCode::GONE);
}

// ---- Authorization that holds until the change is made, and nothing without a real owner ----

async fn status(app: &axum::Router, token: &str) -> Value {
    let (st, body) = call(app, token, "GET", "/v1/node/diagnostics", None).await;
    assert_eq!(st, StatusCode::OK, "{body}");
    serde_json::from_str(&body).unwrap()
}

/// The switch as the page sends it: with the generation it showed, when the node gives one.
fn gate(on: bool, shown: &Value) -> Value {
    let mut b = json!({ "enabled": on });
    if !shown["generation"].is_null() {
        b["generation"] = shown["generation"].clone();
    }
    b
}

/// A Start as the page sends it: with the generation shown when Start was pressed.
fn start_body(viewport: &str, shown: &Value) -> Value {
    let mut b = json!({ "viewport": viewport });
    if !shown["generation"].is_null() {
        b["generation"] = shown["generation"].clone();
    }
    b
}

async fn switch(app: &axum::Router, token: &str, on: bool) -> Value {
    let shown = status(app, token).await;
    let (st, body) = call(app, token, "PUT", "/v1/node/diagnostics", Some(gate(on, &shown))).await;
    assert_eq!(st, StatusCode::OK, "{body}");
    serde_json::from_str(&body).unwrap()
}

async fn begin(app: &axum::Router, token: &str) -> String {
    let shown = status(app, token).await;
    let (st, body) = call(
        app,
        token,
        "POST",
        "/v1/node/diagnostics/sessions",
        Some(start_body("desktop", &shown)),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{body}");
    serde_json::from_str::<Value>(&body).unwrap()["session"]
        .as_str()
        .unwrap()
        .to_string()
}

/// Make a new token (Settings): the old one stops working at once.
async fn rotate(app: &axum::Router, token: &str) -> String {
    let (st, body) = call(app, token, "POST", "/v1/node/token", None).await;
    assert_eq!(st, StatusCode::OK, "{body}");
    serde_json::from_str::<Value>(&body).unwrap()["token"]
        .as_str()
        .unwrap()
        .to_string()
}

/// A request whose body arrives only when released. `started` returns once the node has let it
/// in and begun reading the body.
struct Stalled {
    polled: tokio::sync::oneshot::Receiver<()>,
    release: tokio::sync::oneshot::Sender<()>,
    answer: tokio::task::JoinHandle<(StatusCode, String)>,
}

fn stalled(app: &axum::Router, token: &str, method: &str, uri: &str, body: Value) -> Stalled {
    let (polled_tx, polled) = tokio::sync::oneshot::channel();
    let (release, release_rx) = tokio::sync::oneshot::channel::<()>();
    let stream = futures_util::stream::unfold(
        (Some(polled_tx), Some(release_rx), Some(body.to_string())),
        |(polled, release, body)| async move {
            if let Some(p) = polled {
                let _ = p.send(());
            }
            let _ = release?.await;
            Some((
                Ok::<_, std::io::Error>(axum::body::Bytes::from(body?)),
                (None, None, None),
            ))
        },
    );
    let req = Request::builder()
        .method(method)
        .uri(uri)
        .header("authorization", format!("Bearer {token}"))
        .header("content-type", "application/json")
        .body(Body::from_stream(stream))
        .unwrap();
    let app = app.clone();
    let answer = tokio::spawn(async move {
        let resp = app.oneshot(req).await.unwrap();
        let st = resp.status();
        let b = resp.into_body().collect().await.unwrap().to_bytes();
        (st, String::from_utf8_lossy(&b).into_owned())
    });
    Stalled {
        polled,
        release,
        answer,
    }
}

impl Stalled {
    async fn started(&mut self) {
        (&mut self.polled).await.expect("the node reads the body");
    }
    async fn finish(self) -> (StatusCode, String) {
        let _ = self.release.send(());
        self.answer.await.unwrap()
    }
}

/// A node without an API token (or an empty one) makes every direct request the owner. That is
/// how such a node always worked and still does, but diagnostics are never open on it.
#[tokio::test]
async fn a_node_without_a_token_refuses_every_diagnostics_route() {
    for configured in [None, Some("")] {
        let dir = tempfile::tempdir().unwrap();
        let app = router(node_with(dir.path(), configured));
        // The rest of the node: as before, the owner
        let (st, me) = call(&app, "", "GET", "/v1/me", None).await;
        assert_eq!(st, StatusCode::OK, "{configured:?}: {me}");
        assert!(me.contains("owner"), "{me}");
        let sid = "0123456789abcdef";
        for presented in ["", "anything", OWNER] {
            for (m, uri, body) in [
                ("GET", "/v1/node/diagnostics".to_string(), None),
                (
                    "PUT",
                    "/v1/node/diagnostics".into(),
                    Some(json!({"enabled": true, "generation": 0})),
                ),
                (
                    "PUT",
                    "/v1/node/diagnostics".into(),
                    Some(json!({"enabled": true})),
                ),
                (
                    "POST",
                    "/v1/node/diagnostics/sessions".into(),
                    Some(json!({"viewport": "desktop", "generation": 1})),
                ),
                (
                    "POST",
                    "/v1/node/diagnostics/sessions".into(),
                    Some(json!({"viewport": "desktop"})),
                ),
                ("DELETE", format!("/v1/node/diagnostics/sessions/{sid}"), None),
                (
                    "POST",
                    "/v1/node/diagnostics/events".into(),
                    Some(json!({"session": sid, "events": [nav("home")]})),
                ),
                ("POST", "/v1/node/diagnostics/clear".into(), None),
                ("GET", "/v1/node/diagnostics/export".into(), None),
            ] {
                let (st, out) = call(&app, presented, m, &uri, body).await;
                // Where the node lets the request in as the owner (any token on a tokenless node,
                // the empty one on an empty-token node), diagnostics refuse it themselves; a token
                // the node doesn't know is refused before, as everywhere
                let let_in_as_owner = configured.is_none() || presented.is_empty();
                let expected = if let_in_as_owner {
                    StatusCode::FORBIDDEN
                } else {
                    StatusCode::UNAUTHORIZED
                };
                assert_eq!(st, expected, "{configured:?} {presented:?} {m} {uri}: {out}");
                assert!(!out.contains(OWNER), "{out}");
            }
        }
    }
}

/// Codex review high 2: a request let in with a token, whose body is still on its way when the
/// owner makes a new token, changes nothing once the body arrives.
#[tokio::test]
async fn a_body_that_stalls_through_a_new_token_changes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let s = node(dir.path());
    s.set_token_file(dir.path().join("api-token"));
    let app = router(s.clone());

    // Events
    switch(&app, OWNER, true).await;
    let sid = begin(&app, OWNER).await;
    let mut late = stalled(
        &app,
        OWNER,
        "POST",
        "/v1/node/diagnostics/events",
        json!({"session": sid, "events": [nav("home")]}),
    );
    late.started().await;
    let t1 = rotate(&app, OWNER).await;
    let (st, out) = late.finish().await;
    assert!(st.is_client_error(), "events after the token changed: {st} {out}");
    assert!(!out.contains(OWNER) && !out.contains(&t1), "{out}");
    let after = status(&app, &t1).await;
    assert_eq!(after["events"], 0, "nothing was kept");
    assert_eq!(after["sessions"], 0, "the old token's session ended with it");

    // The switch: an On sent before the token changed stays Off
    switch(&app, &t1, false).await;
    let shown = status(&app, &t1).await;
    let mut late = stalled(&app, &t1, "PUT", "/v1/node/diagnostics", gate(true, &shown));
    late.started().await;
    let t2 = rotate(&app, &t1).await;
    let (st, out) = late.finish().await;
    assert!(
        st.is_client_error(),
        "the switch after the token changed: {st} {out}"
    );
    assert_eq!(status(&app, &t2).await["enabled"], false);

    // A Start sent before the token changed makes no session
    switch(&app, &t2, true).await;
    let shown = status(&app, &t2).await;
    let mut late = stalled(
        &app,
        &t2,
        "POST",
        "/v1/node/diagnostics/sessions",
        start_body("phone", &shown),
    );
    late.started().await;
    let t3 = rotate(&app, &t2).await;
    let (st, out) = late.finish().await;
    assert!(
        st.is_client_error(),
        "a Start after the token changed: {st} {out}"
    );
    assert_eq!(status(&app, &t3).await["sessions"], 0);
}

/// Codex review high 3: a Start (or an On) shown before someone turned diagnostics off and on
/// again elsewhere is refused when it arrives; it never joins the new generation.
#[tokio::test]
async fn a_start_or_an_on_that_waited_through_off_and_on_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let app = router(node(dir.path()));

    switch(&app, OWNER, true).await;
    let shown = status(&app, OWNER).await;
    let mut late = stalled(
        &app,
        OWNER,
        "POST",
        "/v1/node/diagnostics/sessions",
        start_body("desktop", &shown),
    );
    late.started().await;
    switch(&app, OWNER, false).await;
    switch(&app, OWNER, true).await;
    let (st, out) = late.finish().await;
    assert_eq!(st, StatusCode::CONFLICT, "a stale Start: {out}");
    assert_eq!(status(&app, OWNER).await["sessions"], 0);

    // An On shown while Off, arriving after someone else turned it on and off again
    switch(&app, OWNER, false).await;
    let shown = status(&app, OWNER).await;
    let mut late = stalled(&app, OWNER, "PUT", "/v1/node/diagnostics", gate(true, &shown));
    late.started().await;
    switch(&app, OWNER, true).await;
    switch(&app, OWNER, false).await;
    let (st, out) = late.finish().await;
    assert_eq!(st, StatusCode::CONFLICT, "a stale On: {out}");
    assert_eq!(status(&app, OWNER).await["enabled"], false);

    // Events from a session of the old generation, arriving after Off and On: refused
    switch(&app, OWNER, true).await;
    let sid = begin(&app, OWNER).await;
    let mut late = stalled(
        &app,
        OWNER,
        "POST",
        "/v1/node/diagnostics/events",
        json!({"session": sid, "events": [nav("home")]}),
    );
    late.started().await;
    switch(&app, OWNER, false).await;
    switch(&app, OWNER, true).await;
    let (st, out) = late.finish().await;
    assert!(st.is_client_error(), "{st} {out}");
    assert_eq!(status(&app, OWNER).await["events"], 0);

    // Off always applies, whatever generation it was shown: turning off is never stale
    let shown = status(&app, OWNER).await;
    switch(&app, OWNER, false).await;
    switch(&app, OWNER, true).await;
    let (st, out) = call(
        &app,
        OWNER,
        "PUT",
        "/v1/node/diagnostics",
        Some(gate(false, &shown)),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{out}");
    assert_eq!(status(&app, OWNER).await["enabled"], false);

    // The generation is part of the request: a Start without one is not in the schema, an On
    // without one can't say what it was shown
    let (st, _) = call(
        &app,
        OWNER,
        "POST",
        "/v1/node/diagnostics/sessions",
        Some(json!({"viewport": "phone"})),
    )
    .await;
    assert_eq!(st, StatusCode::BAD_REQUEST);
    let (st, _) = call(
        &app,
        OWNER,
        "PUT",
        "/v1/node/diagnostics",
        Some(json!({"enabled": true})),
    )
    .await;
    assert_eq!(st, StatusCode::CONFLICT);
    assert_eq!(status(&app, OWNER).await["enabled"], false);
}

#[derive(Clone, Default)]
struct Logs(Arc<std::sync::Mutex<Vec<u8>>>);

impl std::io::Write for Logs {
    fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(b);
        Ok(b.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// What a malformed request carries never reaches the node's log (or the answer). The node's
/// framework refusals are logged with their text (lib.rs `plain_errors`), which a control shows
/// this capture sees; the diagnostics routes never go that way.
#[tokio::test]
async fn malformed_bodies_never_reach_the_log() {
    const SECRET: &str = "SECRET-CANARY-5e1d";
    let logs = Logs::default();
    let w = logs.clone();
    let sub = tracing_subscriber::fmt()
        .with_ansi(false)
        .with_max_level(tracing::Level::TRACE)
        .with_writer(move || w.clone())
        .finish();
    let _guard = tracing::subscriber::set_default(sub);
    let text = || String::from_utf8_lossy(&logs.0.lock().unwrap()).into_owned();

    let dir = tempfile::tempdir().unwrap();
    let app = router(node(dir.path()));
    // The control: a framework refusal elsewhere is logged with the request's text
    let (st, _) = call(
        &app,
        OWNER,
        "PUT",
        "/v1/people/zones",
        Some(json!({"zones": "CONTROL-CANARY"})),
    )
    .await;
    assert!(st.is_client_error());
    assert!(
        text().contains("CONTROL-CANARY"),
        "the capture sees the log: {}",
        text()
    );

    switch(&app, OWNER, true).await;
    let sid = begin(&app, OWNER).await;
    let ev = "/v1/node/diagnostics/events";
    let bad: Vec<(&str, &str, String)> = vec![
        ("POST", ev, format!("{SECRET} not json")),
        (
            "POST",
            ev,
            format!(r#"{{"session":"{sid}","events":[{{"kind":"nav","view":"{SECRET}"}}]}}"#),
        ),
        (
            "POST",
            ev,
            format!(r#"{{"session":"{sid}","events":[{{"kind":"{SECRET}"}}]}}"#),
        ),
        (
            "POST",
            ev,
            format!(r#"{{"session":"{sid}","events":[{{"kind":"nav","view":"home","{SECRET}":1}}]}}"#),
        ),
        (
            "POST",
            ev,
            format!(r#"{{"session":"{sid}","events":[],"{SECRET}":true}}"#),
        ),
        ("POST", ev, format!(r#"{{"session":"{SECRET}","events":[]}}"#)),
        (
            "POST",
            ev,
            format!(r#"{{"session":"{sid}","events":[],"dropped":"{SECRET}"}}"#),
        ),
        (
            "POST",
            ev,
            format!(
                r#"{{"session":"{sid}","events":[],"pad":"{SECRET}{}"}}"#,
                "x".repeat(20 * 1024)
            ),
        ),
        (
            "PUT",
            "/v1/node/diagnostics",
            format!(r#"{{"enabled":"{SECRET}"}}"#),
        ),
        (
            "PUT",
            "/v1/node/diagnostics",
            format!(r#"{{"enabled":true,"generation":"{SECRET}"}}"#),
        ),
        (
            "POST",
            "/v1/node/diagnostics/sessions",
            format!(r#"{{"viewport":"{SECRET}"}}"#),
        ),
        (
            "POST",
            "/v1/node/diagnostics/sessions",
            format!(r#"{{"viewport":"phone","{SECRET}":1}}"#),
        ),
    ];
    for (m, uri, body) in bad {
        for ctype in ["application/json", "text/plain"] {
            let req = Request::builder()
                .method(m)
                .uri(uri)
                .header("authorization", format!("Bearer {OWNER}"))
                .header("content-type", ctype)
                .body(Body::from(body.clone()))
                .unwrap();
            let resp = app.clone().oneshot(req).await.unwrap();
            let st = resp.status();
            let out = resp.into_body().collect().await.unwrap().to_bytes();
            let out = String::from_utf8_lossy(&out);
            assert!(st.is_client_error(), "{m} {uri} {ctype}: {st} {out}");
            assert!(!out.contains(SECRET), "the answer echoes it: {out}");
        }
    }
    // In the path, the query and a wrong token
    for (token, m, uri) in [
        (
            OWNER.to_string(),
            "DELETE",
            format!("/v1/node/diagnostics/sessions/{SECRET}"),
        ),
        (
            OWNER.to_string(),
            "GET",
            format!("/v1/node/diagnostics/export?{SECRET}=1"),
        ),
        (
            format!("{SECRET}-token"),
            "GET",
            "/v1/node/diagnostics".to_string(),
        ),
    ] {
        let (_, out) = call(&app, &token, m, &uri, None).await;
        assert!(!out.contains(SECRET), "{out}");
    }
    let logged = text();
    assert!(!logged.contains(SECRET), "a canary reached the log:\n{logged}");
    assert!(!logged.contains(OWNER), "the token reached the log:\n{logged}");
}

/// Codex final review 2: an explicit Off while already off still cancels an On shown before it
#[tokio::test]
async fn an_explicit_off_while_off_cancels_a_pending_on() {
    let dir = tempfile::tempdir().unwrap();
    let app = router(node(dir.path()));
    let shown = status(&app, OWNER).await;
    assert_eq!(shown["enabled"], false);
    let mut late = stalled(&app, OWNER, "PUT", "/v1/node/diagnostics", gate(true, &shown));
    late.started().await;
    // Off, as it already is: the On on its way was shown before this Off
    switch(&app, OWNER, false).await;
    let (st, out) = late.finish().await;
    assert_eq!(st, StatusCode::CONFLICT, "an On from before an Off: {out}");
    assert_eq!(status(&app, OWNER).await["enabled"], false);
}

/// A settings restore replaces the owner's browser sessions (the library they are kept in): a
/// way owner credentials end, so diagnostics sessions end and a request on its way changes
/// nothing, though the API token it carries still works.
#[tokio::test]
async fn a_settings_restore_ends_sessions_and_requests_on_their_way() {
    let dir = tempfile::tempdir().unwrap();
    let app = router(node(dir.path()));
    let req = Request::builder()
        .uri("/v1/backup")
        .header("authorization", format!("Bearer {OWNER}"))
        .body(Body::empty())
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let backup = resp.into_body().collect().await.unwrap().to_bytes();
    switch(&app, OWNER, true).await;
    let sid = begin(&app, OWNER).await;
    let mut late = stalled(
        &app,
        OWNER,
        "POST",
        "/v1/node/diagnostics/events",
        json!({"session": sid, "events": [nav("home")]}),
    );
    late.started().await;
    let req = Request::builder()
        .method("POST")
        .uri("/v1/restore")
        .header("authorization", format!("Bearer {OWNER}"))
        .header("content-type", "application/octet-stream")
        .body(Body::from(backup))
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let (st, out) = late.finish().await;
    assert_eq!(
        st,
        StatusCode::CONFLICT,
        "a request let in before the restore: {out}"
    );
    let after = status(&app, OWNER).await;
    assert_eq!(
        (after["events"].clone(), after["sessions"].clone()),
        (json!(0), json!(0)),
        "{after}"
    );
}

/// A batch without events (a drop count alone) is refused and counts nothing
#[tokio::test]
async fn an_empty_batch_is_refused_over_http() {
    let dir = tempfile::tempdir().unwrap();
    let app = router(node(dir.path()));
    switch(&app, OWNER, true).await;
    let sid = begin(&app, OWNER).await;
    for body in [
        json!({"session": sid, "events": []}),
        json!({"session": sid, "events": [], "dropped": 65535}),
    ] {
        let (st, out) = call(&app, OWNER, "POST", "/v1/node/diagnostics/events", Some(body)).await;
        assert_eq!(st, StatusCode::BAD_REQUEST, "{out}");
    }
    assert_eq!(status(&app, OWNER).await["dropped"]["by_browsers"], 0);
}

/// The manual download: Odeion's file name, the schema identifier kept as it is
#[tokio::test]
async fn the_export_is_an_odeion_download_of_schema_pm_diagnostics_1() {
    let dir = tempfile::tempdir().unwrap();
    let app = router(node(dir.path()));
    let req = Request::builder()
        .uri("/v1/node/diagnostics/export")
        .header("authorization", format!("Bearer {OWNER}"))
        .body(Body::empty())
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let disposition = resp
        .headers()
        .get("content-disposition")
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_string();
    assert_eq!(disposition, "attachment; filename=\"odeion-diagnostics.json\"");
    let body = resp.into_body().collect().await.unwrap().to_bytes();
    let v: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(v["manifest"]["schema"], "pm-diagnostics/1");
}
