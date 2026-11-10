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

fn set_roster(s: &NodeState, roster: &pm_api::people::Roster) {
    s.library
        .lock()
        .unwrap()
        .set_setting("people", &serde_json::to_string(roster).unwrap())
        .unwrap();
}

async fn restore_backup(app: &axum::Router, bytes: Vec<u8>, pass: Option<&str>) {
    let mut req = Request::post("/v1/restore")
        .header("authorization", format!("Bearer {OWNER}"))
        .header("content-type", "application/octet-stream");
    if let Some(pass) = pass {
        req = req.header(pm_api::backup::PASSPHRASE_HEADER, pass);
    }
    let (st, r) = send(app, req.body(Body::from(bytes)).unwrap()).await;
    assert_eq!(st, StatusCode::OK, "{r}");
}

/// Run the production membership cleanup once, under its production lock order. Unlike waiting
/// for an unchanged list, the returned count proves the delete step actually ran.
async fn clean_memberships(s: &Arc<NodeState>) -> usize {
    let s = s.clone();
    tokio::task::spawn_blocking(move || {
        let _one = s.roster.blocking_lock();
        pm_api::feedback::forget_unlisted(&s.data_dir(), &pm_api::people::music_reporters(&s).unwrap())
            .unwrap()
    })
    .await
    .unwrap()
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
        // The owner learns why; a listener only that reports are off
        let says_why = r["error"].as_str().unwrap().contains("16 characters");
        assert_eq!(says_why, who == "k3y9z", "{who}: {r}");
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
    let (st, r) = call(&app, Some(&ann), "GET", "/v1/feedback/mine", None).await;
    assert_eq!(
        st,
        StatusCode::FORBIDDEN,
        "counted in characters on both routes: {r}"
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

/// Claude review of b015b991, L2: People › Remove takes everything the node keeps for a
/// listener, their problem reports too; everyone else's stay.
#[tokio::test]
async fn removing_a_listener_takes_their_reports() {
    let dir = tempfile::tempdir().unwrap();
    let s = node(dir.path(), Some(OWNER));
    let ann = pm_api::accounts::mint(&s, ANN).unwrap();
    let ben = pm_api::accounts::mint(&s, BEN).unwrap();
    let app = router(s.clone());
    for (who, text) in [
        (&ann, "ann one"),
        (&ann, "ann two"),
        (&ben, "ben's"),
        (&OWNER.to_string(), "owner's"),
    ] {
        let (st, r) = call(&app, Some(who), "POST", "/v1/feedback", Some(report(text))).await;
        assert_eq!(st, StatusCode::CREATED, "{r}");
    }
    let (st, r) = call(&app, Some(OWNER), "DELETE", &format!("/v1/people/{ANN}"), None).await;
    assert_eq!(st, StatusCode::OK, "{r}");
    let mut left: Vec<String> = inbox(dir.path())
        .pending(pm_api::feedback::now())
        .unwrap()
        .into_iter()
        .map(|p| p.text)
        .collect();
    left.sort();
    assert_eq!(left, ["ben's", "owner's"]);
}

/// Under "This node's sign-ins" (the source policy `node`) a listener still reports problems and
/// sees their own: a report isn't a sign-in.
#[tokio::test]
async fn a_listener_reports_under_the_nodes_sign_ins() {
    let dir = tempfile::tempdir().unwrap();
    let s = node(dir.path(), Some(OWNER));
    s.library
        .lock()
        .unwrap()
        .set_setting(pm_api::accounts::SOURCE_POLICY_KEY, "node")
        .unwrap();
    let ann = pm_api::accounts::mint(&s, ANN).unwrap();
    let app = router(s.clone());
    let (st, r) = call(
        &app,
        Some(&ann),
        "POST",
        "/v1/feedback",
        Some(report("ann's, on the node's")),
    )
    .await;
    assert_eq!(st, StatusCode::CREATED, "{r}");
    let (st, r) = call(&app, Some(&ann), "GET", "/v1/feedback/mine", None).await;
    assert_eq!(st, StatusCode::OK, "{r}");
    assert_eq!(r.as_array().map(Vec::len), Some(1), "{r}");
}

/// The texts of the reports in the inbox, sorted.
fn texts(dir: &std::path::Path) -> Vec<String> {
    let mut t: Vec<String> = inbox(dir)
        .pending(pm_api::feedback::now())
        .unwrap()
        .into_iter()
        .map(|p| p.text)
        .collect();
    t.sort();
    t
}

/// Ann, Ben and the owner each send a report.
async fn three_reports(app: &axum::Router, ann: &str, ben: &str) {
    for (who, text) in [(ann, "ann's"), (ben, "ben's"), (OWNER, "owner's")] {
        let (st, r) = call(app, Some(who), "POST", "/v1/feedback", Some(report(text))).await;
        assert_eq!(st, StatusCode::CREATED, "{r}");
    }
}

/// Waits up to 15 s for the inbox to hold exactly `want`.
async fn until_left(dir: &std::path::Path, want: &[&str]) -> Vec<String> {
    let mut left = texts(dir);
    for _ in 0..150 {
        if left == want {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        left = texts(dir);
    }
    left
}

/// People › Remove while the inbox is held longer than it waits (Kimi review of fc1dca51, M1): the
/// listener is removed, and their reports go at the next housekeeping, which the node runs at
/// start and every hour. Nobody else's go.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn reports_a_busy_inbox_kept_at_a_remove_go_at_housekeeping() {
    let dir = tempfile::tempdir().unwrap();
    let s = node(dir.path(), Some(OWNER));
    let ann = pm_api::accounts::mint(&s, ANN).unwrap();
    let ben = pm_api::accounts::mint(&s, BEN).unwrap();
    let app = router(s.clone());
    three_reports(&app, &ann, &ben).await;
    let hold = rusqlite::Connection::open(dir.path().join(pm_api::feedback::FILE)).unwrap();
    hold.execute_batch("BEGIN EXCLUSIVE").unwrap();
    let (st, r) = call(&app, Some(OWNER), "DELETE", &format!("/v1/people/{ANN}"), None).await;
    assert_eq!(st, StatusCode::OK, "{r}");
    hold.execute_batch("ROLLBACK").unwrap();
    drop(hold);
    assert_eq!(
        texts(dir.path()),
        ["ann's", "ben's", "owner's"],
        "precondition: the remove couldn't reach the inbox"
    );
    pm_api::feedback::start_housekeeping(&s);
    assert_eq!(
        until_left(dir.path(), &["ben's", "owner's"]).await,
        ["ben's", "owner's"]
    );
}

/// A restore brings back a roster without Ann (held Low 6 of the Claude review of 2d2f0a8a): her
/// reports go at the next housekeeping, as a People › Remove would have taken them.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn reports_of_someone_no_longer_on_the_roster_go_at_housekeeping() {
    let dir = tempfile::tempdir().unwrap();
    let s = node(dir.path(), Some(OWNER));
    let ann = pm_api::accounts::mint(&s, ANN).unwrap();
    let ben = pm_api::accounts::mint(&s, BEN).unwrap();
    let app = router(s.clone());
    three_reports(&app, &ann, &ben).await;
    let only_ben = json!({"people": [{"key": BEN, "access": "music", "music_enabled": true, "added_at": 0}]});
    s.library
        .lock()
        .unwrap()
        .set_setting("people", &only_ben.to_string())
        .unwrap();
    pm_api::feedback::start_housekeeping(&s);
    assert_eq!(
        until_left(dir.path(), &["ben's", "owner's"]).await,
        ["ben's", "owner's"]
    );
}

/// A roster the node can't read is never taken for an empty one: no report goes for it.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_unreadable_roster_takes_no_reports() {
    let dir = tempfile::tempdir().unwrap();
    let s = node(dir.path(), Some(OWNER));
    let ann = pm_api::accounts::mint(&s, ANN).unwrap();
    let ben = pm_api::accounts::mint(&s, BEN).unwrap();
    let app = router(s.clone());
    three_reports(&app, &ann, &ben).await;
    s.library
        .lock()
        .unwrap()
        .set_setting("people", "{not a roster")
        .unwrap();
    pm_api::feedback::start_housekeeping(&s);
    tokio::time::sleep(std::time::Duration::from_millis(1500)).await;
    assert_eq!(texts(dir.path()), ["ann's", "ben's", "owner's"]);
}

