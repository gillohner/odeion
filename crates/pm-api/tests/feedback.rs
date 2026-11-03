//! Problem reports from the node's people (TASK-trusted-user-feedback): the owner and listeners
//! send a description, and the details they tick, to a private inbox on the node. Party guests and
//! a node without an API token can't. Each person sees only their own reports. Nothing is sent
//! anywhere: the node's operator reads the inbox with `pm-node feedback` and records a receipt.
use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use pm_api::feedback::{Inbox, State};
use pm_api::{router, NodeSettings, NodeState};
use pm_audio::PipelineConfig;
use pm_library::Library;
use pm_zones::OutputRegistry;
use serde_json::{json, Value};
use std::sync::Arc;
use tower::ServiceExt;

const OWNER: &str = "owner-token-canary-7f3a9c";
const ANN: &str = "o3ftkuxjd9k6zgnc4hhbhbg5jusubr4zqyb1dtzi8hhm3hf5wbmy";
const BEN: &str = "8pinxxgqs41n4aididenw5apqp1urfmzdztr8jt4abrkdn435ewo";

async fn call(
    app: &axum::Router,
    token: Option<&str>,
    method: &str,
    uri: &str,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let mut req = Request::builder()
        .method(method)
        .uri(uri)
        .header("content-type", "application/json");
    if let Some(t) = token {
        req = req.header("authorization", format!("Bearer {t}"));
    }
    let req = req
        .body(
            body.map(|b| Body::from(b.to_string()))
                .unwrap_or_else(Body::empty),
        )
        .unwrap();
    send(app, req).await
}

async fn send(app: &axum::Router, req: Request<Body>) -> (StatusCode, Value) {
    let resp = app.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    (status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
}

fn node(dir: &std::path::Path, token: Option<&str>) -> Arc<NodeState> {
    let db = dir.join("lib.sqlite");
    let s = NodeState::new(
        NodeSettings {
            name: "t".into(),
            version: "0.2.3".into(),
            db_path: db.clone(),
            scan_folders: vec![],
            art_dir: dir.join("art"),
            mirror_dir: dir.join("homeserver"),
            pipeline: PipelineConfig::default(),
            token: token.map(Into::into),
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
    );
    let roster = json!({"people": [
        {"key": ANN, "access": "music", "music_enabled": true, "added_at": 0},
        {"key": BEN, "access": "music", "music_enabled": true, "added_at": 0}]});
    s.library
        .lock()
        .unwrap()
        .set_setting("people", &roster.to_string())
        .unwrap();
    s
}

fn report(text: &str) -> Value {
    json!({"kind": "problem", "text": text, "client_id": uuid::Uuid::new_v4().to_string()})
}

fn inbox(dir: &std::path::Path) -> Inbox {
    Inbox::open(dir).unwrap()
}

/// Everything stored for a report, as the operator's export shows it.
fn stored(dir: &std::path::Path) -> String {
    serde_json::to_string(&inbox(dir).pending(pm_api::feedback::now()).unwrap()).unwrap()
}

#[tokio::test]
async fn the_owner_and_listeners_send_reports_to_the_nodes_private_inbox() {
    let dir = tempfile::tempdir().unwrap();
    let s = node(dir.path(), Some(OWNER));
    let ann = pm_api::accounts::mint(&s, ANN).unwrap();
    let app = router(s.clone());

    let (st, r) = call(
        &app,
        Some(OWNER),
        "POST",
        "/v1/feedback",
        Some(report("The queue skips a song")),
    )
    .await;
    assert_eq!(st, StatusCode::CREATED, "{r}");
    assert_eq!(r["state"], "saved");
    let id = r["id"].as_str().unwrap().to_string();
    assert!(id.starts_with("fb_") && id.len() == 19, "{id}");

    let mut body = report("Search finds nothing for umlauts");
    body["kind"] = json!("idea");
    body["details"] = json!({"version": "0.2.3", "view": "search", "shell": "phone",
        "browser": "safari", "browser_major": 17});
    let (st, r) = call(&app, Some(&ann), "POST", "/v1/feedback", Some(body)).await;
    assert_eq!(st, StatusCode::CREATED, "{r}");

    let pending = inbox(dir.path()).pending(pm_api::feedback::now()).unwrap();
    assert_eq!(pending.len(), 2);
    let from_ann = pending.iter().find(|p| p.from == "listener").unwrap();
    assert_eq!(from_ann.text, "Search finds nothing for umlauts");
    assert_eq!(from_ann.details.as_ref().unwrap().view.as_deref(), Some("search"));
    // A pseudonym, never the person's key
    assert_eq!(from_ann.person.len(), 8);
    assert!(!stored(dir.path()).contains(ANN));
    assert_eq!(pending.iter().find(|p| p.from == "owner").unwrap().id, id);

    // The inbox is its own file, outside the library database (so never in a backup)
    assert!(dir.path().join(pm_api::feedback::FILE).exists());
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(dir.path().join(pm_api::feedback::FILE))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o077, 0, "only the node's user reads the inbox: {mode:o}");
    }
    for f in ["lib.sqlite", "lib.sqlite-wal"] {
        if let Ok(bytes) = std::fs::read(dir.path().join(f)) {
            assert!(
                !String::from_utf8_lossy(&bytes).contains("umlauts"),
                "{f} holds a report"
            );
        }
    }
}

