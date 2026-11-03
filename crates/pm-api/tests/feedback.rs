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
    // The node's log (every test's, so only this test's own canaries are looked for)
    let logs = logs();
    let text = format!(
        "my token {ann} and the node's {OWNER}, a session pmo_AbCdEfGhIjKlMnOpQrStUv and a guest pmg_ZyXwVuTsRqPoNmLkJiHg end"
    );
    let (st, saved) = call(&app, Some(&ann), "POST", "/v1/feedback", Some(report(&text))).await;
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
    let id = saved["id"].as_str().unwrap();
    assert!(log.contains(id), "the log was captured: {id} not in it");
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
    let asked = Arc::new(tokio::sync::Notify::new());
    let stream = tokio_stream_from(rx, asked.clone());
    let req = Request::post("/v1/feedback")
        .header("authorization", format!("Bearer {ann}"))
        .header("content-type", "application/json")
        .body(Body::from_stream(stream))
        .unwrap();
    let pending = tokio::spawn(send_owned(app.clone(), req));
    // The request is past the token check: the handler is reading its body
    tokio::time::timeout(std::time::Duration::from_secs(10), asked.notified())
        .await
        .expect("the handler asks for the body");
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

#[tokio::test]
async fn the_token_goes_in_the_header_never_the_address() {
    // A token in the address ends up in proxy logs and the browser's history
    let dir = tempfile::tempdir().unwrap();
    let app = router(node(dir.path(), Some(OWNER)));
    let (st, _) = call(
        &app,
        None,
        "POST",
        &format!("/v1/feedback?token={OWNER}"),
        Some(report("hi")),
    )
    .await;
    assert_eq!(st, StatusCode::UNAUTHORIZED);
    let (st, _) = call(
        &app,
        None,
        "GET",
        &format!("/v1/feedback/mine?token={OWNER}"),
        None,
    )
    .await;
    assert_eq!(st, StatusCode::UNAUTHORIZED);
    assert_eq!(stored(dir.path()), "[]");
}

#[tokio::test]
async fn an_edited_retry_is_not_taken_for_the_first_version() {
    let dir = tempfile::tempdir().unwrap();
    let app = router(node(dir.path(), Some(OWNER)));
    let first = report("First version");
    let (st, _) = call(&app, Some(OWNER), "POST", "/v1/feedback", Some(first.clone())).await;
    assert_eq!(st, StatusCode::CREATED);
    let mut edited = first;
    edited["text"] = json!("Edited after the answer was lost");
    let (st, r) = call(&app, Some(OWNER), "POST", "/v1/feedback", Some(edited)).await;
    assert_eq!(st, StatusCode::CONFLICT, "{r}");
    let texts: Vec<String> = inbox(dir.path())
        .pending(pm_api::feedback::now())
        .unwrap()
        .into_iter()
        .map(|p| p.text)
        .collect();
    assert_eq!(texts, ["First version"]);
}

#[tokio::test]
async fn a_full_inbox_says_so_and_reports_already_taken_in_leave_room() {
    let dir = tempfile::tempdir().unwrap();
    let app = router(node(dir.path(), Some(OWNER)));
    inbox(dir.path());
    let db = rusqlite::Connection::open(dir.path().join(pm_api::feedback::FILE)).unwrap();
    let old = pm_api::feedback::now() - 2 * 86_400;
    for i in 0..pm_api::feedback::MAX_REPORTS {
        db.execute(
            "INSERT INTO report (id, client_id, reporter, kind, text, sent_at, state)
             VALUES (?1, ?1, 'x', 'problem', 'x', ?2, 'saved')",
            rusqlite::params![format!("fb_{i:016x}"), old],
        )
        .unwrap();
    }
    let (st, r) = call(
        &app,
        Some(OWNER),
        "POST",
        "/v1/feedback",
        Some(report("one more")),
    )
    .await;
    assert_eq!(st, StatusCode::INSUFFICIENT_STORAGE, "{r}");
    // Taken in by the operator: no longer waiting in the inbox
    db.execute("UPDATE report SET state = 'received', received_at = ?1", [old])
        .unwrap();
    let (st, r) = call(
        &app,
        Some(OWNER),
        "POST",
        "/v1/feedback",
        Some(report("one more")),
    )
    .await;
    assert_eq!(st, StatusCode::CREATED, "{r}");
}