/// Signed out while My reports waits for the inbox (Kimi review of fc1dca51, M2): the answer is
/// a 401, never the list.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_sign_out_while_my_reports_are_read_shows_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let s = node(dir.path(), Some(OWNER));
    let ann = pm_api::accounts::mint(&s, ANN).unwrap();
    let app = router(s.clone());
    let (st, r) = call(
        &app,
        Some(&ann),
        "POST",
        "/v1/feedback",
        Some(report("ann's own")),
    )
    .await;
    assert_eq!(st, StatusCode::CREATED, "{r}");
    // Another connection holds the inbox: the read waits there, past the token check
    let hold = rusqlite::Connection::open(dir.path().join(pm_api::feedback::FILE)).unwrap();
    hold.execute_batch("BEGIN EXCLUSIVE").unwrap();
    let req = Request::get("/v1/feedback/mine")
        .header("authorization", format!("Bearer {ann}"))
        .body(Body::empty())
        .unwrap();
    let pending = tokio::spawn(send_owned(app.clone(), req));
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    assert!(
        !pending.is_finished(),
        "precondition: the read waits for the inbox"
    );
    pm_api::accounts::sign_out_everywhere(&s, ANN).unwrap();
    hold.execute_batch("ROLLBACK").unwrap();
    drop(hold);
    let (st, r) = pending.await.unwrap();
    assert_eq!(st, StatusCode::UNAUTHORIZED, "{r}");
    assert!(!r.to_string().contains("ann's own"), "{r}");
}