#[tokio::test]
async fn without_an_api_token_or_as_a_party_guest_nobody_sends() {
    // Without a token every caller is the owner, so the node can't tell reporters apart
    let dir = tempfile::tempdir().unwrap();
    let s = node(dir.path(), None);
    let app = router(s);
    let (st, r) = call(&app, None, "POST", "/v1/feedback", Some(report("hello"))).await;
    assert_eq!(st, StatusCode::FORBIDDEN, "{r}");
    let (st, _) = call(&app, None, "GET", "/v1/feedback/mine", None).await;
    assert_eq!(st, StatusCode::FORBIDDEN);
    assert!(Inbox::open(dir.path())
        .unwrap()
        .pending(pm_api::feedback::now())
        .unwrap()
        .is_empty());

    // No token, a wrong one
    let dir = tempfile::tempdir().unwrap();
    let app = router(node(dir.path(), Some(OWNER)));
    let (st, _) = call(&app, None, "POST", "/v1/feedback", Some(report("hello"))).await;
    assert_eq!(st, StatusCode::UNAUTHORIZED);
    let (st, _) = call(
        &app,
        Some("pml_nope"),
        "POST",
        "/v1/feedback",
        Some(report("hello")),
    )
    .await;
    assert_eq!(st, StatusCode::UNAUTHORIZED);
    // A party guest only adds songs to the party
    assert!(pm_api::party::guest_refusal(&axum::http::Method::POST, "/v1/feedback").is_some());
    assert!(pm_api::party::guest_refusal(&axum::http::Method::GET, "/v1/feedback/mine").is_some());
}