#[tokio::test]
async fn reports_are_never_in_a_backup() {
    let dir = tempfile::tempdir().unwrap();
    let s = node(dir.path(), Some(OWNER));
    let app = router(s.clone());
    let canary = "BACKUP-CANARY-report-text";
    let (st, _) = call(&app, Some(OWNER), "POST", "/v1/feedback", Some(report(canary))).await;
    assert_eq!(st, StatusCode::CREATED);
    let (zip, _) = pm_api::backup::make(&s, None).unwrap();
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(zip)).unwrap();
    assert!(!archive.is_empty());
    for i in 0..archive.len() {
        let mut f = archive.by_index(i).unwrap();
        let mut bytes = Vec::new();
        std::io::Read::read_to_end(&mut f, &mut bytes).unwrap();
        assert!(
            !String::from_utf8_lossy(&bytes).contains(canary),
            "{} holds a report",
            f.name()
        );
    }
}

#[tokio::test]
async fn a_listeners_short_name_is_this_nodes_own() {
    // Keyed with a secret of the node's: an export can't be matched to a public key, and the same
    // person has another short name on another node
    let mut people = Vec::new();
    for _ in 0..2 {
        let dir = tempfile::tempdir().unwrap();
        let s = node(dir.path(), Some(OWNER));
        let ann = pm_api::accounts::mint(&s, ANN).unwrap();
        let app = router(s);
        let (st, _) = call(&app, Some(&ann), "POST", "/v1/feedback", Some(report("hi"))).await;
        assert_eq!(st, StatusCode::CREATED);
        let (st, _) = call(&app, Some(&ann), "POST", "/v1/feedback", Some(report("again"))).await;
        assert_eq!(st, StatusCode::CREATED);
        let pending = inbox(dir.path()).pending(pm_api::feedback::now()).unwrap();
        assert_eq!(
            pending[0].person, pending[1].person,
            "one short name per person on a node"
        );
        people.push(pending[0].person.clone());
    }
    assert_ne!(people[0], people[1]);
    use sha2::Digest;
    let unkeyed = hex::encode(sha2::Sha256::digest(format!("pm-feedback-reporter:{ANN}")));
    assert!(!people.contains(&unkeyed[..8].to_string()));
}

#[tokio::test]
async fn a_node_with_a_short_api_token_takes_no_reports() {
    // A short owner token would be guessable through the reports (a scrubbed match changes the
    // text): reports wait until the token is long enough
    let dir = tempfile::tempdir().unwrap();
    let s = node(dir.path(), Some("k3y9z"));
    let ann = pm_api::accounts::mint(&s, ANN).unwrap();
    let app = router(s);
    for who in ["k3y9z", ann.as_str()] {
        let (st, r) = call(
            &app,
            Some(who),
            "POST",
            "/v1/feedback",
            Some(report("token k3y9z pasted")),
        )
        .await;
        assert_eq!(st, StatusCode::FORBIDDEN, "{r}");
        let (st, _) = call(&app, Some(who), "GET", "/v1/feedback/mine", None).await;
        assert_eq!(st, StatusCode::FORBIDDEN);
    }
    assert_eq!(stored(dir.path()), "[]");
}

#[tokio::test]
async fn whether_a_report_is_taken_never_depends_on_a_secret_in_it() {
    // The length is measured before tokens come out: the owner's token in a listener's text
    // changes nothing about the answer
    let dir = tempfile::tempdir().unwrap();
    let s = node(dir.path(), Some(OWNER));
    let ann = pm_api::accounts::mint(&s, ANN).unwrap();
    let app = router(s);
    let pad = |n: usize, with: &str| format!("{with}{}", "x".repeat(n - with.chars().count()));
    for n in [pm_api::feedback::MAX_TEXT, pm_api::feedback::MAX_TEXT + 1] {
        let (with, _) = call(
            &app,
            Some(&ann),
            "POST",
            "/v1/feedback",
            Some(report(&pad(n, OWNER))),
        )
        .await;
        let (without, _) = call(
            &app,
            Some(&ann),
            "POST",
            "/v1/feedback",
            Some(report(&pad(n, "y"))),
        )
        .await;
        assert_eq!(with, without, "{n} characters: the answer gave the token away");
    }
    assert!(!stored(dir.path()).contains(OWNER));
}