/// `feedback.sqlite` is a link to another database (Kimi review of fc1dca51, M3): the inbox
/// refuses it, and the database it points to is left exactly as it was, mode included.
#[cfg(unix)]
#[tokio::test]
async fn a_link_in_place_of_the_inbox_is_refused_and_its_target_left_alone() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let s = node(dir.path(), Some(OWNER));
    let app = router(s.clone());
    let elsewhere = dir.path().join("elsewhere.sqlite");
    {
        let db = rusqlite::Connection::open(&elsewhere).unwrap();
        db.execute_batch("CREATE TABLE keep (x TEXT); INSERT INTO keep VALUES ('as it was');")
            .unwrap();
    }
    std::fs::set_permissions(&elsewhere, std::fs::Permissions::from_mode(0o644)).unwrap();
    let before = std::fs::read(&elsewhere).unwrap();
    std::os::unix::fs::symlink(&elsewhere, dir.path().join(pm_api::feedback::FILE)).unwrap();
    let (st, r) = call(&app, Some(OWNER), "POST", "/v1/feedback", Some(report("owner's"))).await;
    assert_eq!(st, StatusCode::INTERNAL_SERVER_ERROR, "{r}");
    let (st, r) = call(&app, Some(OWNER), "GET", "/v1/feedback/mine", None).await;
    assert_eq!(st, StatusCode::INTERNAL_SERVER_ERROR, "{r}");
    assert!(Inbox::open(dir.path()).is_err());
    assert!(Inbox::open_existing(dir.path()).is_err());
    assert!(pm_api::feedback::housekeeping(dir.path(), pm_api::feedback::now()).is_err());
    assert_eq!(
        std::fs::read(&elsewhere).unwrap(),
        before,
        "the linked database is unchanged"
    );
    let mode = std::fs::metadata(&elsewhere).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o644, "the linked database's mode is unchanged");
}

/// People › Remove of someone with an account on the node's homeserver keeps them listed, with
/// their music taken away (Claude review of 9ee7e1e3, Medium 1): with the inbox busy at the
/// remove, their reports still go at the next housekeeping. Music paused (People › music off)
/// isn't a remove: Ben's reports stay.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_removed_homeserver_listeners_reports_go_at_housekeeping() {
    let dir = tempfile::tempdir().unwrap();
    let s = node(dir.path(), Some(OWNER));
    let roster = json!({"people": [
        {"key": ANN, "access": "homeserver_music", "music_enabled": true, "added_at": 0},
        {"key": BEN, "access": "music", "music_enabled": true, "added_at": 0}]});
    s.library
        .lock()
        .unwrap()
        .set_setting("people", &roster.to_string())
        .unwrap();
    let ann = pm_api::accounts::mint(&s, ANN).unwrap();
    let ben = pm_api::accounts::mint(&s, BEN).unwrap();
    let app = router(s.clone());
    three_reports(&app, &ann, &ben).await;
    let hold = rusqlite::Connection::open(dir.path().join(pm_api::feedback::FILE)).unwrap();
    hold.execute_batch("BEGIN EXCLUSIVE").unwrap();
    let (st, r) = call(&app, Some(OWNER), "DELETE", &format!("/v1/people/{ANN}"), None).await;
    assert_eq!(st, StatusCode::OK, "{r}");
    hold.execute_batch("ROLLBACK").unwrap();
    drop(hold);
    let (st, r) = call(&app, Some(OWNER), "GET", "/v1/people", None).await;
    assert_eq!(st, StatusCode::OK, "{r}");
    assert!(
        r.to_string().contains(ANN),
        "precondition: Ann stays listed for her homeserver account: {r}"
    );
    let paused = json!({"people": [
        {"key": ANN, "access": "homeserver", "music_enabled": false, "added_at": 0},
        {"key": BEN, "access": "music", "music_enabled": false, "added_at": 0}]});
    s.library
        .lock()
        .unwrap()
        .set_setting("people", &paused.to_string())
        .unwrap();
    assert_eq!(
        texts(dir.path()),
        ["ann's", "ben's", "owner's"],
        "precondition: the remove couldn't reach the inbox"
    );
    pm_api::feedback::start_housekeeping(&s);
    assert_eq!(
        until_left(dir.path(), &["ben's", "owner's"]).await,
        ["ben's", "owner's"]
    );
}