#[tokio::test]
async fn a_report_has_one_fixed_shape_and_a_refusal_never_repeats_it() {
    let dir = tempfile::tempdir().unwrap();
    let app = router(node(dir.path(), Some(OWNER)));
    let canary = "CANARY-secret-in-a-refused-body";
    let id = uuid::Uuid::new_v4().to_string();
    let too_long = "x".repeat(pm_api::feedback::MAX_TEXT + 1);
    let bad = [
        json!({"kind": "problem", "text": canary, "client_id": id, "url": "https://x"}),
        json!({"kind": canary, "text": "a", "client_id": id}),
        json!({"kind": "problem", "text": "   \n ", "client_id": id}),
        json!({"kind": "problem", "text": too_long, "client_id": id}),
        json!({"kind": "problem", "text": canary, "client_id": canary}),
        json!({"kind": "problem", "text": "a", "client_id": id, "details": {"view": canary}}),
        json!({"kind": "problem", "text": "a", "client_id": id, "details": {"cookie": canary}}),
        json!({"kind": "problem", "text": "a", "client_id": id, "details": {"version": format!("1 {canary}")}}),
        json!({"kind": "problem", "text": "a", "client_id": id, "details": {"shell": "watch"}}),
        json!([canary]),
    ];
    for b in bad {
        let (st, r) = call(&app, Some(OWNER), "POST", "/v1/feedback", Some(b.clone())).await;
        assert_eq!(st, StatusCode::BAD_REQUEST, "{b}");
        assert!(
            !r.to_string().contains(canary),
            "the refusal repeats the body: {r}"
        );
        assert!(!r.to_string().contains("deserialize"), "a raw error: {r}");
    }
    // Not JSON at all, and a body far over the limit
    let req = Request::post("/v1/feedback")
        .header("authorization", format!("Bearer {OWNER}"))
        .header("content-type", "application/json")
        .body(Body::from(format!("{{\"text\": \"{canary}")))
        .unwrap();
    let (st, r) = send(&app, req).await;
    assert_eq!(st, StatusCode::BAD_REQUEST);
    assert!(!r.to_string().contains(canary));
    let huge = json!({"kind": "problem", "text": "y".repeat(100_000), "client_id": id});
    let (st, _) = call(&app, Some(OWNER), "POST", "/v1/feedback", Some(huge)).await;
    assert_eq!(st, StatusCode::PAYLOAD_TOO_LARGE);
    assert!(stored(dir.path()) == "[]", "nothing refused is kept");

    // Exactly the limit is fine; control characters and direction overrides go, new lines stay
    let ok = "z".repeat(pm_api::feedback::MAX_TEXT);
    let (st, _) = call(&app, Some(OWNER), "POST", "/v1/feedback", Some(report(&ok))).await;
    assert_eq!(st, StatusCode::CREATED);
    let (st, _) = call(
        &app,
        Some(OWNER),
        "POST",
        "/v1/feedback",
        Some(report("  one\r\ntwo\u{7}\u{202e}three  ")),
    )
    .await;
    assert_eq!(st, StatusCode::CREATED);
    let texts: Vec<String> = inbox(dir.path())
        .pending(pm_api::feedback::now())
        .unwrap()
        .into_iter()
        .map(|p| p.text)
        .collect();
    assert!(texts.contains(&"one\ntwothree".to_string()), "{texts:?}");
}

#[tokio::test]
async fn sending_again_after_a_lost_answer_keeps_one_report() {
    let dir = tempfile::tempdir().unwrap();
    let app = router(node(dir.path(), Some(OWNER)));
    let body = report("Sent twice");
    let (st, a) = call(&app, Some(OWNER), "POST", "/v1/feedback", Some(body.clone())).await;
    assert_eq!(st, StatusCode::CREATED);
    let (st, b) = call(&app, Some(OWNER), "POST", "/v1/feedback", Some(body)).await;
    assert_eq!(st, StatusCode::OK, "the same report again: {b}");
    assert_eq!(a["id"], b["id"]);
    assert_eq!(
        inbox(dir.path()).pending(pm_api::feedback::now()).unwrap().len(),
        1
    );
}

