//! Usage insights (usage.rs): off until the owner turns them on, then automatic for every
//! signed-in owner and listener browser, each person's own Stop and Clear, a closed schema, every
//! way a credential ends, limits, and a summary that never reads non-use as dislike.

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
const KEY2: &str = "8um71us3fyw6h8wbcxb5ar3rwusy1a6u49956ikzojg3gcwd1dty";

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

async fn raw(app: &axum::Router, token: &str, method: &str, uri: &str, body: String) -> (StatusCode, String) {
    let req = Request::builder()
        .method(method)
        .uri(uri)
        .header("authorization", format!("Bearer {token}"))
        .header("content-type", "application/json")
        .body(Body::from(body))
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    (status, String::from_utf8_lossy(&bytes).into_owned())
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

/// Two listeners with music access; returns a token for `KEY` (and one for `KEY2`).
fn listeners(s: &Arc<NodeState>) -> (String, String) {
    let roster = json!({"people": [
        {"key": KEY, "access": "music", "music_enabled": true, "added_at": 0},
        {"key": KEY2, "access": "music", "music_enabled": true, "added_at": 0},
    ]});
    s.library
        .lock()
        .unwrap()
        .set_setting("people", &roster.to_string())
        .unwrap();
    (
        pm_api::accounts::mint(s, KEY).unwrap(),
        pm_api::accounts::mint(s, KEY2).unwrap(),
    )
}

async fn get_json(app: &axum::Router, token: &str, uri: &str) -> Value {
    let (st, body) = call(app, token, "GET", uri, None).await;
    assert_eq!(st, StatusCode::OK, "{uri}: {body}");
    serde_json::from_str(&body).unwrap()
}

/// The owner turns usage insights on (or off), naming the generation the page showed.
async fn switch(app: &axum::Router, on: bool) {
    let st = get_json(app, OWNER, "/v1/node/usage").await;
    let (code, body) = call(
        app,
        OWNER,
        "PUT",
        "/v1/node/usage",
        Some(json!({"enabled": on, "generation": st["generation"]})),
    )
    .await;
    assert_eq!(code, StatusCode::OK, "{body}");
}

/// A page load of `token` takes part (as the client does on its own, no Start pressed).
async fn open(app: &axum::Router, token: &str) -> String {
    let me = get_json(app, token, "/v1/usage/me").await;
    let (st, body) = call(
        app,
        token,
        "POST",
        "/v1/usage/sessions",
        Some(json!({"viewport": "phone", "generation": me["generation"], "choice": me["choice"]})),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{body}");
    let v: Value = serde_json::from_str(&body).unwrap();
    v["alias"].as_str().unwrap().to_string()
}

async fn send(app: &axum::Router, token: &str, alias: &str, events: Value) -> (StatusCode, String) {
    call(
        app,
        token,
        "POST",
        "/v1/usage/events",
        Some(json!({"alias": alias, "events": events})),
    )
    .await
}

fn nav(view: &str) -> Value {
    json!({"kind": "nav", "view": view, "how": "push"})
}

fn act(control: &str) -> Value {
    json!({"kind": "action", "control": control, "view": "albums", "surface": "page", "via": "click"})
}

fn visit(exposed: &[&str], used: &[&str]) -> Value {
    json!({"kind": "visit", "view": "albums", "surface": "page", "ms": "lt3000", "exposed": exposed, "used": used})
}

async fn export(app: &axum::Router) -> Value {
    get_json(app, OWNER, "/v1/node/usage/export").await
}

/// Events in an export, per role.
fn by_role(x: &Value) -> (usize, usize) {
    let sessions = x["sessions"].as_array().cloned().unwrap_or_default();
    let count = |role: &str| {
        sessions
            .iter()
            .filter(|s| s["role"] == role)
            .map(|s| s["events"].as_u64().unwrap() as usize)
            .sum()
    };
    (count("owner"), count("listener"))
}

#[tokio::test]
async fn off_by_default_nothing_is_taken_and_no_file_is_made() {
    let dir = tempfile::tempdir().unwrap();
    let s = node(dir.path());
    let (l1, _) = listeners(&s);
    let app = router(s);
    let me = get_json(&app, &l1, "/v1/usage/me").await;
    assert_eq!(me["node_on"], json!(false), "{me}");
    assert_eq!(me["sharing"], json!(true), "sharing is the default once on: {me}");
    assert_eq!(me["role"], json!("listener"));
    let (st, _) = call(
        &app,
        &l1,
        "POST",
        "/v1/usage/sessions",
        Some(json!({"viewport": "phone", "generation": 0, "choice": 0})),
    )
    .await;
    assert_eq!(st, StatusCode::CONFLICT, "no alias while off");
    let (st, _) = send(&app, &l1, "0123456789abcdef", json!([nav("home")])).await;
    assert_eq!(st, StatusCode::CONFLICT, "no events while off");
    let status = get_json(&app, OWNER, "/v1/node/usage").await;
    assert_eq!(status["enabled"], json!(false));
    assert_eq!(
        status["coverage"]["guests"],
        json!("not instrumented"),
        "{status}"
    );
    assert!(
        !dir.path().join(pm_api::usage::FILE).exists(),
        "nothing on disk until something is stored"
    );
}

/// T1: two people take part automatically once the owner turns it on; neither reaches the
/// other's events, aliases or the owner's side; the export carries no one's identity.
#[tokio::test]
async fn everyone_signed_in_takes_part_and_nobody_reaches_anyone_elses() {
    let dir = tempfile::tempdir().unwrap();
    let s = node(dir.path());
    let (l1, l2) = listeners(&s);
    let app = router(s);
    switch(&app, true).await;
    let a_owner = open(&app, OWNER).await;
    let a_l1 = open(&app, &l1).await;
    let a_l2 = open(&app, &l2).await;
    assert_eq!(
        send(
            &app,
            OWNER,
            &a_owner,
            json!([nav("settings"), act("player_toggle")])
        )
        .await
        .0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        send(
            &app,
            &l1,
            &a_l1,
            json!([nav("albums"), act("row_play"), act("row_more")])
        )
        .await
        .0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        send(&app, &l2, &a_l2, json!([nav("search")])).await.0,
        StatusCode::NO_CONTENT
    );

    // Nobody sends into someone else's alias, or ends it
    assert_eq!(
        send(&app, &l1, &a_owner, json!([nav("home")])).await.0,
        StatusCode::GONE
    );
    assert_eq!(
        send(&app, &l2, &a_l1, json!([nav("home")])).await.0,
        StatusCode::GONE
    );
    let (st, _) = call(
        &app,
        &l2,
        "POST",
        "/v1/usage/sessions/close",
        Some(json!({"alias": a_l1})),
    )
    .await;
    assert_eq!(st, StatusCode::NO_CONTENT, "a fixed answer either way");
    assert_eq!(
        send(&app, &l1, &a_l1, json!([nav("home")])).await.0,
        StatusCode::NO_CONTENT,
        "still open"
    );

    // Listeners never reach the owner's side, reads included
    for (m, uri) in [
        ("GET", "/v1/node/usage"),
        ("PUT", "/v1/node/usage"),
        ("POST", "/v1/node/usage/clear"),
        ("GET", "/v1/node/usage/summary"),
        ("GET", "/v1/node/usage/export"),
    ] {
        let (st, _) = call(&app, &l1, m, uri, Some(json!({"enabled": false}))).await;
        assert_eq!(st, StatusCode::FORBIDDEN, "{m} {uri}");
    }

    let x = export(&app).await;
    assert_eq!(by_role(&x), (2, 5), "{x}");
    let text = x.to_string();
    for secret in [OWNER, KEY, KEY2, &l1, &l2, &a_owner, &a_l1, &a_l2] {
        assert!(!text.contains(secret), "the export carries no identity or alias");
    }

    // Clear mine takes only the caller's own
    let (st, body) = call(&app, &l1, "POST", "/v1/usage/me/clear", None).await;
    assert_eq!(st, StatusCode::OK, "{body}");
    assert_eq!(by_role(&export(&app).await), (2, 1), "only l1's four went");
}

/// T2: Stop applies at once (nothing queued from before is written); a page that showed the old
/// choice can't share again; sharing again makes a new alias.
#[tokio::test]
async fn stop_is_immediate_and_a_stale_page_cannot_share_again() {
    let dir = tempfile::tempdir().unwrap();
    let s = node(dir.path());
    let (l1, _) = listeners(&s);
    let app = router(s);
    switch(&app, true).await;
    let shown = get_json(&app, &l1, "/v1/usage/me").await;
    let alias = open(&app, &l1).await;
    assert_eq!(
        send(&app, &l1, &alias, json!([nav("albums")])).await.0,
        StatusCode::NO_CONTENT
    );
    let (st, body) = call(&app, &l1, "PUT", "/v1/usage/me", Some(json!({"share": false}))).await;
    assert_eq!(st, StatusCode::OK, "Stop needs no generation: {body}");
    let me: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(me["sharing"], json!(false));
    assert_eq!(
        by_role(&export(&app).await),
        (0, 0),
        "what waited from before Stop is not written"
    );
    assert_eq!(
        send(&app, &l1, &alias, json!([nav("home")])).await.0,
        StatusCode::GONE
    );
    let (st, _) = call(
        &app,
        &l1,
        "POST",
        "/v1/usage/sessions",
        Some(json!({"viewport": "phone", "generation": me["generation"], "choice": me["choice"]})),
    )
    .await;
    assert_eq!(st, StatusCode::CONFLICT, "no alias while stopped");
    // The page that still shows the choice from before Stop
    let (st, _) = call(
        &app,
        &l1,
        "PUT",
        "/v1/usage/me",
        Some(json!({"share": true, "choice": shown["choice"]})),
    )
    .await;
    assert_eq!(st, StatusCode::CONFLICT, "stale");
    let (st, body) = call(
        &app,
        &l1,
        "PUT",
        "/v1/usage/me",
        Some(json!({"share": true, "choice": me["choice"]})),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{body}");
    let fresh = open(&app, &l1).await;
    assert_ne!(fresh, alias);
    assert_eq!(
        send(&app, &l1, &fresh, json!([nav("home")])).await.0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(by_role(&export(&app).await), (0, 1));
}

/// T2: Off ends every alias and keeps what is stored; On again never revives an old alias; an On
/// shown before an Off is refused.
#[tokio::test]
async fn off_ends_every_alias_and_on_again_never_revives_one() {
    let dir = tempfile::tempdir().unwrap();
    let s = node(dir.path());
    let (l1, _) = listeners(&s);
    let app = router(s);
    switch(&app, true).await;
    let alias = open(&app, &l1).await;
    assert_eq!(
        send(&app, &l1, &alias, json!([nav("albums")])).await.0,
        StatusCode::NO_CONTENT
    );
    let shown = get_json(&app, OWNER, "/v1/node/usage").await;
    switch(&app, false).await;
    let (st, _) = call(
        &app,
        OWNER,
        "PUT",
        "/v1/node/usage",
        Some(json!({"enabled": true, "generation": shown["generation"]})),
    )
    .await;
    assert_eq!(st, StatusCode::CONFLICT, "an On shown before the Off");
    switch(&app, true).await;
    assert_eq!(
        send(&app, &l1, &alias, json!([nav("home")])).await.0,
        StatusCode::GONE
    );
    assert_eq!(by_role(&export(&app).await), (0, 1), "Off keeps what was stored");
    let (st, _) = call(&app, OWNER, "POST", "/v1/node/usage/clear", None).await;
    assert_eq!(st, StatusCode::NO_CONTENT);
    assert_eq!(by_role(&export(&app).await), (0, 0), "Clear all is separate");
    assert_eq!(
        get_json(&app, OWNER, "/v1/node/usage").await["enabled"],
        json!(true),
        "Clear is not Off"
    );
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

/// T3: text typed into the app, credentials and anything outside the schema never reach the log,
/// an answer, the file or the export. The control shows the capture sees framework refusals.
#[tokio::test]
async fn canaries_never_reach_the_log_an_answer_the_file_or_the_export() {
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
    let s = node(dir.path());
    let (l1, _) = listeners(&s);
    let app = router(s);
    let (st, _) = call(
        &app,
        OWNER,
        "PUT",
        "/v1/people/zones",
        Some(json!({"zones": "CONTROL-CANARY"})),
    )
    .await;
    assert!(st.is_client_error());
    assert!(text().contains("CONTROL-CANARY"), "the capture sees the log");

    switch(&app, true).await;
    let alias = open(&app, &l1).await;
    let ev = "/v1/usage/events";
    let bad = [
        format!("{SECRET} not json"),
        format!(r#"{{"alias":"{alias}","events":[{{"kind":"nav","view":"{SECRET}","how":"push"}}]}}"#),
        format!(r#"{{"alias":"{alias}","events":[{{"kind":"{SECRET}"}}]}}"#),
        format!(
            r#"{{"alias":"{alias}","events":[{{"kind":"nav","view":"home","how":"push","{SECRET}":1}}]}}"#
        ),
        format!(
            r#"{{"alias":"{alias}","events":[{{"kind":"action","control":"row_play","view":"albums","surface":"page","via":"click","text":"{SECRET}"}}]}}"#
        ),
        format!(
            r#"{{"alias":"{alias}","events":[{{"kind":"friction","signal":"error_toast","view":"home","message":"{SECRET}"}}]}}"#
        ),
        format!(
            r#"{{"alias":"{alias}","events":[{{"kind":"api","area":"library","class":"read","outcome":"ok","ms":"lt100","path":"/v1/library/search?q={SECRET}"}}]}}"#
        ),
        format!(r#"{{"alias":"{SECRET}","events":[{{"kind":"nav","view":"home","how":"push"}}]}}"#),
        format!(r#"{{"alias":"{alias}","events":[],"{SECRET}":true}}"#),
        format!(
            r#"{{"alias":"{alias}","events":[{{"kind":"nav","view":"home","how":"push"}}],"dropped":"{SECRET}"}}"#
        ),
    ];
    for b in &bad {
        let (st, answer) = raw(&app, &l1, "POST", ev, b.clone()).await;
        assert!(st.is_client_error(), "{st} for {b}");
        assert!(
            !answer.contains(SECRET),
            "the answer never quotes the input: {answer}"
        );
    }
    for (m, uri) in [
        ("PUT", "/v1/usage/me"),
        ("POST", "/v1/usage/sessions"),
        ("POST", "/v1/usage/me/notice"),
    ] {
        let (st, answer) = raw(&app, &l1, m, uri, format!(r#"{{"share":true,"x":"{SECRET}"}}"#)).await;
        assert!(st.is_client_error(), "{m} {uri}");
        assert!(!answer.contains(SECRET));
    }
    // A good batch next to them lands; the file and export hold no canary
    assert_eq!(
        send(&app, &l1, &alias, json!([nav("search"), act("search_box")]))
            .await
            .0,
        StatusCode::NO_CONTENT
    );
    let x = export(&app).await;
    assert_eq!(by_role(&x), (0, 2));
    assert!(!x.to_string().contains(SECRET));
    let file = std::fs::read(dir.path().join(pm_api::usage::FILE)).unwrap();
    assert!(
        !String::from_utf8_lossy(&file).contains(SECRET),
        "nothing outside the schema is stored"
    );
    for secret in [SECRET, OWNER, l1.as_str(), KEY] {
        assert!(
            !String::from_utf8_lossy(&file).contains(secret),
            "no credential or key in the file"
        );
        assert!(!text().contains(secret), "never in the log");
    }
}

/// A request whose body arrives only when released.
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

/// T4/T6: a batch on its way while its own sender logs out lands nowhere; one on its way while
/// someone else logs out still lands (one person's credential never stops another's).
#[tokio::test]
async fn a_body_that_stalls_through_its_own_sign_out_changes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let s = node(dir.path());
    let (l1, l2) = listeners(&s);
    let app = router(s);
    switch(&app, true).await;
    let a1 = open(&app, &l1).await;
    let a2 = open(&app, &l2).await;
    let mut mine = stalled(
        &app,
        &l1,
        "POST",
        "/v1/usage/events",
        json!({"alias": a1, "events": [nav("albums")]}),
    );
    let mut theirs = stalled(
        &app,
        &l2,
        "POST",
        "/v1/usage/events",
        json!({"alias": a2, "events": [nav("home")]}),
    );
    mine.started().await;
    theirs.started().await;
    let (st, _) = call(&app, &l1, "DELETE", "/v1/me/token", None).await;
    assert_eq!(st, StatusCode::OK);
    let (st, out) = mine.finish().await;
    assert!(
        st == StatusCode::UNAUTHORIZED || st == StatusCode::CONFLICT,
        "{st}: {out}"
    );
    let (st, out) = theirs.finish().await;
    assert_eq!(
        st,
        StatusCode::NO_CONTENT,
        "another person's batch still lands: {out}"
    );
    assert_eq!(by_role(&export(&app).await), (0, 1));
}

/// T6: a restore ends every alias and every request on its way; people's choices are not in the
/// backup and stay as they were.
#[tokio::test]
async fn a_restore_ends_aliases_and_requests_on_their_way_and_keeps_choices() {
    let dir = tempfile::tempdir().unwrap();
    let s = node(dir.path());
    let (l1, _) = listeners(&s);
    let app = router(s);
    switch(&app, true).await;
    let req = Request::builder()
        .uri("/v1/backup")
        .header("authorization", format!("Bearer {OWNER}"))
        .body(Body::empty())
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let backup = resp.into_body().collect().await.unwrap().to_bytes();
    // Not in the backup (T7)
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(backup.to_vec())).unwrap();
    let names: Vec<String> = (0..zip.len())
        .map(|i| zip.by_index(i).unwrap().name().to_string())
        .collect();
    assert!(names.iter().all(|n| !n.contains("usage")), "{names:?}");

    let (st, _) = call(&app, OWNER, "PUT", "/v1/usage/me", Some(json!({"share": false}))).await;
    assert_eq!(st, StatusCode::OK);
    let alias = open(&app, &l1).await;
    let mut late = stalled(
        &app,
        &l1,
        "POST",
        "/v1/usage/events",
        json!({"alias": alias, "events": [nav("albums")]}),
    );
    late.started().await;
    let req = Request::builder()
        .method("POST")
        .uri("/v1/restore")
        .header("authorization", format!("Bearer {OWNER}"))
        .header("content-type", "application/octet-stream")
        .body(Body::from(backup))
        .unwrap();
    assert_eq!(app.clone().oneshot(req).await.unwrap().status(), StatusCode::OK);
    let (st, out) = late.finish().await;
    // The restored library has the same tokens, so only the epoch fence can refuse it
    assert_eq!(st, StatusCode::CONFLICT, "let in before the restore: {out}");
    let me = get_json(&app, OWNER, "/v1/usage/me").await;
    assert_eq!(
        me["sharing"],
        json!(false),
        "the owner's Stop survives a restore: {me}"
    );
    assert_eq!(by_role(&export(&app).await), (0, 0));
}

/// T6: every way the owner's credentials end ends the owner's aliases (and only theirs).
#[tokio::test]
async fn ending_the_owners_sessions_ends_the_owners_aliases_only() {
    let dir = tempfile::tempdir().unwrap();
    let s = node(dir.path());
    let (l1, _) = listeners(&s);
    let app = router(s.clone());
    switch(&app, true).await;
    let a_owner = open(&app, OWNER).await;
    let a_l1 = open(&app, &l1).await;
    pm_api::accounts::end_owner_sessions(&s).unwrap();
    assert_eq!(
        send(&app, OWNER, &a_owner, json!([nav("home")])).await.0,
        StatusCode::GONE
    );
    assert_eq!(
        send(&app, &l1, &a_l1, json!([nav("home")])).await.0,
        StatusCode::NO_CONTENT
    );
}

/// T6: a listener removed (People › Remove) or with music access taken away loses their aliases;
/// removal also takes their stored choice and the events still linked to them.
#[tokio::test]
async fn a_removed_listener_loses_aliases_choice_and_linked_events() {
    let dir = tempfile::tempdir().unwrap();
    let s = node(dir.path());
    let (l1, l2) = listeners(&s);
    let app = router(s.clone());
    switch(&app, true).await;
    let a1 = open(&app, &l1).await;
    let a2 = open(&app, &l2).await;
    assert_eq!(
        send(&app, &l1, &a1, json!([nav("albums")])).await.0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        send(&app, &l2, &a2, json!([nav("albums")])).await.0,
        StatusCode::NO_CONTENT
    );
    pm_api::accounts::forget_person(&s, KEY).unwrap();
    assert_eq!(
        send(&app, &l1, &a1, json!([nav("home")])).await.0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(by_role(&export(&app).await), (0, 1), "their linked events went");
    // Music access off for the other one: their aliases end at once, so turning it back on (their
    // token works again) never revives one
    let (st, body) = call(
        &app,
        OWNER,
        "PATCH",
        &format!("/v1/people/{KEY2}"),
        Some(json!({"music": false})),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{body}");
    let (st, _) = send(&app, &l2, &a2, json!([nav("home")])).await;
    assert_eq!(st, StatusCode::UNAUTHORIZED);
    let (st, body) = call(
        &app,
        OWNER,
        "PATCH",
        &format!("/v1/people/{KEY2}"),
        Some(json!({"music": true})),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{body}");
    assert_eq!(
        send(&app, &l2, &a2, json!([nav("home")])).await.0,
        StatusCode::GONE,
        "the old alias stays ended"
    );
    let fresh = open(&app, &l2).await;
    assert_eq!(
        send(&app, &l2, &fresh, json!([nav("home")])).await.0,
        StatusCode::NO_CONTENT
    );
}

/// T6: a node without an API token (every direct request is the owner there) refuses every
/// usage route, with any token or none.
#[tokio::test]
async fn a_node_without_a_token_refuses_every_usage_route() {
    for token in [None, Some("")] {
        let dir = tempfile::tempdir().unwrap();
        let app = router(node_with(dir.path(), token));
        for presented in ["", "anything", OWNER] {
            for (m, uri) in [
                ("GET", "/v1/usage/me"),
                ("PUT", "/v1/usage/me"),
                ("POST", "/v1/usage/me/clear"),
                ("POST", "/v1/usage/me/notice"),
                ("POST", "/v1/usage/sessions"),
                ("POST", "/v1/usage/sessions/close"),
                ("POST", "/v1/usage/events"),
                ("GET", "/v1/node/usage"),
                ("PUT", "/v1/node/usage"),
                ("POST", "/v1/node/usage/clear"),
                ("GET", "/v1/node/usage/summary"),
                ("GET", "/v1/node/usage/export"),
            ] {
                let (st, _) = call(
                    &app,
                    presented,
                    m,
                    uri,
                    Some(json!({"enabled": true, "generation": 0})),
                )
                .await;
                assert!(
                    st == StatusCode::FORBIDDEN || st == StatusCode::UNAUTHORIZED,
                    "{m} {uri} with {presented:?} on a tokenless node: {st}"
                );
            }
        }
        assert!(!dir.path().join(pm_api::usage::FILE).exists());
    }
}

/// Party guests have no usage route yet (slice S6); the summary says they are not covered.
#[tokio::test]
async fn guests_are_refused_and_reported_as_not_covered() {
    use axum::http::Method;
    for (m, p) in [
        (Method::GET, "/v1/usage/me"),
        (Method::POST, "/v1/usage/events"),
        (Method::POST, "/v1/usage/sessions"),
    ] {
        assert!(pm_api::party::guest_refusal(&m, p).is_some(), "{m} {p}");
    }
    let dir = tempfile::tempdir().unwrap();
    let app = router(node(dir.path()));
    switch(&app, true).await;
    let summary = get_json(&app, OWNER, "/v1/node/usage/summary").await;
    assert_eq!(
        summary["manifest"]["coverage"]["guests"],
        json!("not instrumented"),
        "{summary}"
    );
}

/// T7: the file is only the node's (0600) and the limits hold over HTTP.
#[tokio::test]
async fn the_file_is_private_and_the_limits_hold() {
    let dir = tempfile::tempdir().unwrap();
    let s = node(dir.path());
    let (l1, _) = listeners(&s);
    let app = router(s);
    switch(&app, true).await;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(dir.path().join(pm_api::usage::FILE))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
    }
    let alias = open(&app, &l1).await;
    let many: Vec<Value> = (0..51).map(|_| nav("home")).collect();
    assert_eq!(
        send(&app, &l1, &alias, json!(many)).await.0,
        StatusCode::PAYLOAD_TOO_LARGE
    );
    let big = format!(
        r#"{{"alias":"{alias}","events":[],"pad":"{}"}}"#,
        "x".repeat(17 << 10)
    );
    assert_eq!(
        raw(&app, &l1, "POST", "/v1/usage/events", big).await.0,
        StatusCode::PAYLOAD_TOO_LARGE
    );
    assert_eq!(
        send(&app, &l1, &alias, json!([])).await.0,
        StatusCode::BAD_REQUEST
    );
    let controls = ["player_toggle", "player_toggle"];
    assert_eq!(
        send(&app, &l1, &alias, json!([visit(&controls, &[])])).await.0,
        StatusCode::BAD_REQUEST,
        "no repeats"
    );
    // 120 a minute per alias
    let fifty: Vec<Value> = (0..50).map(|_| nav("home")).collect();
    assert_eq!(
        send(&app, &l1, &alias, json!(fifty)).await.0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        send(&app, &l1, &alias, json!(fifty)).await.0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        send(&app, &l1, &alias, json!(fifty)).await.0,
        StatusCode::TOO_MANY_REQUESTS
    );
    let summary = get_json(&app, OWNER, "/v1/node/usage/summary").await;
    assert_eq!(summary["dropped"]["rate_limited"], json!(50), "{summary}");
}

/// T8: use is reported with how often a control was really shown, and only as a rate once there
/// is enough evidence; nothing is ever called unused or disliked.
#[tokio::test]
async fn the_summary_pairs_use_with_exposure_and_never_calls_anything_unused() {
    let dir = tempfile::tempdir().unwrap();
    let s = node(dir.path());
    let (l1, _) = listeners(&s);
    let app = router(s);
    switch(&app, true).await;
    let alias = open(&app, &l1).await;
    // 30 visits that showed both, one of which used Shuffle
    let mut visits: Vec<Value> = (0..29)
        .map(|_| visit(&["item_play", "item_shuffle"], &["item_play"]))
        .collect();
    visits.push(visit(&["item_play", "item_shuffle"], &["item_shuffle"]));
    assert_eq!(
        send(&app, &l1, &alias, json!(visits)).await.0,
        StatusCode::NO_CONTENT
    );
    let summary = get_json(&app, OWNER, "/v1/node/usage/summary").await;
    let shuffle = &summary["exposure"]["item_shuffle"];
    assert_eq!(shuffle["visits_shown"], json!(30), "{summary}");
    assert_eq!(shuffle["visits_used"], json!(1));
    assert_eq!(shuffle["aliases_shown"], json!(1));
    assert_eq!(
        shuffle["evidence"],
        json!("insufficient"),
        "one alias is not enough"
    );
    assert_eq!(shuffle["use_rate"], Value::Null);
    let text = summary.to_string();
    for word in ["unused", "dislike", "remove"] {
        assert!(
            !text.contains(&format!("\"{word}")),
            "no verdict words as keys: {word}"
        );
    }
    assert!(summary["manifest"]["reading"]
        .as_str()
        .unwrap()
        .contains("not a dislike"));
}

/// T6: a listener signing out on one device ends what their other device sent on its way. That
/// device's token still works, so only the person's epoch can refuse it (this pins the sign-out
/// hook in accounts.rs).
#[tokio::test]
async fn signing_out_one_device_ends_a_batch_on_its_way_from_another() {
    let dir = tempfile::tempdir().unwrap();
    let s = node(dir.path());
    let (phone, _) = listeners(&s);
    let laptop = pm_api::accounts::mint(&s, KEY).unwrap();
    let app = router(s);
    switch(&app, true).await;
    let alias = open(&app, &laptop).await;
    let mut late = stalled(
        &app,
        &laptop,
        "POST",
        "/v1/usage/events",
        json!({"alias": alias, "events": [nav("albums")]}),
    );
    late.started().await;
    let (st, _) = call(&app, &phone, "DELETE", "/v1/me/token", None).await;
    assert_eq!(st, StatusCode::OK);
    let (st, out) = late.finish().await;
    assert_eq!(st, StatusCode::CONFLICT, "the laptop's batch on its way: {out}");
    assert_eq!(by_role(&export(&app).await), (0, 0));
    // The laptop itself still works: a new alias takes part
    let fresh = open(&app, &laptop).await;
    assert_eq!(
        send(&app, &laptop, &fresh, json!([nav("home")])).await.0,
        StatusCode::NO_CONTENT
    );
}