/// Opening the inbox leaves every SQLite lock this process holds on it in place (Claude review
/// of 9ee7e1e3, Medium 2): closing any other descriptor to the file would drop them all (POSIX;
/// sqlite.org/howtocorrupt.html, 2.2), and another process (the operator's `pm-node feedback`)
/// could then take a write in progress for a crashed one. Here another connection holds the
/// inbox's write lock while the inbox is opened; a separate process still finds it locked.
#[cfg(unix)]
#[tokio::test]
async fn opening_the_inbox_keeps_the_locks_this_process_holds() {
    let dir = tempfile::tempdir().unwrap();
    let s = node(dir.path(), Some(OWNER));
    let app = router(s.clone());
    let (st, r) = call(&app, Some(OWNER), "POST", "/v1/feedback", Some(report("owner's"))).await;
    assert_eq!(st, StatusCode::CREATED, "{r}");
    let path = dir.path().join(pm_api::feedback::FILE);
    let other_process_writes = || {
        std::process::Command::new("python3")
            .args([
                "-I",
                "-c",
                "import sqlite3, sys\nc = sqlite3.connect(sys.argv[1], timeout=0)\nc.execute('BEGIN IMMEDIATE')\nc.rollback()",
            ])
            .arg(&path)
            .output()
            .expect("python3 runs")
            .status
            .success()
    };
    assert!(
        other_process_writes(),
        "precondition: unlocked, another process may write"
    );
    let hold = rusqlite::Connection::open(&path).unwrap();
    hold.execute_batch("BEGIN IMMEDIATE").unwrap();
    assert!(!other_process_writes(), "precondition: the write lock is held");
    // Opening waits on that lock (5 s) and gives up; whatever it answers, the lock must stay
    let _ = Inbox::open(dir.path());
    let _ = Inbox::open_existing(dir.path());
    assert!(
        !other_process_writes(),
        "another process could write while this one holds the inbox's write lock"
    );
    hold.execute_batch("ROLLBACK").unwrap();
}

/// A hard link in place of the inbox, to another database (Claude review of 9ee7e1e3, Low 1):
/// refused like a symbolic one, and the linked database is left exactly as it was.
#[cfg(unix)]
#[tokio::test]
async fn a_hard_link_in_place_of_the_inbox_is_refused_and_its_target_left_alone() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let s = node(dir.path(), Some(OWNER));
    let app = router(s.clone());
    let elsewhere = dir.path().join("elsewhere.sqlite");
    {
        let db = rusqlite::Connection::open(&elsewhere).unwrap();
        db.execute_batch("CREATE TABLE keep (x TEXT); INSERT INTO keep VALUES ('as it was');")
            .unwrap();
    }
    std::fs::set_permissions(&elsewhere, std::fs::Permissions::from_mode(0o644)).unwrap();
    let before = std::fs::read(&elsewhere).unwrap();
    std::fs::hard_link(&elsewhere, dir.path().join(pm_api::feedback::FILE)).unwrap();
    let (st, r) = call(&app, Some(OWNER), "POST", "/v1/feedback", Some(report("owner's"))).await;
    assert_eq!(st, StatusCode::INTERNAL_SERVER_ERROR, "{r}");
    assert!(Inbox::open(dir.path()).is_err());
    assert_eq!(
        std::fs::read(&elsewhere).unwrap(),
        before,
        "the linked database is unchanged"
    );
    let mode = std::fs::metadata(&elsewhere).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o644, "the linked database's mode is unchanged");
}

/// Removed while the inbox is busy, then given music again before any housekeeping (Kimi review
/// of a58e5a5a, M1): the reports from before the remove are never Ann's again (My reports shows
/// none) and go at housekeeping; what she sends after stays, and so does everyone else's.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn reports_from_before_a_remove_never_come_back_with_music_again() {
    let dir = tempfile::tempdir().unwrap();
    let s = node(dir.path(), Some(OWNER));
    let roster = json!({"people": [
        {"key": ANN, "access": "homeserver_music", "music_enabled": true, "added_at": 0},
        {"key": BEN, "access": "music", "music_enabled": true, "added_at": 0}]});
    s.library
        .lock()
        .unwrap()
        .set_setting("people", &roster.to_string())
        .unwrap();
    let ann = pm_api::accounts::mint(&s, ANN).unwrap();
    let ben = pm_api::accounts::mint(&s, BEN).unwrap();
    let app = router(s.clone());
    three_reports(&app, &ann, &ben).await;
    let hold = rusqlite::Connection::open(dir.path().join(pm_api::feedback::FILE)).unwrap();
    hold.execute_batch("BEGIN EXCLUSIVE").unwrap();
    let (st, r) = call(&app, Some(OWNER), "DELETE", &format!("/v1/people/{ANN}"), None).await;
    assert_eq!(st, StatusCode::OK, "{r}");
    hold.execute_batch("ROLLBACK").unwrap();
    drop(hold);
    // Music again, through People, before any housekeeping
    let (st, r) = call(
        &app,
        Some(OWNER),
        "PATCH",
        &format!("/v1/people/{ANN}"),
        Some(json!({"music": true})),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{r}");
    let ann = pm_api::accounts::mint(&s, ANN).unwrap();
    let (st, r) = call(&app, Some(&ann), "GET", "/v1/feedback/mine", None).await;
    assert_eq!(st, StatusCode::OK, "{r}");
    assert_eq!(
        r.as_array().map(Vec::len),
        Some(0),
        "none from before the remove: {r}"
    );
    let (st, r) = call(
        &app,
        Some(&ann),
        "POST",
        "/v1/feedback",
        Some(report("ann's new")),
    )
    .await;
    assert_eq!(st, StatusCode::CREATED, "{r}");
    pm_api::feedback::start_housekeeping(&s);
    let want = ["ann's new", "ben's", "owner's"];
    assert_eq!(until_left(dir.path(), &want).await, want);
}