#[tokio::test]
async fn each_person_sees_only_their_own_reports() {
    let dir = tempfile::tempdir().unwrap();
    let s = node(dir.path(), Some(OWNER));
    let ann = pm_api::accounts::mint(&s, ANN).unwrap();
    let ben = pm_api::accounts::mint(&s, BEN).unwrap();
    let app = router(s);
    for (who, text) in [
        (&ann, "ann's report"),
        (&ben, "ben's report"),
        (&OWNER.to_string(), "the owner's"),
    ] {
        let (st, _) = call(&app, Some(who), "POST", "/v1/feedback", Some(report(text))).await;
        assert_eq!(st, StatusCode::CREATED);
    }
    for (who, mine, others) in [
        (&ann, "ann's report", ["ben's report", "the owner's"]),
        (&ben, "ben's report", ["ann's report", "the owner's"]),
        (
            &OWNER.to_string(),
            "the owner's",
            ["ann's report", "ben's report"],
        ),
    ] {
        let (st, list) = call(&app, Some(who), "GET", "/v1/feedback/mine", None).await;
        assert_eq!(st, StatusCode::OK);
        let list = list.as_array().unwrap();
        assert_eq!(list.len(), 1, "{list:?}");
        assert_eq!(list[0]["preview"], mine);
        assert_eq!(list[0]["state"], "saved");
        for o in others {
            assert!(!Value::Array(list.clone()).to_string().contains(o));
        }
        assert!(list[0].get("person").is_none() && list[0].get("from").is_none());
    }
    // There is no list of everyone's reports over HTTP, for anyone
    let (st, _) = call(&app, Some(OWNER), "GET", "/v1/feedback", None).await;
    assert!(
        st == StatusCode::METHOD_NOT_ALLOWED || st == StatusCode::NOT_FOUND,
        "{st}"
    );
}

#[tokio::test]
async fn too_many_reports_wait_and_say_when() {
    let dir = tempfile::tempdir().unwrap();
    let s = node(dir.path(), Some(OWNER));
    let ann = pm_api::accounts::mint(&s, ANN).unwrap();
    let ben = pm_api::accounts::mint(&s, BEN).unwrap();
    let app = router(s);
    for i in 0..pm_api::feedback::PER_HOUR {
        let (st, _) = call(
            &app,
            Some(&ann),
            "POST",
            "/v1/feedback",
            Some(report(&format!("n{i}"))),
        )
        .await;
        assert_eq!(st, StatusCode::CREATED);
    }
    let req = Request::post("/v1/feedback")
        .header("authorization", format!("Bearer {ann}"))
        .header("content-type", "application/json")
        .body(Body::from(report("one too many").to_string()))
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::TOO_MANY_REQUESTS);
    let wait: u64 = resp.headers()["retry-after"].to_str().unwrap().parse().unwrap();
    assert!((1..=3600).contains(&wait), "{wait}");
    // Someone else still can
    let (st, _) = call(&app, Some(&ben), "POST", "/v1/feedback", Some(report("ben"))).await;
    assert_eq!(st, StatusCode::CREATED);
}

#[tokio::test]
async fn tokens_pasted_into_a_report_are_not_kept_or_logged() {
    let dir = tempfile::tempdir().unwrap();
    let s = node(dir.path(), Some(OWNER));
    let ann = pm_api::accounts::mint(&s, ANN).unwrap();
    let app = router(s);
    // The node's log from here on: sending and refusing reports
    let logs = Logs::default();
    let _guard = tracing::subscriber::set_default(
        tracing_subscriber::fmt()
            .with_max_level(tracing::Level::TRACE)
            .with_writer(logs.clone())
            .finish(),
    );
    let text = format!(
        "my token {ann} and the node's {OWNER}, a session pmo_AbCdEfGhIjKlMnOpQrStUv and a guest pmg_ZyXwVuTsRqPoNmLkJiHg end"
    );
    let (st, _) = call(&app, Some(&ann), "POST", "/v1/feedback", Some(report(&text))).await;
    assert_eq!(st, StatusCode::CREATED);
    let kept = stored(dir.path());
    for secret in [
        ann.as_str(),
        OWNER,
        "pmo_AbCdEfGhIjKlMnOpQrStUv",
        "pmg_ZyXwVuTsRqPoNmLkJiHg",
    ] {
        assert!(!kept.contains(secret), "{secret} kept");
    }
    assert!(
        kept.contains("[token removed]") && kept.contains(" end"),
        "{kept}"
    );
    // Refused and accepted bodies alike: no text, token or key in the node's log
    call(
        &app,
        Some(&ann),
        "POST",
        "/v1/feedback",
        Some(json!({"text": "LOGCANARY", "x": 1})),
    )
    .await;
    let log = logs.text();
    for secret in [ann.as_str(), OWNER, "LOGCANARY", "my token", ANN] {
        assert!(!log.contains(secret), "{secret} in the log: {log}");
    }
}