#[tokio::test]
async fn the_shortest_token_is_counted_in_characters() {
    // 6 characters in 18 bytes: short however many bytes it takes. Listeners only hear that
    // reports are off; the owner learns why
    let dir = tempfile::tempdir().unwrap();
    let s = node(dir.path(), Some("日本語の鍵だ"));
    let ann = pm_api::accounts::mint(&s, ANN).unwrap();
    let app = router(s);
    let (st, r) = call(&app, Some(&ann), "POST", "/v1/feedback", Some(report("hi"))).await;
    assert_eq!(st, StatusCode::FORBIDDEN, "{r}");
    assert!(
        !r.to_string().contains("16") && !r.to_string().contains("token"),
        "a listener learns nothing of the token: {r}"
    );
    assert_eq!(stored(dir.path()), "[]");
}

#[tokio::test]
async fn a_resent_report_never_tells_a_guess_from_the_owners_token() {
    // Sending the same report id again is compared on what was written, not on what is stored once
    // tokens are out: a guess at the owner's token gets the same answer, right or wrong
    let dir = tempfile::tempdir().unwrap();
    let s = node(dir.path(), Some(OWNER));
    let ann = pm_api::accounts::mint(&s, ANN).unwrap();
    let app = router(s);
    let id = uuid::Uuid::new_v4().to_string();
    let with = |text: &str| json!({"kind": "problem", "text": text, "client_id": id});
    let (st, _) = call(
        &app,
        Some(&ann),
        "POST",
        "/v1/feedback",
        Some(with("[token removed]")),
    )
    .await;
    assert_eq!(st, StatusCode::CREATED);
    let wrong = "x".repeat(OWNER.len());
    let (right_guess, _) = call(&app, Some(&ann), "POST", "/v1/feedback", Some(with(OWNER))).await;
    let (wrong_guess, _) = call(&app, Some(&ann), "POST", "/v1/feedback", Some(with(&wrong))).await;
    assert_eq!(right_guess, wrong_guess, "the answer gave the token away");
    assert_eq!(right_guess, StatusCode::CONFLICT);
    // The same words again are still the same report
    let (st, _) = call(
        &app,
        Some(&ann),
        "POST",
        "/v1/feedback",
        Some(with("[token removed]")),
    )
    .await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(
        inbox(dir.path()).pending(pm_api::feedback::now()).unwrap().len(),
        1
    );
}

// ---- helpers ----

async fn send_owned(app: axum::Router, req: Request<Body>) -> (StatusCode, Value) {
    send(&app, req).await
}

/// A body that arrives when the test sends it; `asked` is notified the first time it is read.
fn tokio_stream_from(
    mut rx: tokio::sync::mpsc::Receiver<Result<Vec<u8>, std::io::Error>>,
    asked: Arc<tokio::sync::Notify>,
) -> impl futures_util::Stream<Item = Result<Vec<u8>, std::io::Error>> {
    let mut first = true;
    futures_util::stream::poll_fn(move |cx| {
        if std::mem::take(&mut first) {
            asked.notify_one();
        }
        rx.poll_recv(cx)
    })
}

/// The node's log for this test binary: one global subscriber, set once. A subscriber set for one
/// thread misses events whose callsite another test's thread registered first.
fn logs() -> Logs {
    static LOGS: std::sync::OnceLock<Logs> = std::sync::OnceLock::new();
    LOGS.get_or_init(|| {
        let logs = Logs::default();
        tracing::subscriber::set_global_default(
            tracing_subscriber::fmt()
                .with_max_level(tracing::Level::TRACE)
                .with_ansi(false)
                .with_writer(logs.clone())
                .finish(),
        )
        .expect("one global subscriber");
        logs
    })
    .clone()
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