/// A successful Remove followed by a music-only re-invite creates B. Restoring A (named or
/// legacy), through the real plain/sealed HTTP restore, must keep B's accepted reports.
#[tokio::test]
async fn restoring_an_older_backup_keeps_current_members_reports() {
    for (pass, old_name) in [(None, Some("membership-a")), (Some("fixture-pass"), None)] {
        let dir = tempfile::tempdir().unwrap();
        let s = node(dir.path(), Some(OWNER));
        let mut roster = pm_api::people::load(&s);
        roster.people[0].reports = old_name.map(str::to_string);
        set_roster(&s, &roster);
        let ann = pm_api::accounts::mint(&s, ANN).unwrap();
        let ben = pm_api::accounts::mint(&s, BEN).unwrap();
        let app = router(s.clone());
        three_reports(&app, &ann, &ben).await;
        let (backup, _) = pm_api::backup::make(&s, pass).unwrap();
        let (st, r) = call(&app, Some(OWNER), "DELETE", &format!("/v1/people/{ANN}"), None).await;
        assert_eq!(st, StatusCode::OK, "{r}");
        assert_eq!(texts(dir.path()), ["ben's", "owner's"], "removal succeeded");
        let (st, r) = call(
            &app,
            Some(OWNER),
            "POST",
            "/v1/people/invites",
            Some(json!({"access": "music", "key": ANN})),
        )
        .await;
        assert_eq!(st, StatusCode::OK, "{r}");
        let current = pm_api::people::reports_of(&s, ANN);
        assert!(current.is_some());
        assert_ne!(current.as_deref(), old_name);
        let ann = pm_api::accounts::mint(&s, ANN).unwrap();
        let (st, saved) = call(
            &app,
            Some(&ann),
            "POST",
            "/v1/feedback",
            Some(report("ann's new")),
        )
        .await;
        assert_eq!(st, StatusCode::CREATED, "{saved}");
        // The offline CLI's staging merge is exercised against the same real A -> Remove -> B
        // history and inbox, before the HTTP restore replaces the live library.
        let staged_path = dir.path().join("cli-staged.sqlite");
        std::fs::write(&staged_path, pm_api::backup::parse(&backup).unwrap().database).unwrap();
        {
            let staged = Library::open(&staged_path).unwrap();
            pm_api::backup::keep_this_nodes(&s.library.lock().unwrap(), &staged).unwrap();
            let roster: pm_api::people::Roster =
                serde_json::from_str(&staged.get_setting("people").unwrap().unwrap()).unwrap();
            let ann = roster.people.iter().find(|p| p.key == ANN).unwrap();
            assert_eq!(ann.reports, current);
            let keep: Vec<_> = roster
                .people
                .into_iter()
                .filter(|p| p.access.music())
                .map(|p| (p.key, p.reports))
                .collect();
            assert_eq!(pm_api::feedback::forget_unlisted(dir.path(), &keep).unwrap(), 0);
        }
        restore_backup(&app, backup, pass).await;
        assert_eq!(pm_api::people::reports_of(&s, ANN), current);
        // Restore also replaces listener sessions; sign in again to check the report identity.
        let ann = pm_api::accounts::mint(&s, ANN).unwrap();
        let (st, mine) = call(&app, Some(&ann), "GET", "/v1/feedback/mine", None).await;
        assert_eq!(st, StatusCode::OK, "{mine}");
        assert_eq!(mine.as_array().unwrap().len(), 1);
        assert_eq!(mine[0]["id"], saved["id"]);
        assert_eq!(clean_memberships(&s).await, 0);
        assert_eq!(texts(dir.path()), ["ann's new", "ben's", "owner's"]);
    }
}