#[tokio::test]
async fn a_sign_out_while_a_report_is_on_its_way_saves_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let s = node(dir.path(), Some(OWNER));
    let ann = pm_api::accounts::mint(&s, ANN).unwrap();
    let app = router(s.clone());
    let (tx, rx) = tokio::sync::mpsc::channel::<Result<Vec<u8>, std::io::Error>>(1);
    let stream = tokio_stream_from(rx);
    let req = Request::post("/v1/feedback")
        .header("authorization", format!("Bearer {ann}"))
        .header("content-type", "application/json")
        .body(Body::from_stream(stream))
        .unwrap();
    let pending = tokio::spawn(send_owned(app.clone(), req));
    // The request is past the token check and waits for its body
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    assert!(!pending.is_finished());
    pm_api::accounts::sign_out_everywhere(&s, ANN).unwrap();
    assert!(
        pm_api::accounts::caller(&s, Some(&ann)).is_none(),
        "precondition: signed out"
    );
    tx.send(Ok(report("late").to_string().into_bytes()))
        .await
        .unwrap();
    drop(tx);
    let (st, _) = pending.await.unwrap();
    assert_eq!(st, StatusCode::UNAUTHORIZED);
    assert_eq!(stored(dir.path()), "[]");
}

#[tokio::test]
async fn the_operator_records_receipts_that_the_reporter_sees() {
    let dir = tempfile::tempdir().unwrap();
    let s = node(dir.path(), Some(OWNER));
    let ann = pm_api::accounts::mint(&s, ANN).unwrap();
    let app = router(s);
    let (_, r) = call(
        &app,
        Some(&ann),
        "POST",
        "/v1/feedback",
        Some(report("synthetic test report")),
    )
    .await;
    let id = r["id"].as_str().unwrap().to_string();

    let now = pm_api::feedback::now();
    assert!(inbox(dir.path())
        .receipt(&id, State::Received, "rcpt-1", now)
        .unwrap());
    // Received: no longer new for the operator, and the reporter sees it
    assert!(inbox(dir.path()).pending(now).unwrap().is_empty());
    let (_, list) = call(&app, Some(&ann), "GET", "/v1/feedback/mine", None).await;
    assert_eq!(list[0]["state"], "received");
    assert!(inbox(dir.path())
        .receipt(&id, State::Reviewed, "rcpt-2", now)
        .unwrap());
    let (_, list) = call(&app, Some(&ann), "GET", "/v1/feedback/mine", None).await;
    assert_eq!(list[0]["state"], "reviewed");
    // A state never goes back, an unknown report or a bad reference is refused
    assert!(inbox(dir.path())
        .receipt(&id, State::Received, "rcpt-3", now)
        .is_err());
    assert!(!inbox(dir.path())
        .receipt("fb_0000000000000000", State::Received, "r", now)
        .unwrap());
    assert!(inbox(dir.path())
        .receipt(&id, State::Reviewed, "bad ref; rm -rf", now)
        .is_err());
}

// ---- helpers ----

async fn send_owned(app: axum::Router, req: Request<Body>) -> (StatusCode, Value) {
    send(&app, req).await
}

fn tokio_stream_from(
    mut rx: tokio::sync::mpsc::Receiver<Result<Vec<u8>, std::io::Error>>,
) -> impl futures_util::Stream<Item = Result<Vec<u8>, std::io::Error>> {
    futures_util::stream::poll_fn(move |cx| rx.poll_recv(cx))
}

/// The node's log, kept for the test to read.
#[derive(Clone, Default)]
struct Logs(Arc<std::sync::Mutex<Vec<u8>>>);

impl Logs {
    fn text(&self) -> String {
        String::from_utf8_lossy(&self.0.lock().unwrap()).into_owned()
    }
}

impl std::io::Write for Logs {
    fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(b);
        Ok(b.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for Logs {
    type Writer = Logs;
    fn make_writer(&'a self) -> Logs {
        self.clone()
    }
}