/// A restore from another roster must preserve this node's legacy None membership, and named
/// memberships whose music is paused. Restoring enabled access does not grant either anew.
#[tokio::test]
async fn restoring_music_keeps_legacy_and_paused_members_reports() {
    for (membership, paused) in [(None, false), (Some("paused-membership"), true)] {
        let dir = tempfile::tempdir().unwrap();
        let s = node(dir.path(), Some(OWNER));
        let mut roster = pm_api::people::load(&s);
        roster.people[0].reports = membership.map(str::to_string);
        set_roster(&s, &roster);
        let ann = pm_api::accounts::mint(&s, ANN).unwrap();
        let app = router(s.clone());
        let (st, saved) = call(
            &app,
            Some(&ann),
            "POST",
            "/v1/feedback",
            Some(report("ann's kept")),
        )
        .await;
        assert_eq!(st, StatusCode::CREATED, "{saved}");
        if paused {
            let (st, r) = call(
                &app,
                Some(OWNER),
                "PATCH",
                &format!("/v1/people/{ANN}"),
                Some(json!({"music": false})),
            )
            .await;
            assert_eq!(st, StatusCode::OK, "{r}");
        }
        let other_dir = tempfile::tempdir().unwrap();
        let other = node(other_dir.path(), Some(OWNER));
        let mut older = pm_api::people::load(&other);
        older.people[0].reports = Some("backup-membership".into());
        set_roster(&other, &older);
        let (backup, _) = pm_api::backup::make(&other, None).unwrap();
        restore_backup(&app, backup, None).await;
        assert_eq!(pm_api::people::reports_of(&s, ANN).as_deref(), membership);
        let ann = pm_api::accounts::mint(&s, ANN).unwrap();
        let (st, mine) = call(&app, Some(&ann), "GET", "/v1/feedback/mine", None).await;
        assert_eq!(st, StatusCode::OK, "{mine}");
        assert_eq!(mine[0]["id"], saved["id"]);
        assert_eq!(clean_memberships(&s).await, 0);
        assert_eq!(texts(dir.path()), ["ann's kept"]);
    }
}

/// Restoring music to a removed listener is a fresh grant, even if an old inbox row survived a
/// busy Remove. It must neither revive that report nor delete the new membership's report.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn restoring_removed_music_never_revives_its_old_reports() {
    let dir = tempfile::tempdir().unwrap();
    let s = node(dir.path(), Some(OWNER));
    let mut roster = pm_api::people::load(&s);
    roster.people[0].access = pm_api::people::Access::HomeserverMusic;
    roster.people[0].reports = Some("membership-a".into());
    set_roster(&s, &roster);
    let ann = pm_api::accounts::mint(&s, ANN).unwrap();
    let ben = pm_api::accounts::mint(&s, BEN).unwrap();
    let app = router(s.clone());
    three_reports(&app, &ann, &ben).await;
    let (backup, _) = pm_api::backup::make(&s, None).unwrap();
    let hold = rusqlite::Connection::open(dir.path().join(pm_api::feedback::FILE)).unwrap();
    hold.execute_batch("BEGIN EXCLUSIVE").unwrap();
    let (st, r) = call(&app, Some(OWNER), "DELETE", &format!("/v1/people/{ANN}"), None).await;
    assert_eq!(st, StatusCode::OK, "{r}");
    hold.execute_batch("ROLLBACK").unwrap();
    drop(hold);
    assert!(!pm_api::people::may_use_music(&s, ANN));
    assert_eq!(texts(dir.path()), ["ann's", "ben's", "owner's"]);
    restore_backup(&app, backup, None).await;
    let current = pm_api::people::reports_of(&s, ANN).unwrap();
    assert_ne!(current, "membership-a");
    let ann = pm_api::accounts::mint(&s, ANN).unwrap();
    let (st, mine) = call(&app, Some(&ann), "GET", "/v1/feedback/mine", None).await;
    assert_eq!(st, StatusCode::OK, "{mine}");
    assert!(mine.as_array().unwrap().is_empty());
    let (st, r) = call(
        &app,
        Some(&ann),
        "POST",
        "/v1/feedback",
        Some(report("ann's new")),
    )
    .await;
    assert_eq!(st, StatusCode::CREATED, "{r}");
    assert_eq!(clean_memberships(&s).await, 1);
    assert_eq!(texts(dir.path()), ["ann's new", "ben's", "owner's"]);
}

/// The library can refuse a roster write while the separate inbox is writable. A failed revoke
/// must not destroy reports; a successful retry removes every membership of only that key.
#[tokio::test]
async fn a_failed_roster_write_keeps_access_and_every_report() {
    let dir = tempfile::tempdir().unwrap();
    let s = node(dir.path(), Some(OWNER));
    let mut roster = pm_api::people::load(&s);
    roster.people[0].reports = Some("membership-a".into());
    roster.people[1].reports = Some("ben-membership".into());
    set_roster(&s, &roster);
    let ann = pm_api::accounts::mint(&s, ANN).unwrap();
    let ben = pm_api::accounts::mint(&s, BEN).unwrap();
    let app = router(s.clone());
    three_reports(&app, &ann, &ben).await;
    // Rows from two memberships, as after an earlier cleanup failure and re-add.
    roster.people[0].reports = Some("membership-b".into());
    set_roster(&s, &roster);
    let (st, r) = call(
        &app,
        Some(&ann),
        "POST",
        "/v1/feedback",
        Some(report("ann's new")),
    )
    .await;
    assert_eq!(st, StatusCode::CREATED, "{r}");
    let db = rusqlite::Connection::open(dir.path().join("lib.sqlite")).unwrap();
    db.execute_batch(
        "CREATE TRIGGER no_people_i BEFORE INSERT ON settings WHEN NEW.key = 'people'
           BEGIN SELECT RAISE(ABORT, 'fixture: roster write refused'); END;
         CREATE TRIGGER no_people_u BEFORE UPDATE ON settings WHEN NEW.key = 'people'
           BEGIN SELECT RAISE(ABORT, 'fixture: roster write refused'); END;",
    )
    .unwrap();
    let (st, r) = call(&app, Some(OWNER), "DELETE", &format!("/v1/people/{ANN}"), None).await;
    assert_eq!(st, StatusCode::INTERNAL_SERVER_ERROR, "{r}");
    assert_eq!(pm_api::people::load(&s), roster);
    assert!(pm_api::people::may_use_music(&s, ANN));
    assert!(pm_api::accounts::caller(&s, Some(&ann)).is_some());
    assert_eq!(texts(dir.path()), ["ann's", "ann's new", "ben's", "owner's"]);
    db.execute_batch("DROP TRIGGER no_people_i; DROP TRIGGER no_people_u;")
        .unwrap();
    let (st, r) = call(&app, Some(OWNER), "DELETE", &format!("/v1/people/{ANN}"), None).await;
    assert_eq!(st, StatusCode::OK, "{r}");
    assert!(!pm_api::people::may_use_music(&s, ANN));
    assert_eq!(texts(dir.path()), ["ben's", "owner's"]);
}

/// Moving cleanup off the runtime must not release roster exclusion when its request is
/// cancelled: the worker's all-membership delete finishes before a re-invite can save B.
#[tokio::test]
async fn cancelling_remove_cannot_delete_reports_from_a_later_membership() {
    let dir = tempfile::tempdir().unwrap();
    let s = node(dir.path(), Some(OWNER));
    let ann = pm_api::accounts::mint(&s, ANN).unwrap();
    let app = router(s.clone());
    let (st, r) = call(&app, Some(&ann), "POST", "/v1/feedback", Some(report("ann's"))).await;
    assert_eq!(st, StatusCode::CREATED, "{r}");
    let hold = rusqlite::Connection::open(dir.path().join(pm_api::feedback::FILE)).unwrap();
    hold.execute_batch("BEGIN EXCLUSIVE").unwrap();
    let req = Request::delete(format!("/v1/people/{ANN}"))
        .header("authorization", format!("Bearer {OWNER}"))
        .body(Body::empty())
        .unwrap();
    let removal = tokio::spawn(send_owned(app.clone(), req));
    tokio::time::timeout(std::time::Duration::from_secs(3), async {
        while pm_api::people::may_use_music(&s, ANN) {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("revocation is persisted while inbox cleanup waits");
    assert!(!removal.is_finished(), "the inbox is still locked");
    removal.abort();
    assert!(removal.await.unwrap_err().is_cancelled());
    assert!(s.roster.try_lock().is_err(), "the worker still excludes re-adds");
    let again = app.clone();
    let mut readd = tokio::spawn(async move {
        call(
            &again,
            Some(OWNER),
            "POST",
            "/v1/people/invites",
            Some(json!({"access": "music", "key": ANN})),
        )
        .await
    });
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(100), &mut readd)
            .await
            .is_err()
    );
    hold.execute_batch("ROLLBACK").unwrap();
    drop(hold);
    let (st, r) = readd.await.unwrap();
    assert_eq!(st, StatusCode::OK, "{r}");
    assert!(
        pm_api::accounts::caller(&s, Some(&ann)).is_none(),
        "cancelling Remove must not preserve the old sign-in after a re-add"
    );
    let ann = pm_api::accounts::mint(&s, ANN).unwrap();
    let (st, r) = call(
        &app,
        Some(&ann),
        "POST",
        "/v1/feedback",
        Some(report("ann's new")),
    )
    .await;
    assert_eq!(st, StatusCode::CREATED, "{r}");
    assert_eq!(clean_memberships(&s).await, 0);
    assert_eq!(texts(dir.path()), ["ann's new"]);
}

/// Housekeeping reads the roster and deletes by it in one step, never while a change to the
/// roster is under way (Kimi review of a58e5a5a, M2): Ann is given music again during it and sends
/// a new report; only her reports from before go.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn housekeeping_waits_for_a_roster_change_and_keeps_what_came_after() {
    let dir = tempfile::tempdir().unwrap();
    let s = node(dir.path(), Some(OWNER));
    let ann = pm_api::accounts::mint(&s, ANN).unwrap();
    let ben = pm_api::accounts::mint(&s, BEN).unwrap();
    let app = router(s.clone());
    three_reports(&app, &ann, &ben).await;
    let hold = rusqlite::Connection::open(dir.path().join(pm_api::feedback::FILE)).unwrap();
    hold.execute_batch("BEGIN EXCLUSIVE").unwrap();
    let (st, r) = call(&app, Some(OWNER), "DELETE", &format!("/v1/people/{ANN}"), None).await;
    assert_eq!(st, StatusCode::OK, "{r}");
    hold.execute_batch("ROLLBACK").unwrap();
    drop(hold);
    // A change to the roster is under way (People and restores hold this while they change it)
    let busy = s.roster.lock().await;
    pm_api::feedback::start_housekeeping(&s);
    tokio::time::sleep(std::time::Duration::from_millis(1500)).await;
    assert_eq!(
        texts(dir.path()),
        ["ann's", "ben's", "owner's"],
        "housekeeping waits for the roster change"
    );
    // The change: Ann has music again, as a new membership, and sends a new report meanwhile
    let again = json!({"people": [
        {"key": ANN, "access": "music", "music_enabled": true, "added_at": 1, "reports": "membership-2"},
        {"key": BEN, "access": "music", "music_enabled": true, "added_at": 0}]});
    s.library
        .lock()
        .unwrap()
        .set_setting("people", &again.to_string())
        .unwrap();
    let ann = pm_api::accounts::mint(&s, ANN).unwrap();
    let (st, r) = call(
        &app,
        Some(&ann),
        "POST",
        "/v1/feedback",
        Some(report("ann's new")),
    )
    .await;
    assert_eq!(st, StatusCode::CREATED, "{r}");
    drop(busy);
    let want = ["ann's new", "ben's", "owner's"];
    assert_eq!(until_left(dir.path(), &want).await, want);
}

/// SQLite's own files beside the inbox, there at 0644 before it is opened (Kimi review of
/// a58e5a5a, M4), are made 0600 before the inbox is used. `-shm` is one SQLite never touches in
/// the inbox's journal mode, so what the node does to it is seen as it is; `-journal` and `-wal`
/// are either made private or taken away by SQLite.
#[cfg(unix)]
#[tokio::test]
async fn sqlite_files_beside_the_inbox_are_made_private_before_use() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let s = node(dir.path(), Some(OWNER));
    let app = router(s.clone());
    let (st, r) = call(&app, Some(OWNER), "POST", "/v1/feedback", Some(report("owner's"))).await;
    assert_eq!(st, StatusCode::CREATED, "{r}");
    for ext in ["-shm", "-journal", "-wal"] {
        let side = dir.path().join(format!("{}{ext}", pm_api::feedback::FILE));
        std::fs::write(&side, b"").unwrap();
        std::fs::set_permissions(&side, std::fs::Permissions::from_mode(0o644)).unwrap();
        let opened = Inbox::open(dir.path());
        assert!(opened.is_ok(), "{ext}");
        let mode = std::fs::metadata(&side).map(|m| m.permissions().mode() & 0o777);
        if ext == "-shm" {
            assert_eq!(mode.unwrap(), 0o600, "{ext} made private");
        } else {
            assert!(matches!(mode, Ok(0o600) | Err(_)), "{ext}: {mode:?}");
        }
        drop(opened);
        let _ = std::fs::remove_file(&side);
    }
    assert_eq!(texts(dir.path()), ["owner's"]);
}

/// The inbox opens only in a data folder no other user may write in (Kimi review of a58e5a5a,
/// M3): putting another file in its place (or swapping one in while it is opened) takes write
/// access to that folder, which only the node's own user then has.
#[cfg(unix)]
#[test]
fn the_inbox_opens_only_in_a_folder_no_one_else_may_write_in() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let data = dir.path().join("data");
    Inbox::open(&data).unwrap();
    for (mode, ok) in [
        (0o777, false),
        (0o775, false),
        (0o757, false),
        (0o711, true),
        (0o700, true),
    ] {
        std::fs::set_permissions(&data, std::fs::Permissions::from_mode(mode)).unwrap();
        assert_eq!(Inbox::open(&data).is_ok(), ok, "{mode:o}");
        assert_eq!(Inbox::open_existing(&data).is_ok(), ok, "{mode:o}");
    }
}

/// An inbox the node can't look at is an error, never "no inbox" (Kimi review of a58e5a5a, Low
/// 7): My reports and the operator's pending list say so instead of showing none.
#[cfg(unix)]
#[test]
fn an_inbox_that_cant_be_looked_at_is_an_error_not_no_inbox() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let data = dir.path().join("data");
    Inbox::open(&data).unwrap();
    std::fs::set_permissions(&data, std::fs::Permissions::from_mode(0o000)).unwrap();
    let looked = Inbox::open_existing(&data);
    std::fs::set_permissions(&data, std::fs::Permissions::from_mode(0o700)).unwrap();
    // As root every file can be looked at; the check means something only for a service user
    if std::fs::read_dir("/root").is_ok() {
        return;
    }
    assert!(looked.is_err(), "taken for no inbox");
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
