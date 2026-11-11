//! Settings › Accounts' vault routes (STEWARD-VAULT.md item 5) on a real pubky-homeserver: the
//! device list, registering the node, "Sign out this device" and "Sign out everywhere".
//!
//! Needs the homeserver's admin API in `PM_BUNDLED_HS_ADMIN` and `PM_BUNDLED_HS_PASSWORD` (CI's
//! `bundle` job); skipped otherwise. A test signer stands in for Pubky Ring.
// The sync future a test spawns (`tokio::spawn`, which needs it Send) nests deeper than the
// default limit of 128 lets the compiler check
#![recursion_limit = "256"]

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use pm_api::sync::PubkySync;
use pm_api::{router, NodeSettings, NodeState};
use pm_appdata::vault::{ClientSecret, SignIn};
use pm_audio::PipelineConfig;
use pm_library::Library;
use pm_pubky::homeserver::NodeHomeserver;
use pm_pubky::vault::Vault;
use pm_zones::OutputRegistry;
use pubky::{Keypair, Pubky, PublicKey};
use serde_json::{json, Value};
use std::sync::Arc;
use tower::ServiceExt;

fn admin() -> Option<(String, String)> {
    Some((
        std::env::var("PM_BUNDLED_HS_ADMIN").ok()?,
        std::env::var("PM_BUNDLED_HS_PASSWORD").ok()?,
    ))
}

async fn admin_call(a: &(String, String), method: reqwest::Method, path: &str, body: Option<Value>) -> Value {
    let mut r = reqwest::Client::new()
        .request(method, format!("{}{path}", a.0))
        .header("X-Admin-Password", &a.1);
    if let Some(b) = body {
        r = r.json(&b);
    }
    let text = r
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .text()
        .await
        .unwrap();
    serde_json::from_str(&text).unwrap_or(Value::String(text))
}

fn testnet() -> bool {
    std::env::var("PM_TESTNET").as_deref() == Ok("1")
}

fn network() -> pm_pubky::config::Network {
    if testnet() {
        pm_pubky::config::Network::Testnet
    } else {
        pm_pubky::config::Network::Mainnet
    }
}

fn pubky() -> Pubky {
    if testnet() {
        Pubky::testnet().unwrap()
    } else {
        Pubky::new().unwrap()
    }
}

fn node(dir: &std::path::Path) -> Arc<NodeState> {
    node_with_token(dir, None)
}

/// [`node`] with an API token, so owner browser sessions and device sign-ins are checked.
fn node_with_token(dir: &std::path::Path, token: Option<&str>) -> Arc<NodeState> {
    let db = dir.join("node.sqlite");
    NodeState::new(
        NodeSettings {
            name: "Living room node".into(),
            version: "0".into(),
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

async fn call(app: &axum::Router, method: &str, uri: &str) -> (StatusCode, Value) {
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method(method)
                .uri(uri)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let st = resp.status();
    let b = resp.into_body().collect().await.unwrap().to_bytes();
    (st, serde_json::from_slice(&b).unwrap_or(Value::Null))
}

async fn post_json(app: &axum::Router, uri: &str, body: Value) -> (StatusCode, Value) {
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(uri)
                .header("content-type", "application/json")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let st = resp.status();
    let b = resp.into_body().collect().await.unwrap().to_bytes();
    (st, serde_json::from_slice(&b).unwrap_or(Value::Null))
}

#[tokio::test]
async fn the_device_list_and_signing_out_through_the_api() {
    let Some(a) = admin() else {
        eprintln!("PM_BUNDLED_HS_ADMIN / PM_BUNDLED_HS_PASSWORD not set: skipped");
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let s = node(dir.path());
    let app = router(s.clone());

    // Not signed in yet
    assert_eq!(call(&app, "GET", "/v1/vault").await.0, StatusCode::CONFLICT);

    // A person, the node signed in (one grant) and their phone (another)
    let info = admin_call(&a, reqwest::Method::GET, "/info", None).await;
    let hs = PublicKey::try_from(info["public_key"].as_str().unwrap()).unwrap();
    let token = admin_call(
        &a,
        reqwest::Method::POST,
        "/generate_signup_token",
        Some(json!({"storage_quota_mb": "unlimited"})),
    )
    .await;
    let signer = pubky().signer(Keypair::random());
    signer.signup(&hs, Some(token.as_str().unwrap())).await.unwrap();
    let homeserver = NodeHomeserver::new(network()).unwrap();
    let mut sessions = Vec::new();
    for id in ["node.pubky.music", "phone.pubky.music"] {
        let flow = homeserver.start_login(id).unwrap();
        signer.approve_auth(flow.url()).await.unwrap();
        sessions.push(flow.await_approval().await.unwrap());
    }
    let phone_session = sessions.pop().unwrap();
    *s.sync.lock().unwrap() = Some(Arc::new(PubkySync::from_session(
        NodeHomeserver::new(network()).unwrap(),
        sessions.pop().unwrap(),
    )));

    // The node registers under its name; its key file is made once, 0600
    let (st, v) = call(&app, "GET", "/v1/vault").await;
    assert_eq!(st, StatusCode::OK, "{v}");
    assert_eq!(v["registered"], false);
    let (st, v) = call(&app, "POST", "/v1/vault/register").await;
    assert_eq!(st, StatusCode::OK, "{v}");
    assert_eq!(v["registered"], true);
    assert_eq!(v["devices"][0]["name"], "Living room node");
    assert_eq!(v["devices"][0]["this_node"], true);
    assert_eq!(v["devices"][0]["device"], "node");
    let node_key = v["this_device"].as_str().unwrap().to_string();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let m = std::fs::metadata(dir.path().join("vault/client.key"))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(m & 0o777, 0o600);
    }

    // The phone joins and signs in to TIDAL for everyone; the node lists both and reads it
    let phone_key = ClientSecret::generate();
    let phone = Vault::new(&phone_session, &phone_key);
    phone.register("Phone").await.unwrap();
    let tidal: SignIn = serde_json::from_value(json!({"service": "tidal", "way": "app",
        "tokens": {"access_token": "never-shown", "refresh_token": "never-shown-either"}}))
    .unwrap();
    phone.write_shared("tidal", &tidal).await.unwrap();
    let first_epoch = phone.envelope("tidal").await.unwrap().unwrap().epoch;
    let (_, v) = call(&app, "GET", "/v1/vault").await;
    assert_eq!(v["devices"].as_array().unwrap().len(), 2, "{v}");
    assert_eq!(v["sign_ins"][0]["service"], "tidal");
    assert_eq!(v["sign_ins"][0]["readable"], true);
    assert!(
        !v.to_string().contains("never-shown"),
        "the API shows a token: {v}"
    );

    // The node can't sign itself out; a bad key is refused
    assert_eq!(
        call(&app, "POST", &format!("/v1/vault/devices/{node_key}/sign-out"))
            .await
            .0,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        call(&app, "POST", "/v1/vault/devices/not-a-key/sign-out").await.0,
        StatusCode::BAD_REQUEST
    );

    // The list as it was before the sign-out, with the phone's valid entry
    let before_sign_out = phone.index().await.unwrap();
    // Sign the phone out: TIDAL rotates past it
    let (st, r) = call(
        &app,
        "POST",
        &format!("/v1/vault/devices/{}/sign-out", phone.client_key().to_hex()),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{r}");
    assert_eq!(r["rotated"], json!(["tidal"]));
    assert_eq!(r["refreshed_at_service"], json!([]));
    assert_eq!(r["failed"], json!([]));
    assert_eq!(r["revoke_in_ring"], true);
    assert!(phone.read("tidal").await.is_err());
    let (_, v) = call(&app, "GET", "/v1/vault").await;
    assert_eq!(v["devices"].as_array().unwrap().len(), 1, "{v}");
    // The phone wrote it and is out, so the node writes past its copy: a later epoch (at the
    // current time at least, so not first_epoch + 1 once a second has passed)
    let epoch = v["sign_ins"][0]["epoch"].as_u64().unwrap();
    assert!(epoch > first_epoch, "{epoch} > {first_epoch}");
    // The node remembers the sign-out: the phone writes the list back without it, and the node
    // still doesn't trust it
    assert!(dir.path().join("vault/revoked.json").exists());
    assert!(before_sign_out.revoked.is_empty());
    phone_session
        .storage()
        .put(
            pm_appdata::vault::INDEX_PATH,
            serde_json::to_vec(&before_sign_out).unwrap(),
        )
        .await
        .unwrap();
    let (_, v) = call(&app, "GET", "/v1/vault").await;
    let listed = v["devices"]
        .as_array()
        .unwrap()
        .iter()
        .find(|d| d["client_key"] == phone.client_key().to_hex())
        .cloned()
        .unwrap_or_else(|| panic!("the phone's entry is back in the list: {v}"));
    assert_eq!(listed["trusted"], false, "{v}");
    // A blob no client wrote doesn't take the device list down
    phone_session
        .storage()
        .put(
            pm_appdata::vault::blob_path("junk").unwrap().as_str(),
            b"junk".to_vec(),
        )
        .await
        .unwrap();
    let (st, v) = call(&app, "GET", "/v1/vault").await;
    assert_eq!(st, StatusCode::OK, "{v}");
    assert!(v["sign_ins"][0]["problem"].is_string(), "{v}");
    assert_eq!(v["sign_ins"][1]["readable"], true, "{v}");

    // A new vault key: only when confirmed; the old file is kept aside and the node registers
    // with the new key
    assert_eq!(
        post_json(&app, "/v1/vault/new-key", json!({})).await.0,
        StatusCode::BAD_REQUEST
    );
    let (st, v) = post_json(&app, "/v1/vault/new-key", json!({"confirm": true})).await;
    assert_eq!(st, StatusCode::OK, "{v}");
    assert_ne!(v["this_device"], json!(node_key));
    assert_eq!(v["registered"], true, "{v}");
    assert!(std::fs::read_dir(dir.path().join("vault")).unwrap().any(|e| e
        .unwrap()
        .file_name()
        .to_string_lossy()
        .starts_with("client.key.replaced-")));

    // Sign out everywhere
    let (st, r) = call(&app, "POST", "/v1/vault/sign-out-everywhere").await;
    assert_eq!(st, StatusCode::OK, "{r}");
    assert_eq!(r["removed"], json!(["junk", "tidal"]));
    assert_eq!(r["left"], json!([]));
    let (_, v) = call(&app, "GET", "/v1/vault").await;
    assert_eq!(v["sign_ins"], json!([]));
}

/// Item 6: two nodes of one person. The first one's sign-ins move into the vault; the second
/// gets them once it's shared; a local refresh on either reaches the other; "Sign out
/// everywhere" moves them aside on both, without deleting anything.
#[tokio::test]
async fn two_nodes_keep_their_sign_ins_in_step_through_the_vault() {
    let Some(a) = admin() else {
        eprintln!("PM_BUNDLED_HS_ADMIN / PM_BUNDLED_HS_PASSWORD not set: skipped");
        return;
    };
    let info = admin_call(&a, reqwest::Method::GET, "/info", None).await;
    let hs = PublicKey::try_from(info["public_key"].as_str().unwrap()).unwrap();
    let token = admin_call(
        &a,
        reqwest::Method::POST,
        "/generate_signup_token",
        Some(json!({"storage_quota_mb": "unlimited"})),
    )
    .await;
    let signer = pubky().signer(Keypair::random());
    signer.signup(&hs, Some(token.as_str().unwrap())).await.unwrap();
    let homeserver = NodeHomeserver::new(network()).unwrap();
    let (d1, d2) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let (n1, n2) = (node(d1.path()), node(d2.path()));
    for (n, id) in [(&n1, "living-room.pubky.music"), (&n2, "kitchen.pubky.music")] {
        let flow = homeserver.start_login(id).unwrap();
        signer.approve_auth(flow.url()).await.unwrap();
        let session = flow.await_approval().await.unwrap();
        *n.sync.lock().unwrap() = Some(Arc::new(PubkySync::from_session(
            NodeHomeserver::new(network()).unwrap(),
            session,
        )));
    }
    let (a1, a2) = (router(n1.clone()), router(n2.clone()));
    let tidal_v1 =
        r#"{"access_token":"at-1","refresh_token":"rt-1","expires_at":1000,"user_id":"7","country":"CH"}"#;
    let set = |n: &NodeState, k: &str, v: &str| n.library.lock().unwrap().set_setting(k, v).unwrap();
    let get = |n: &NodeState, k: &str| n.library.lock().unwrap().get_setting(k).unwrap();
    set(&n1, "tidal.user", tidal_v1);
    set(
        &n1,
        "jamendo.user",
        r#"{"access_token":"jam-1","refresh_token":"jam-r","expires_at":5000,"name":"Gil"}"#,
    );

    // Node 1: its two sign-ins move into the vault
    let (st, r) = call(&a1, "POST", "/v1/vault/sync").await;
    assert_eq!(st, StatusCode::OK, "{r}");
    assert_eq!(r["migrated"], json!(["tidal", "jamendo"]), "{r}");
    // ...and stay where they were on the node
    assert_eq!(get(&n1, "tidal.user").as_deref(), Some(tidal_v1));

    // Node 2 joins: it can't read them until node 1 shares them
    let (_, r) = call(&a2, "POST", "/v1/vault/sync").await;
    assert_eq!(r["not_readable"], json!(["tidal", "jamendo"]), "{r}");
    call(&a1, "POST", "/v1/vault/sync").await;
    let (_, r) = call(&a2, "POST", "/v1/vault/sync").await;
    assert_eq!(r["pulled"], json!(["tidal", "jamendo"]), "{r}");
    assert_eq!(get(&n2, "tidal.user").as_deref(), Some(tidal_v1));
    // What node 2 read was signed by node 1 (Settings says so)
    let (_, v) = call(&a2, "GET", "/v1/vault").await;
    let (_, v1) = call(&a1, "GET", "/v1/vault").await;
    assert_eq!(v["sign_ins"][0]["signed_by"], v1["this_device"], "{v}");
    assert_eq!(
        get(&n2, "jamendo.user").as_deref(),
        Some(r#"{"access_token":"jam-1","refresh_token":"jam-r","expires_at":5000,"name":"Gil"}"#)
    );

    // Node 2's source refreshes TIDAL locally: it reaches node 1
    let tidal_v2 =
        r#"{"access_token":"at-2","refresh_token":"rt-2","expires_at":2000,"user_id":"7","country":"CH"}"#;
    set(&n2, "tidal.user", tidal_v2);
    let (_, r) = call(&a2, "POST", "/v1/vault/sync").await;
    assert_eq!(r["pushed"], json!(["tidal"]), "{r}");
    let (_, r) = call(&a1, "POST", "/v1/vault/sync").await;
    assert_eq!(r["pulled"], json!(["tidal"]), "{r}");
    assert_eq!(get(&n1, "tidal.user").as_deref(), Some(tidal_v2));
    // Node 2 refreshes the old account while node 1 signs in to another one: node 1's wins (the
    // person just acted there), and reaches node 2
    let tidal_v3 =
        r#"{"access_token":"at-3","refresh_token":"rt-3","expires_at":3000,"user_id":"7","country":"CH"}"#;
    set(&n2, "tidal.user", tidal_v3);
    let (_, r) = call(&a2, "POST", "/v1/vault/sync").await;
    assert_eq!(r["pushed"], json!(["tidal"]), "{r}");
    let other = r#"{"access_token":"other-1","refresh_token":"other-r","expires_at":2500,"user_id":"9","country":"DE"}"#;
    set(&n1, "tidal.user", other);
    // (a new sign-in, as the source's own sign-in flow marks it)
    pm_api::vault_cache::new_sign_in(&n1.library.lock().unwrap(), "tidal");
    let (_, r) = call(&a1, "POST", "/v1/vault/sync").await;
    assert_eq!(
        r["pushed"],
        json!(["tidal"]),
        "node 1's new account goes out: {r}"
    );
    assert_eq!(get(&n1, "tidal.user").as_deref(), Some(other));
    let (_, r) = call(&a2, "POST", "/v1/vault/sync").await;
    assert_eq!(r["pulled"], json!(["tidal"]), "{r}");
    assert_eq!(get(&n2, "tidal.user").as_deref(), Some(other));
    let tidal_v2 = other;
    // Settings sees the last sync
    let (_, v) = call(&a2, "GET", "/v1/vault").await;
    assert_eq!(v["last_sync"]["report"]["pulled"], json!(["tidal"]), "{v}");
    assert!(v["last_sync"]["at"].as_i64().is_some());
    // Nothing changed: nothing moves
    let (_, r) = call(&a1, "POST", "/v1/vault/sync").await;
    let quiet = json!({"migrated": [], "pushed": [], "signed": [], "pulled": [], "signed_out": [], "removed": [],
        "conflicts": [], "not_usable": [], "not_readable": [], "failed": [], "forgotten": [], "details": []});
    assert_eq!(r, quiet);

    // A node restored from a backup made without a passphrase would have the sync records and
    // not the sign-ins. Its records are another machine's (another vault key), so it takes the
    // vault's copies instead of signing the person out everywhere
    let d3 = tempfile::tempdir().unwrap();
    let n3 = node(d3.path());
    {
        let flow = homeserver.start_login("restored.pubky.music").unwrap();
        signer.approve_auth(flow.url()).await.unwrap();
        let session = flow.await_approval().await.unwrap();
        *n3.sync.lock().unwrap() = Some(Arc::new(PubkySync::from_session(
            NodeHomeserver::new(network()).unwrap(),
            session,
        )));
    }
    let a3 = router(n3.clone());
    for k in ["vault.synced.tidal", "vault.synced.jamendo"] {
        let v = get(&n1, k).expect("node 1 has a record");
        set(&n3, k, &v);
    }
    call(&a3, "POST", "/v1/vault/register").await;
    call(&a1, "POST", "/v1/vault/register").await;
    let (_, r) = call(&a3, "POST", "/v1/vault/sync").await;
    assert_eq!(r["pulled"], json!(["tidal", "jamendo"]), "{r}");
    assert_eq!(r["removed"], json!([]), "{r}");
    let (_, r) = call(&a1, "POST", "/v1/vault/sync").await;
    assert_eq!(r, quiet, "node 1 keeps its sign-ins");

    // Signing out of Jamendo on node 2 removes it from the vault, and node 1 follows; it doesn't
    // come back on node 2's next pass
    n2.library.lock().unwrap().delete_setting("jamendo.user").unwrap();
    let (_, r) = call(&a2, "POST", "/v1/vault/sync").await;
    assert_eq!(r["removed"], json!(["jamendo"]), "{r}");
    let (_, r) = call(&a1, "POST", "/v1/vault/sync").await;
    assert_eq!(r["signed_out"], json!(["jamendo"]), "{r}");
    assert_eq!(get(&n1, "jamendo.user"), None);
    let (_, r) = call(&a2, "POST", "/v1/vault/sync").await;
    assert_eq!(r, quiet);

    // Two nodes that each had their own ListenBrainz before: neither replaces the other
    let (lb1, lb2) = (r#"{"token":"lb-1","user":"a"}"#, r#"{"token":"lb-2","user":"b"}"#);
    set(&n1, "listenbrainz.account", lb1);
    set(&n2, "listenbrainz.account", lb2);
    let (_, r) = call(&a1, "POST", "/v1/vault/sync").await;
    assert_eq!(r["migrated"], json!(["listenbrainz"]), "{r}");
    call(&a1, "POST", "/v1/vault/register").await;
    let (_, r) = call(&a2, "POST", "/v1/vault/sync").await;
    assert_eq!(r["conflicts"], json!(["listenbrainz"]), "{r}");
    // Settings gets why, each side's account (never a token), and what can be taken
    assert_eq!(r["details"][0]["service"], "listenbrainz", "{r}");
    assert_eq!(r["details"][0]["reason"], "other_account");
    assert_eq!(r["details"][0]["node"]["account"], "b");
    assert_eq!(r["details"][0]["vault"]["account"], "a");
    assert_eq!(r["details"][0]["vault"]["written_by"], "node");
    assert_eq!(r["details"][0]["can_take"], json!(["node", "vault"]));
    assert!(
        !r.to_string().contains("lb-1") && !r.to_string().contains("lb-2"),
        "a token in {r}"
    );
    // A take that can't apply is a 409 with the reason; a service the node doesn't keep is a 400
    let (st, v) = post_json(&a2, "/v1/vault/sync", json!({"service": "tidal", "take": "node"})).await;
    assert_eq!(st, StatusCode::CONFLICT, "{v}");
    let (st, _) = post_json(&a2, "/v1/vault/sync", json!({"service": "plex", "take": "node"})).await;
    assert_eq!(st, StatusCode::BAD_REQUEST);
    assert_eq!(get(&n2, "listenbrainz.account").as_deref(), Some(lb2));
    let (_, r) = call(&a1, "POST", "/v1/vault/sync").await;
    assert_eq!(r, quiet, "the vault keeps node 1's");

    // Sign out everywhere from node 1: aside on node 1 now, on node 2 at its next sync (its
    // conflicting copy too)
    // A Spotify sign-in made on node 1 a moment before, not synced yet, goes aside too: it is
    // never pushed back into the wiped vault
    set(
        &n1,
        "spotify.user",
        r#"{"access_token":"sp-1","refresh_token":"sp-r","expires_at":9000,"user_id":"gil"}"#,
    );
    pm_api::vault_cache::new_sign_in(&n1.library.lock().unwrap(), "spotify");
    let (st, r) = call(&a1, "POST", "/v1/vault/sign-out-everywhere").await;
    assert_eq!(st, StatusCode::OK, "{r}");
    assert_eq!(r["removed"], json!(["listenbrainz", "tidal"]));
    assert_eq!(r["left"], json!([]));
    assert_eq!(r["signed_out_here"], json!(["tidal", "spotify", "listenbrainz"]));
    assert_eq!(get(&n1, "spotify.user"), None);
    let (_, r) = call(&a2, "POST", "/v1/vault/sync").await;
    assert_eq!(r["signed_out"], json!(["tidal", "listenbrainz"]), "{r}");
    for n in [&n1, &n2] {
        assert_eq!(get(n, "tidal.user"), None, "the node no longer uses it");
        assert_eq!(get(n, "listenbrainz.account"), None);
        assert_eq!(
            get(n, "vault.signed_out.tidal.user").as_deref(),
            Some(tidal_v2),
            "kept aside"
        );
    }
    // ...and nothing comes back on the next pass
    for a in [&a1, &a2] {
        let (_, r) = call(a, "POST", "/v1/vault/sync").await;
        assert_eq!(r, quiet);
    }
}

/// A node's key and session, to look at (and tamper with) the vault as a test.
fn node_vault_parts(n: &NodeState) -> (Arc<PubkySync>, ClientSecret) {
    let sync = n.sync.lock().unwrap().clone().expect("signed in");
    let key = pm_pubky::vault::load_or_create_client_secret(&n.data_dir().join("vault").join("client.key"))
        .unwrap();
    (sync, key)
}

/// A copy in the vault that nobody the person trusts signed is written over by a node's own
/// copy at its next sync (sync goes on); and a copy a node took from an unsigned one (the
/// migration window) is never signed by that node as its own.
#[tokio::test]
async fn an_untrusted_copy_is_written_over_and_an_unsigned_one_taken_is_not_signed() {
    use pm_appdata::vault::{blob_path, seal, sign_envelope, DataKey, SigningKey, INDEX_PATH};
    let Some(a) = admin() else {
        eprintln!("PM_BUNDLED_HS_ADMIN / PM_BUNDLED_HS_PASSWORD not set: skipped");
        return;
    };
    let info = admin_call(&a, reqwest::Method::GET, "/info", None).await;
    let hs = PublicKey::try_from(info["public_key"].as_str().unwrap()).unwrap();
    let token = admin_call(
        &a,
        reqwest::Method::POST,
        "/generate_signup_token",
        Some(json!({"storage_quota_mb": "unlimited"})),
    )
    .await;
    let signer = pubky().signer(Keypair::random());
    signer.signup(&hs, Some(token.as_str().unwrap())).await.unwrap();
    let homeserver = NodeHomeserver::new(network()).unwrap();
    let (d1, d2, d3) = (
        tempfile::tempdir().unwrap(),
        tempfile::tempdir().unwrap(),
        tempfile::tempdir().unwrap(),
    );
    let (n1, n2, n3) = (node(d1.path()), node(d2.path()), node(d3.path()));
    for (n, id) in [
        (&n1, "living-room.pubky.music"),
        (&n2, "kitchen.pubky.music"),
        (&n3, "bedroom.pubky.music"),
    ] {
        let flow = homeserver.start_login(id).unwrap();
        signer.approve_auth(flow.url()).await.unwrap();
        let session = flow.await_approval().await.unwrap();
        *n.sync.lock().unwrap() = Some(Arc::new(PubkySync::from_session(
            NodeHomeserver::new(network()).unwrap(),
            session,
        )));
    }
    let (a1, a2, a3) = (router(n1.clone()), router(n2.clone()), router(n3.clone()));
    let set = |n: &NodeState, k: &str, v: &str| n.library.lock().unwrap().set_setting(k, v).unwrap();
    set(
        &n1,
        "tidal.user",
        r#"{"access_token":"at-1","refresh_token":"rt-1","expires_at":1000,"user_id":"7","country":"CH"}"#,
    );
    call(&a2, "POST", "/v1/vault/register").await;
    let (_, r) = call(&a1, "POST", "/v1/vault/sync").await;
    assert_eq!(r["migrated"], json!(["tidal"]), "{r}");
    let (_, r) = call(&a2, "POST", "/v1/vault/sync").await;
    assert_eq!(r["pulled"], json!(["tidal"]), "{r}");
    let (s1, k1) = node_vault_parts(&n1);
    let (_, k2) = node_vault_parts(&n2);
    let v1 = Vault::new(s1.session(), &k1);
    let owner = v1.owner().to_string();

    // A stranger's copy of the same content over node 1's (a writer that is no device of the
    // person's, or one whose grant ended since): node 1 writes its own over it
    let path = blob_path("tidal").unwrap();
    let mine = v1.read("tidal").await.unwrap().unwrap();
    let before = v1.envelope("tidal").await.unwrap().unwrap();
    let stranger = ClientSecret::generate();
    let mut forged = seal(
        &owner,
        &path,
        mine.epoch + 1,
        &DataKey::generate(),
        &serde_json::to_vec(&mine.sign_in).unwrap(),
        &[k1.public(), k2.public()],
    )
    .unwrap();
    sign_envelope(
        &owner,
        &path,
        &mut forged,
        stranger.public(),
        &SigningKey::derive(&stranger),
    )
    .unwrap();
    s1.session()
        .storage()
        .put(path.as_str(), serde_json::to_vec(&forged).unwrap())
        .await
        .unwrap();
    let (_, r) = call(&a1, "POST", "/v1/vault/sync").await;
    assert_eq!(
        r["pushed"],
        json!(["tidal"]),
        "node 1's own copy goes over it: {r}"
    );
    assert_eq!(r["not_usable"], json!([]), "{r}");
    let back = v1.envelope("tidal").await.unwrap().unwrap();
    assert_eq!(back.writer.map(|w| w.client_key), Some(k1.public()));
    let (_, r) = call(&a1, "POST", "/v1/vault/sync").await;
    assert_eq!(r["pushed"], json!([]), "once: {r}");
    // An older copy put back (the one from before node 1's write) is written over as well, and
    // not reported as failed on every pass
    s1.session()
        .storage()
        .put(path.as_str(), serde_json::to_vec(&before).unwrap())
        .await
        .unwrap();
    let (_, r) = call(&a1, "POST", "/v1/vault/sync").await;
    assert_eq!(
        (r["pushed"].clone(), r["failed"].clone()),
        (json!(["tidal"]), json!([])),
        "{r}"
    );
    assert!(v1.read("tidal").await.unwrap().unwrap().epoch > before.epoch);

    // The migration window: node 1's entry from before v2 (no signer proof), an unsigned copy
    // of Jamendo in the vault, node 3 with none of its own and no record of node 1 ever signing:
    // node 3 takes it, and never signs it. Node 2 has seen node 1's proof verify, so for it the
    // entry without its proof is still a client that signs: it refuses the unsigned copy
    call(&a3, "POST", "/v1/vault/register").await;
    let (_, k3) = node_vault_parts(&n3);
    set(
        &n1,
        "jamendo.user",
        r#"{"access_token":"jam-1","refresh_token":"jam-r","expires_at":5000,"name":"Gil"}"#,
    );
    let (_, r) = call(&a1, "POST", "/v1/vault/sync").await;
    assert_eq!(r["migrated"], json!(["jamendo"]), "{r}");
    let j = v1.read("jamendo").await.unwrap().unwrap();
    let ix_full = v1.index().await.unwrap();
    let mut ix_old = ix_full.clone();
    for e in ix_old.clients.iter_mut().filter(|e| e.client_key == k1.public()) {
        e.signer = None;
    }
    s1.session()
        .storage()
        .put(INDEX_PATH, serde_json::to_vec(&ix_old).unwrap())
        .await
        .unwrap();
    // ...rewritten unsigned, as an older client would have written it
    let jpath = blob_path("jamendo").unwrap();
    let unsigned = seal(
        &owner,
        &jpath,
        j.epoch,
        &DataKey::generate(),
        &serde_json::to_vec(&j.sign_in).unwrap(),
        &[k1.public(), k2.public(), k3.public()],
    )
    .unwrap();
    s1.session()
        .storage()
        .put(jpath.as_str(), serde_json::to_vec(&unsigned).unwrap())
        .await
        .unwrap();
    let (_, r) = call(&a2, "POST", "/v1/vault/sync").await;
    assert!(
        r["pulled"] == json!([]) && r["not_usable"].as_array().unwrap().contains(&json!("jamendo")),
        "a proof dropped from the list doesn't reopen unsigned reads: {r}"
    );
    // (node 2's pass wrote its own signed TIDAL over the copy whose writer lost its proof, so
    // node 3 takes that one as well)
    let (_, r) = call(&a3, "POST", "/v1/vault/sync").await;
    assert!(r["pulled"].as_array().unwrap().contains(&json!("jamendo")), "{r}");
    let (_, r) = call(&a3, "POST", "/v1/vault/sync").await;
    assert_eq!(
        r["signed"],
        json!([]),
        "node 3 doesn't vouch for what it took unsigned: {r}"
    );
    assert!(v1.envelope("jamendo").await.unwrap().unwrap().writer.is_none());

    // ...nor does it write it over a copy nobody trusted signed: a stranger's copy of Jamendo
    // stays, since node 3's own copy came from the unsigned one (a node that held its own
    // pushes over such a copy; node 3 may not)
    let not_pushed = |r: &serde_json::Value| {
        r["pushed"] == json!([])
            && r["signed"] == json!([])
            && r["not_usable"].as_array().unwrap().contains(&json!("jamendo"))
    };
    let mut by_stranger = seal(
        &owner,
        &jpath,
        j.epoch + 1,
        &DataKey::generate(),
        &serde_json::to_vec(&j.sign_in).unwrap(),
        &[k1.public(), k2.public(), k3.public()],
    )
    .unwrap();
    sign_envelope(
        &owner,
        &jpath,
        &mut by_stranger,
        stranger.public(),
        &SigningKey::derive(&stranger),
    )
    .unwrap();
    s1.session()
        .storage()
        .put(jpath.as_str(), serde_json::to_vec(&by_stranger).unwrap())
        .await
        .unwrap();
    let (_, r) = call(&a3, "POST", "/v1/vault/sync").await;
    assert!(
        not_pushed(&r),
        "a copy taken unsigned isn't written over a stranger's: {r}"
    );
    assert_eq!(
        v1.envelope("jamendo")
            .await
            .unwrap()
            .unwrap()
            .writer
            .map(|w| w.client_key),
        Some(stranger.public()),
        "the stranger's copy is still there"
    );
    // Node 1 (whose copy is its own) writes over it once its proof is back in the list; node 3
    // finds the same content signed, keeps its record as taken unsigned, and signs nothing
    s1.session()
        .storage()
        .put(INDEX_PATH, serde_json::to_vec(&ix_full).unwrap())
        .await
        .unwrap();
    let (_, r) = call(&a1, "POST", "/v1/vault/sync").await;
    assert_eq!(r["pushed"], json!(["jamendo"]), "{r}");
    let signed_by_1 = v1.envelope("jamendo").await.unwrap().unwrap();
    assert_eq!(
        signed_by_1.writer.as_ref().map(|w| w.client_key),
        Some(k1.public())
    );
    let (_, r) = call(&a3, "POST", "/v1/vault/sync").await;
    assert_eq!(
        (r["signed"].clone(), r["pushed"].clone()),
        (json!([]), json!([])),
        "{r}"
    );
    // An older copy put back is stale for node 3 now (it has seen the signed one): still not
    // written over by node 3, and node 1 does it
    s1.session()
        .storage()
        .put(jpath.as_str(), serde_json::to_vec(&unsigned).unwrap())
        .await
        .unwrap();
    let (_, r) = call(&a3, "POST", "/v1/vault/sync").await;
    assert!(
        not_pushed(&r),
        "a copy taken unsigned isn't written over a stale one: {r}"
    );
    assert!(v1.envelope("jamendo").await.unwrap().unwrap().writer.is_none());
    let (_, r) = call(&a1, "POST", "/v1/vault/sync").await;
    assert_eq!(r["pushed"], json!(["jamendo"]), "{r}");
}

/// Taking the vault's copy by hand lets a copy older than one this node has seen in as new. The
/// node's record of the newest copy seen goes only for that take, and comes back when the take
/// doesn't go through: a copy refused for another reason (here: made before the person signed
/// out everywhere) is refused as stale again at the next pass, not taken later as if it were new.
#[tokio::test]
async fn a_take_of_the_vaults_copy_that_doesnt_go_through_keeps_the_nodes_record() {
    use pm_appdata::vault::{blob_path, INDEX_PATH};
    let Some(a) = admin() else {
        eprintln!("PM_BUNDLED_HS_ADMIN / PM_BUNDLED_HS_PASSWORD not set: skipped");
        return;
    };
    let info = admin_call(&a, reqwest::Method::GET, "/info", None).await;
    let hs = PublicKey::try_from(info["public_key"].as_str().unwrap()).unwrap();
    let token = admin_call(
        &a,
        reqwest::Method::POST,
        "/generate_signup_token",
        Some(json!({"storage_quota_mb": "unlimited"})),
    )
    .await;
    let signer = pubky().signer(Keypair::random());
    signer.signup(&hs, Some(token.as_str().unwrap())).await.unwrap();
    let homeserver = NodeHomeserver::new(network()).unwrap();
    let d1 = tempfile::tempdir().unwrap();
    let n1 = node(d1.path());
    {
        let flow = homeserver.start_login("living-room.pubky.music").unwrap();
        signer.approve_auth(flow.url()).await.unwrap();
        let session = flow.await_approval().await.unwrap();
        *n1.sync.lock().unwrap() = Some(Arc::new(PubkySync::from_session(
            NodeHomeserver::new(network()).unwrap(),
            session,
        )));
    }
    let a1 = router(n1.clone());
    let set = |n: &NodeState, k: &str, v: &str| n.library.lock().unwrap().set_setting(k, v).unwrap();
    let get = |n: &NodeState, k: &str| n.library.lock().unwrap().get_setting(k).unwrap();
    set(
        &n1,
        "tidal.user",
        r#"{"access_token":"at-1","refresh_token":"rt-1","expires_at":1000,"user_id":"7","country":"CH"}"#,
    );
    let (_, r) = call(&a1, "POST", "/v1/vault/sync").await;
    assert_eq!(r["migrated"], json!(["tidal"]), "{r}");
    let (s1, k1) = node_vault_parts(&n1);
    let v1 = Vault::new(s1.session(), &k1);
    let first = v1.envelope("tidal").await.unwrap().unwrap();
    // The sign-in moves on, then the node signs out of it: the vault's copy goes, the node
    // remembers the newest copy it saw
    set(
        &n1,
        "tidal.user",
        r#"{"access_token":"at-2","refresh_token":"rt-2","expires_at":2000,"user_id":"7","country":"CH"}"#,
    );
    let (_, r) = call(&a1, "POST", "/v1/vault/sync").await;
    assert_eq!(r["pushed"], json!(["tidal"]), "{r}");
    n1.library.lock().unwrap().delete_setting("tidal.user").unwrap();
    let (_, r) = call(&a1, "POST", "/v1/vault/sync").await;
    assert_eq!(r["removed"], json!(["tidal"]), "{r}");
    // The first copy put back: older than what the node saw, there but not taken
    let path = blob_path("tidal").unwrap();
    s1.session()
        .storage()
        .put(path.as_str(), serde_json::to_vec(&first).unwrap())
        .await
        .unwrap();
    let (_, r) = call(&a1, "POST", "/v1/vault/sync").await;
    assert_eq!(r["not_usable"], json!(["tidal"]), "{r}");
    assert_eq!(r["details"][0]["reason"], "stale", "{r}");
    // The person signed out everywhere since that copy was made (the device list says so)
    let mut ix = v1.index().await.unwrap();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    ix.extra
        .insert(pm_pubky::vault::SIGNED_OUT_EVERYWHERE.into(), json!(now));
    s1.session()
        .storage()
        .put(INDEX_PATH, serde_json::to_vec(&ix).unwrap())
        .await
        .unwrap();
    // Taking the vault's copy by hand: it would be let in past the record, but it is from before
    // the sign-out everywhere, so the take is refused...
    let (st, r) = post_json(
        &a1,
        "/v1/vault/sync",
        json!({"service": "tidal", "take": "vault"}),
    )
    .await;
    assert_eq!(st, StatusCode::CONFLICT, "{r}");
    assert!(
        r["error"]
            .as_str()
            .unwrap_or_default()
            .contains("signed out everywhere"),
        "{r}"
    );
    assert_eq!(get(&n1, "tidal.user"), None);
    // ...and the record stays: the next pass refuses the old copy as stale again
    let (_, r) = call(&a1, "POST", "/v1/vault/sync").await;
    assert_eq!(r["not_usable"], json!(["tidal"]), "{r}");
    assert_eq!(
        r["details"][0]["reason"], "stale",
        "the record of the newest copy seen is back: {r}"
    );
}

/// A sign-in that replaces this node's while a vault pass runs for the old one ends the pass
/// before its next write: nothing of the old identity's sign-ins lands on the node, and the old
/// identity's vault (its device list, its sign-ins) is left as it was.
#[tokio::test]
async fn a_sign_in_switch_ends_the_old_vault_pass() {
    use pm_pubky::vault::VaultStoreError;
    let Some(a) = admin() else {
        eprintln!("PM_BUNDLED_HS_ADMIN / PM_BUNDLED_HS_PASSWORD not set: skipped");
        return;
    };
    let info = admin_call(&a, reqwest::Method::GET, "/info", None).await;
    let hs = PublicKey::try_from(info["public_key"].as_str().unwrap()).unwrap();
    let homeserver = NodeHomeserver::new(network()).unwrap();
    let sign_up = |ids: Vec<&'static str>| {
        let (a, hs, homeserver) = (a.clone(), hs.clone(), homeserver.clone());
        async move {
            let token = admin_call(
                &a,
                reqwest::Method::POST,
                "/generate_signup_token",
                Some(json!({"storage_quota_mb": "unlimited"})),
            )
            .await;
            let signer = pubky().signer(Keypair::random());
            signer.signup(&hs, Some(token.as_str().unwrap())).await.unwrap();
            let mut sessions = Vec::new();
            for id in ids {
                let flow = homeserver.start_login(id).unwrap();
                signer.approve_auth(flow.url()).await.unwrap();
                sessions.push(flow.await_approval().await.unwrap());
            }
            sessions
        }
    };
    let mut a_sessions = sign_up(vec!["node.pubky.music", "phone.pubky.music"]).await;
    let mut b_sessions = sign_up(vec!["node.pubky.music"]).await;
    let phone_session = a_sessions.pop().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let n = node(dir.path());
    n.set_sync(Some(Arc::new(PubkySync::from_session(
        NodeHomeserver::new(network()).unwrap(),
        a_sessions.pop().unwrap(),
    ))));
    let app = router(n.clone());
    let (st, v) = call(&app, "POST", "/v1/vault/register").await;
    assert_eq!(st, StatusCode::OK, "{v}");
    // A's phone signed in to TIDAL for every device of A's
    let phone_key = ClientSecret::generate();
    let phone = Vault::new(&phone_session, &phone_key);
    phone.register("Phone").await.unwrap();
    let tidal: SignIn = serde_json::from_value(json!({"service": "tidal", "way": "app",
        "tokens": {"access_token": "never-shown", "refresh_token": "never-shown-either"}}))
    .unwrap();
    phone.write_shared("tidal", &tidal).await.unwrap();
    let index_before = serde_json::to_value(phone.index().await.unwrap()).unwrap();
    let blob_before = phone.envelope("tidal").await.unwrap().unwrap();
    // The node's pass for A starts as B signs in on the node
    let sync_a = n.sync.lock().unwrap().clone().expect("signed in as A");
    let key = pm_pubky::vault::load_or_create_client_secret(&n.data_dir().join("vault").join("client.key"))
        .unwrap();
    n.set_sync(Some(Arc::new(PubkySync::from_session(
        NodeHomeserver::new(network()).unwrap(),
        b_sessions.pop().unwrap(),
    ))));
    assert!(sync_a.is_retired());
    let vault = Vault::new(sync_a.session(), &key).device("node");
    let r = pm_api::vault_cache::pass(&n, &vault, None, &|| !sync_a.is_retired()).await;
    assert!(matches!(r, Err(VaultStoreError::Replaced)), "{r:?}");
    let get = |k: &str| n.library.lock().unwrap().get_setting(k).unwrap();
    assert_eq!(get("tidal.user"), None, "A's sign-in landed on the node");
    assert_eq!(
        serde_json::to_value(phone.index().await.unwrap()).unwrap(),
        index_before,
        "A's device list was written"
    );
    assert_eq!(phone.envelope("tidal").await.unwrap().unwrap(), blob_before);
    // B's own pass runs: B has no sign-ins, nothing to take; the node is registered with B
    let (st, v) = call(&app, "POST", "/v1/vault/register").await;
    assert_eq!(st, StatusCode::OK, "{v}");
    let (st, r) = call(&app, "POST", "/v1/vault/sync").await;
    assert_eq!(st, StatusCode::OK, "{r}");
    assert_eq!(r["pulled"], json!([]), "{r}");
    assert_eq!(get("tidal.user"), None);
}

/// Identities on the test homeserver, each with sessions for `ids` (client ids).
async fn identities_with_sessions(n: usize, ids: &[&str]) -> Option<Vec<Vec<pubky::PubkySession>>> {
    let a = admin()?;
    let info = admin_call(&a, reqwest::Method::GET, "/info", None).await;
    let hs = PublicKey::try_from(info["public_key"].as_str().unwrap()).unwrap();
    let homeserver = NodeHomeserver::new(network()).unwrap();
    let mut out = Vec::new();
    for _ in 0..n {
        let token = admin_call(
            &a,
            reqwest::Method::POST,
            "/generate_signup_token",
            Some(json!({"storage_quota_mb": "unlimited"})),
        )
        .await;
        let signer = pubky().signer(Keypair::random());
        signer.signup(&hs, Some(token.as_str().unwrap())).await.unwrap();
        let mut sessions = Vec::new();
        for id in ids {
            let flow = homeserver.start_login(id).unwrap();
            signer.approve_auth(flow.url()).await.unwrap();
            sessions.push(flow.await_approval().await.unwrap());
        }
        out.push(sessions);
    }
    Some(out)
}

fn bookmark(n: &NodeState, i: u32) -> String {
    let uri = pm_model::MusicUri::parse(&format!(
        "https://musicbrainz.org/recording/00000000-0000-4000-8000-{i:012}"
    ))
    .unwrap();
    n.library
        .lock()
        .unwrap()
        .add_bookmark(
            &pm_pubky::Bookmark::id_for(&uri),
            &uri.to_url(),
            1_700_000_000_000_000,
        )
        .unwrap();
    uri.to_url()
}

/// A sign-in that replaces the node's in the middle of a sync pass (after its reads, before its
/// writes: the test stops the pass there) ends the pass at its next write: nothing of the old
/// identity's lands in the library and nothing of the node's goes to the old identity's
/// homeserver.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_sign_in_switch_mid_sync_writes_nothing_more() {
    use pm_api::sync::SyncError;
    let Some(mut ids) = identities_with_sessions(2, &["node.pubky.music", "phone.pubky.music"]).await else {
        eprintln!("PM_BUNDLED_HS_ADMIN / PM_BUNDLED_HS_PASSWORD not set: skipped");
        return;
    };
    let mut b = ids.pop().unwrap();
    let mut a = ids.pop().unwrap();
    let (a_phone, a_node) = (a.pop().unwrap(), a.pop().unwrap());
    let homeserver = NodeHomeserver::new(network()).unwrap();
    // A's other device put a bookmark on A's homeserver
    let d_other = tempfile::tempdir().unwrap();
    let other = node(d_other.path());
    let theirs = bookmark(&other, 1);
    let sync_other = Arc::new(PubkySync::from_session(homeserver.clone(), a_phone));
    other.set_sync(Some(sync_other.clone()));
    assert_eq!(sync_other.sync(&other.library).await.unwrap().pushed, 1);
    // The node, signed in as A with a bookmark queued, starts a pass; it stops after its reads
    let d = tempfile::tempdir().unwrap();
    let n = node(d.path());
    let mine = bookmark(&n, 2);
    let sync_a = Arc::new(PubkySync::from_session(homeserver.clone(), a_node.clone()));
    n.set_sync(Some(sync_a.clone()));
    let (reached, release) = pm_api::test_hooks::arm(&format!("sync.after_reads:{}", sync_a.z32()));
    let pass = {
        let (sync_a, library) = (sync_a.clone(), n.library.clone());
        tokio::spawn(async move { sync_a.sync(&library).await })
    };
    reached.await.unwrap();
    // B signs in now, mid-pass
    n.set_sync(Some(Arc::new(PubkySync::from_session(
        homeserver.clone(),
        b.pop().unwrap(),
    ))));
    let _ = release.send(());
    let r = pass.await.unwrap();
    assert!(matches!(r, Err(SyncError::Replaced)), "{r:?}");
    let urls: Vec<String> = n
        .library
        .lock()
        .unwrap()
        .list_bookmarks()
        .unwrap()
        .into_iter()
        .map(|b| b.uri)
        .collect();
    assert_eq!(urls, [mine], "A's bookmark landed on the node");
    let on_a: Vec<String> = homeserver
        .read_v1_bookmarks_except(&a_node, &std::collections::HashMap::new())
        .await
        .unwrap()
        .into_iter()
        .map(|(_, url, _)| url)
        .collect();
    assert_eq!(on_a, [theirs], "the node's bookmark went to A's homeserver");
}

/// The same for a vault pass, on a sign-in a node can take: another node of A's put its TIDAL
/// sign-in in A's vault; a node of A's that runs its pass undisturbed takes it (the control), while
/// the node whose pass is stopped right after it read the sign-in, and replaced by B's sign-in
/// there, takes nothing and writes nothing to A's vault.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_sign_in_switch_mid_vault_pass_writes_nothing_more() {
    use pm_pubky::vault::VaultStoreError;
    let Some(mut ids) = identities_with_sessions(
        2,
        &["node.pubky.music", "kitchen.pubky.music", "bedroom.pubky.music"],
    )
    .await
    else {
        eprintln!("PM_BUNDLED_HS_ADMIN / PM_BUNDLED_HS_PASSWORD not set: skipped");
        return;
    };
    let mut b = ids.pop().unwrap();
    let mut a = ids.pop().unwrap();
    let (a_control, a_writer, a_node) = (a.pop().unwrap(), a.pop().unwrap(), a.pop().unwrap());
    let make = |session: pubky::PubkySession| {
        let d = tempfile::tempdir().unwrap();
        let n = node(d.path());
        n.set_sync(Some(Arc::new(PubkySync::from_session(
            NodeHomeserver::new(network()).unwrap(),
            session,
        ))));
        (d, n)
    };
    let (_dn, n) = make(a_node);
    let (_dw, writer) = make(a_writer);
    let (_dc, control) = make(a_control);
    // The node and the control join A's vault first, so the sign-in is wrapped to them
    for x in [&n, &control] {
        let (st, v) = call(&router(x.clone()), "POST", "/v1/vault/register").await;
        assert_eq!(st, StatusCode::OK, "{v}");
    }
    let tidal =
        r#"{"access_token":"at-1","refresh_token":"rt-1","expires_at":1000,"user_id":"7","country":"CH"}"#;
    writer
        .library
        .lock()
        .unwrap()
        .set_setting("tidal.user", tidal)
        .unwrap();
    let (_, r) = call(&router(writer.clone()), "POST", "/v1/vault/sync").await;
    assert_eq!(r["migrated"], json!(["tidal"]), "{r}");
    // Undisturbed, a node of A's takes it
    let (_, r) = call(&router(control.clone()), "POST", "/v1/vault/sync").await;
    assert_eq!(r["pulled"], json!(["tidal"]), "the control takes it: {r}");
    let (s1, k1) = node_vault_parts(&writer);
    let v_writer = Vault::new(s1.session(), &k1);
    let index_before = serde_json::to_value(v_writer.index().await.unwrap()).unwrap();
    let blob_before = v_writer.envelope("tidal").await.unwrap().unwrap();
    // The node's pass, stopped after its read of TIDAL; B signs in there
    let (sync_a, key) = node_vault_parts(&n);
    let vault = Vault::new(sync_a.session(), &key).device("node");
    let (reached, release) = pm_api::test_hooks::arm(&format!("vault.after_read:{}:tidal", vault.owner()));
    let pass = {
        let (n, sync_a) = (n.clone(), sync_a.clone());
        tokio::spawn(async move {
            let vault = Vault::new(sync_a.session(), &key).device("node");
            pm_api::vault_cache::pass(&n, &vault, None, &|| !sync_a.is_retired()).await
        })
    };
    tokio::time::timeout(std::time::Duration::from_secs(60), reached)
        .await
        .expect("the pass reached its pause")
        .unwrap();
    n.set_sync(Some(Arc::new(PubkySync::from_session(
        NodeHomeserver::new(network()).unwrap(),
        b.pop().unwrap(),
    ))));
    let _ = release.send(());
    let r = pass.await.unwrap();
    assert!(matches!(r, Err(VaultStoreError::Replaced)), "{r:?}");
    let get = |k: &str| n.library.lock().unwrap().get_setting(k).unwrap();
    assert_eq!(get("tidal.user"), None, "A's sign-in landed on the node");
    assert_eq!(
        serde_json::to_value(v_writer.index().await.unwrap()).unwrap(),
        index_before
    );
    assert_eq!(v_writer.envelope("tidal").await.unwrap().unwrap(), blob_before);
}

/// A replaced first sync leaves the new sign-in's activity alone: whatever it set running is reset
/// when the session is replaced (here while the first sync is stopped mid-pass), it never writes
/// the activity afterwards, and the node's storage save, which skips while a sync runs, saves.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_replaced_first_sync_leaves_no_sync_running() {
    let Some(mut ids) = identities_with_sessions(2, &["node.pubky.music"]).await else {
        eprintln!("PM_BUNDLED_HS_ADMIN / PM_BUNDLED_HS_PASSWORD not set: skipped");
        return;
    };
    let mut b = ids.pop().unwrap();
    let mut a = ids.pop().unwrap();
    let homeserver = NodeHomeserver::new(network()).unwrap();
    let d = tempfile::tempdir().unwrap();
    let n = node(d.path());
    let sync_a = Arc::new(PubkySync::from_session(homeserver.clone(), a.pop().unwrap()));
    n.set_sync(Some(sync_a.clone()));
    let (reached, release) = pm_api::test_hooks::arm(&format!("sync.after_reads:{}", sync_a.z32()));
    pm_api::sync::first_sync_in_background(&n, sync_a.clone());
    tokio::time::timeout(std::time::Duration::from_secs(60), reached)
        .await
        .expect("the first sync reached its pause")
        .unwrap();
    assert!(n.sync_activity.lock().unwrap().running, "the first sync runs");
    // Replaced there (what a restore with a session in it, or a new sign-in, does)
    n.set_sync(Some(Arc::new(PubkySync::from_session(
        homeserver.clone(),
        b.pop().unwrap(),
    ))));
    assert!(
        !n.sync_activity.lock().unwrap().running,
        "reset at the replacement"
    );
    let _ = release.send(());
    // The retired first sync ends; it never sets anything running again
    for _ in 0..50 {
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        assert!(!n.sync_activity.lock().unwrap().running);
    }
    let (st, v) = call(&router(n.clone()), "POST", "/v1/storage/save").await;
    assert_eq!(st, StatusCode::OK, "{v}");
    assert!(v["saving"]["last_saved_at"].is_number(), "the save ran: {v}");
}

/// The settings backup of a session replaced while it read the old identity's settings copy
/// doesn't adopt that identity's history delete: the node's listens and its delete time stay.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_replaced_settings_backup_adopts_no_history_delete() {
    let Some(mut ids) = identities_with_sessions(2, &["node.pubky.music"]).await else {
        eprintln!("PM_BUNDLED_HS_ADMIN / PM_BUNDLED_HS_PASSWORD not set: skipped");
        return;
    };
    let mut b = ids.pop().unwrap();
    let mut a = ids.pop().unwrap();
    let a_node = a.pop().unwrap();
    let homeserver = NodeHomeserver::new(network()).unwrap();
    // A's settings copy says A deleted its history just now
    let now_us = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_micros() as i64;
    homeserver
        .write_node_settings(
            &a_node,
            &json!({"version": 1, "saved_at": 0, "node": "elsewhere",
                "settings": {"listens.cleared_before": now_us.to_string()}}),
        )
        .await
        .unwrap();
    let d = tempfile::tempdir().unwrap();
    let n = node(d.path());
    let sync_a = Arc::new(PubkySync::from_session(homeserver.clone(), a_node));
    n.set_sync(Some(sync_a.clone()));
    let (reached, release) = pm_api::test_hooks::arm(&format!("sync.backup_after_read:{}", sync_a.z32()));
    let backup = {
        let (n, sync_a) = (n.clone(), sync_a.clone());
        tokio::spawn(async move { sync_a.backup_settings(&n).await })
    };
    reached.await.unwrap();
    n.set_sync(Some(Arc::new(PubkySync::from_session(
        homeserver.clone(),
        b.pop().unwrap(),
    ))));
    let _ = release.send(());
    backup.await.unwrap();
    assert_eq!(
        n.library
            .lock()
            .unwrap()
            .get_setting("listens.cleared_before")
            .unwrap(),
        None,
        "A's history delete was adopted"
    );
}

/// The settings backup checks the session after it has the library: a replacement made while it
/// waited for the lock (another task held it) is seen, and the old identity's history delete is
/// not adopted, so the node's listens stay.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_replacement_while_the_backup_waits_for_the_library_adopts_nothing() {
    let Some(mut ids) = identities_with_sessions(1, &["node.pubky.music"]).await else {
        eprintln!("PM_BUNDLED_HS_ADMIN / PM_BUNDLED_HS_PASSWORD not set: skipped");
        return;
    };
    let mut a = ids.pop().unwrap();
    let a_node = a.pop().unwrap();
    let homeserver = NodeHomeserver::new(network()).unwrap();
    let now_us = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_micros() as i64;
    homeserver
        .write_node_settings(
            &a_node,
            &json!({"version": 1, "saved_at": 0, "node": "elsewhere",
                "settings": {"listens.cleared_before": now_us.to_string()}}),
        )
        .await
        .unwrap();
    let d = tempfile::tempdir().unwrap();
    let n = node(d.path());
    // A listen of the node's, written before A's delete time
    let listen_id = pm_pubky::ids::timestamp_id(now_us - 10_000_000);
    n.library
        .lock()
        .unwrap()
        .upsert_music_post(&pm_library::MusicPostRow {
            id: listen_id.clone(),
            kind: "pubky.music/activity".into(),
            visibility: "private".into(),
            json: "{}".into(),
            created_at: now_us - 10_000_000,
            updated_at: now_us - 10_000_000,
            synced_at: None,
            deleted: false,
        })
        .unwrap();
    let sync_a = Arc::new(PubkySync::from_session(homeserver.clone(), a_node));
    n.set_sync(Some(sync_a.clone()));
    let (reached, release) = pm_api::test_hooks::arm(&format!("sync.backup_after_read:{}", sync_a.z32()));
    let backup = {
        let (n, sync_a) = (n.clone(), sync_a.clone());
        tokio::spawn(async move { sync_a.backup_settings(&n).await })
    };
    tokio::time::timeout(std::time::Duration::from_secs(60), reached)
        .await
        .expect("the backup reached its pause")
        .unwrap();
    // Another task holds the library; the backup goes on and waits for it
    let held = n.library.lock().unwrap();
    let _ = release.send(());
    std::thread::sleep(std::time::Duration::from_millis(300));
    // The session is replaced while the backup waits (only the flag: the library is held here)
    sync_a.retire();
    drop(held);
    backup.await.unwrap();
    let lib = n.library.lock().unwrap();
    assert_eq!(
        lib.get_setting("listens.cleared_before").unwrap(),
        None,
        "A's delete adopted"
    );
    let row = lib.get_music_post(&listen_id).unwrap().unwrap();
    assert!(
        !row.deleted,
        "the node's listen was deleted by A's history delete"
    );
}

/// A successful remote clear still cleans the captured local sign-ins if another identity
/// replaces its session before cleanup. A genuinely new sign-in with identical bytes survives.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_accepted_clear_does_not_migrate_old_sign_ins_to_a_replacement_identity() {
    let Some(mut ids) = identities_with_sessions(2, &["node.pubky.music"]).await else {
        eprintln!("PM_BUNDLED_HS_ADMIN / PM_BUNDLED_HS_PASSWORD not set: skipped");
        return;
    };
    let mut b = ids.pop().unwrap();
    let mut a = ids.pop().unwrap();
    let d = tempfile::tempdir().unwrap();
    let n = node(d.path());
    let sync_a = Arc::new(PubkySync::from_session(
        NodeHomeserver::new(network()).unwrap(),
        a.pop().unwrap(),
    ));
    n.set_sync(Some(sync_a.clone()));
    let tidal = r#"{"access_token":"fixture-at","refresh_token":"fixture-rt","user_id":"7"}"#;
    {
        let lib = n.library.lock().unwrap();
        lib.set_setting("tidal.user", tidal).unwrap();
        lib.set_setting("listenbrainz.account", r#"{"token":"fixture-lb","user":"owner"}"#)
            .unwrap();
        pm_api::vault_cache::new_sign_in(&lib, "tidal");
        pm_api::vault_cache::new_sign_in(&lib, "listenbrainz");
    }
    let (status, before) = call(&router(n.clone()), "POST", "/v1/vault/sync").await;
    assert_eq!(status, StatusCode::OK, "{before}");
    assert!(
        before["migrated"]
            .as_array()
            .unwrap()
            .contains(&json!("listenbrainz")),
        "{before}"
    );
    let (reached, release) = pm_api::test_hooks::arm(&format!("vault.after_wipe:{}", sync_a.z32()));
    let clear = {
        let n = n.clone();
        tokio::spawn(async move { call(&router(n), "POST", "/v1/vault/sign-out-everywhere").await })
    };
    tokio::time::timeout(std::time::Duration::from_secs(60), reached)
        .await
        .unwrap()
        .unwrap();
    n.set_sync(Some(Arc::new(PubkySync::from_session(
        NodeHomeserver::new(network()).unwrap(),
        b.pop().unwrap(),
    ))));
    {
        let lib = n.library.lock().unwrap();
        // Same contents, but a new interactive sign-in, after the clear was accepted.
        lib.set_setting("tidal.user", tidal).unwrap();
        pm_api::vault_cache::new_sign_in(&lib, "tidal");
    }
    let _ = release.send(());
    let (status, result) = clear.await.unwrap();
    assert_eq!(status, StatusCode::OK, "{result}");
    assert_eq!(result["signed_out_here"], json!(["listenbrainz"]));
    {
        let lib = n.library.lock().unwrap();
        assert_eq!(lib.get_setting("listenbrainz.account").unwrap(), None);
        assert_eq!(lib.get_setting("tidal.user").unwrap().as_deref(), Some(tidal));
        assert_eq!(lib.get_setting("vault.pending_local_clear").unwrap(), None);
    }
    let (status, result) = call(&router(n.clone()), "POST", "/v1/vault/sync").await;
    assert_eq!(status, StatusCode::OK, "{result}");
    let (sync_b, key) = node_vault_parts(&n);
    assert!(Vault::new(sync_b.session(), &key)
        .envelope("listenbrainz")
        .await
        .unwrap()
        .is_none());
}

/// Replaying the equal-newest signed envelope and old index after an accepted clear must
/// not reactivate the sign-in, including when a new NodeState reloads the durable records.
#[tokio::test]
async fn accepted_clear_remembers_its_owner_marker_before_any_later_sync() {
    use pm_appdata::vault::{blob_path, INDEX_PATH};
    let Some(mut ids) = identities_with_sessions(2, &["node.pubky.music"]).await else {
        eprintln!("PM_BUNDLED_HS_ADMIN / PM_BUNDLED_HS_PASSWORD not set: skipped");
        return;
    };
    let other = ids.pop().unwrap().pop().unwrap();
    let session = ids.pop().unwrap().pop().unwrap();
    let d = tempfile::tempdir().unwrap();
    let n = node(d.path());
    let sync = Arc::new(PubkySync::from_session(
        NodeHomeserver::new(network()).unwrap(),
        session,
    ));
    n.set_sync(Some(sync.clone()));
    {
        let lib = n.library.lock().unwrap();
        lib.set_setting("listenbrainz.account", r#"{"token":"fixture","user":"owner"}"#)
            .unwrap();
        pm_api::vault_cache::new_sign_in(&lib, "listenbrainz");
    }
    let (status, r) = call(&router(n.clone()), "POST", "/v1/vault/sync").await;
    assert_eq!(status, StatusCode::OK, "{r}");
    assert_eq!(r["migrated"], json!(["listenbrainz"]));
    let (_, key) = node_vault_parts(&n);
    let vault = Vault::new(sync.session(), &key);
    let saved_index = vault.index().await.unwrap();
    let saved_blob = vault.envelope("listenbrainz").await.unwrap().unwrap();
    let (status, r) = call(&router(n.clone()), "POST", "/v1/vault/sign-out-everywhere").await;
    assert_eq!(status, StatusCode::OK, "{r}");
    assert_eq!(r["signed_out_here"], json!(["listenbrainz"]));
    assert!(r["cache_error"].is_null(), "{r}");
    let marks = n.data_dir().join("vault/marks.json");
    assert!(Vault::new(sync.session(), &key)
        .keep_marks(&marks)
        .marker(&saved_index)
        .unwrap()
        .is_some());
    // The same machine file must not apply A's newly written marker to B.
    assert_eq!(
        Vault::new(&other, &key)
            .keep_marks(&marks)
            .marker(&saved_index)
            .unwrap(),
        None
    );
    sync.session()
        .storage()
        .put(INDEX_PATH, serde_json::to_vec(&saved_index).unwrap())
        .await
        .unwrap();
    let path = blob_path("listenbrainz").unwrap();
    sync.session()
        .storage()
        .put(path.as_str(), serde_json::to_vec(&saved_blob).unwrap())
        .await
        .unwrap();
    let reloaded = node(d.path());
    reloaded.set_sync(Some(sync));
    let (status, r) = call(&router(reloaded.clone()), "POST", "/v1/vault/sync").await;
    assert_eq!(status, StatusCode::OK, "{r}");
    assert_eq!(r["not_usable"], json!(["listenbrainz"]), "{r}");
    assert_eq!(
        reloaded
            .library
            .lock()
            .unwrap()
            .get_setting("listenbrainz.account")
            .unwrap(),
        None
    );
}

/// A new identical-content sign-in after the wipe belongs to a new lineage even on the
/// same owner; the next complete sync must preserve it and publish it again.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn accepted_clear_preserves_new_same_owner_lineage_through_next_sync() {
    let Some(mut ids) = identities_with_sessions(1, &["node.pubky.music"]).await else {
        eprintln!("PM_BUNDLED_HS_ADMIN / PM_BUNDLED_HS_PASSWORD not set: skipped");
        return;
    };
    let d = tempfile::tempdir().unwrap();
    let n = node(d.path());
    let sync = Arc::new(PubkySync::from_session(
        NodeHomeserver::new(network()).unwrap(),
        ids.pop().unwrap().pop().unwrap(),
    ));
    n.set_sync(Some(sync.clone()));
    let account = r#"{"token":"fixture","user":"owner"}"#;
    {
        let lib = n.library.lock().unwrap();
        lib.set_setting("listenbrainz.account", account).unwrap();
        pm_api::vault_cache::new_sign_in(&lib, "listenbrainz");
    }
    let (status, r) = call(&router(n.clone()), "POST", "/v1/vault/sync").await;
    assert_eq!(status, StatusCode::OK, "{r}");
    let (reached, release) = pm_api::test_hooks::arm(&format!("vault.after_wipe:{}", sync.z32()));
    let clear = {
        let n = n.clone();
        tokio::spawn(async move { call(&router(n), "POST", "/v1/vault/sign-out-everywhere").await })
    };
    tokio::time::timeout(std::time::Duration::from_secs(60), reached)
        .await
        .unwrap()
        .unwrap();
    let (_, key) = node_vault_parts(&n);
    let vault = Vault::new(sync.session(), &key);
    let wipe = Vault::signed_out_everywhere_at(&vault.index().await.unwrap()).unwrap();
    {
        let lib = n.library.lock().unwrap();
        lib.set_setting("listenbrainz.account", account).unwrap();
        lib.set_setting("vault.lineage.listenbrainz", &format!("{}-new-fixture", wipe + 1))
            .unwrap();
    }
    let _ = release.send(());
    let (status, r) = clear.await.unwrap();
    assert_eq!(status, StatusCode::OK, "{r}");
    assert_eq!(r["signed_out_here"], json!([]), "{r}");
    let (status, r) = call(&router(n.clone()), "POST", "/v1/vault/sync").await;
    assert_eq!(status, StatusCode::OK, "{r}");
    assert_eq!(r["pushed"], json!(["listenbrainz"]), "{r}");
    assert_eq!(
        n.library
            .lock()
            .unwrap()
            .get_setting("listenbrainz.account")
            .unwrap()
            .as_deref(),
        Some(account)
    );
    assert!(vault.envelope("listenbrainz").await.unwrap().is_some());
}

#[tokio::test]
async fn failed_wipe_marker_persistence_keeps_local_clear_intent() {
    let Some(mut ids) = identities_with_sessions(1, &["node.pubky.music"]).await else {
        eprintln!("PM_BUNDLED_HS_ADMIN / PM_BUNDLED_HS_PASSWORD not set: skipped");
        return;
    };
    let d = tempfile::tempdir().unwrap();
    let n = node(d.path());
    n.set_sync(Some(Arc::new(PubkySync::from_session(
        NodeHomeserver::new(network()).unwrap(),
        ids.pop().unwrap().pop().unwrap(),
    ))));
    n.library
        .lock()
        .unwrap()
        .set_setting("listenbrainz.account", r#"{"token":"fixture","user":"owner"}"#)
        .unwrap();
    let (status, r) = call(&router(n.clone()), "POST", "/v1/vault/sync").await;
    assert_eq!(status, StatusCode::OK, "{r}");
    // Only this fixture's marks path is made unwritable as a file.
    let marks = n.data_dir().join("vault/marks.json");
    std::fs::rename(&marks, marks.with_extension("saved")).unwrap();
    std::fs::create_dir(&marks).unwrap();
    let (status, r) = call(&router(n.clone()), "POST", "/v1/vault/sign-out-everywhere").await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR, "{r}");
    let lib = n.library.lock().unwrap();
    assert!(lib.get_setting("vault.pending_local_clear").unwrap().is_some());
    assert!(lib.get_setting("listenbrainz.account").unwrap().is_some());
}

/// Suspend A-to-B after publication, then complete B-to-C before A-to-B's trailing cleanup.
/// Every predecessor must already be retired when its replacement can be observed, and the
/// trailing cleanup keeps C's browser session and device sign-in made meanwhile.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn overlapping_identity_swaps_retire_before_publication() {
    let Some(mut ids) = identities_with_sessions(3, &["node.pubky.music"]).await else {
        eprintln!("PM_BUNDLED_HS_ADMIN / PM_BUNDLED_HS_PASSWORD not set: skipped");
        return;
    };
    let make = |session| {
        Arc::new(PubkySync::from_session(
            NodeHomeserver::new(network()).unwrap(),
            session,
        ))
    };
    let c = make(ids.pop().unwrap().pop().unwrap());
    let b = make(ids.pop().unwrap().pop().unwrap());
    let a = make(ids.pop().unwrap().pop().unwrap());
    let dir = tempfile::tempdir().unwrap();
    let n = node_with_token(dir.path(), Some("api-fixture"));
    n.set_sync(Some(a.clone()));
    let (reached, release) = pm_api::test_hooks::arm(&format!("sync.after_swap:{}", a.id()));
    let first = {
        let n = n.clone();
        let b = b.clone();
        tokio::task::spawn_blocking(move || n.set_sync(Some(b)))
    };
    tokio::time::timeout(std::time::Duration::from_secs(10), reached)
        .await
        .unwrap()
        .unwrap();
    assert!(a.is_retired(), "A must retire before B becomes visible");
    assert!(Arc::ptr_eq(&n.sync_handle().unwrap(), &b));
    n.set_sync(Some(c.clone()));
    assert!(a.is_retired() && b.is_retired() && !c.is_retired());
    // C's owner signs in from a browser and from the app while A-to-B's cleanup is still held
    let browser = pm_api::accounts::mint_owner(&n, c.z32()).unwrap();
    let device = pm_api::accounts::mint_device(&n, c.z32()).unwrap();
    {
        let mut activity = n.sync_activity.lock().unwrap();
        activity.owner = Some(c.id());
        activity.running = true;
        activity.step = Some("C owns this activity".into());
    }
    let _ = release.send(());
    tokio::time::timeout(std::time::Duration::from_secs(10), first)
        .await
        .unwrap()
        .unwrap();
    {
        let activity = n.sync_activity.lock().unwrap();
        assert_eq!(activity.owner, Some(c.id()));
        assert!(activity.running);
        assert_eq!(activity.step.as_deref(), Some("C owns this activity"));
    }
    let c_key = c.z32().trim_start_matches("pubky").to_string();
    for (what, token) in [("browser session", &browser), ("device sign-in", &device)] {
        let who = pm_api::accounts::caller(&n, Some(token.as_str()));
        assert!(
            who.as_ref().is_some_and(
                |w| w.role == pm_api::accounts::Role::Owner && w.key.as_deref() == Some(c_key.as_str())
            ),
            "C's {what} survives A-to-B's trailing cleanup: {:?}",
            who.map(|w| w.key)
        );
    }
}

/// Exercise the actual synchronous completion boundary with success, ordinary failure and
/// 507 outcomes held until a different identity has replaced the captured sender.
#[tokio::test]
async fn stale_storage_completions_preserve_replacement_status_and_quota() {
    let Some(mut ids) = identities_with_sessions(2, &["node.pubky.music"]).await else {
        eprintln!("PM_BUNDLED_HS_ADMIN / PM_BUNDLED_HS_PASSWORD not set: skipped");
        return;
    };
    let make = |session| {
        Arc::new(PubkySync::from_session(
            NodeHomeserver::new(network()).unwrap(),
            session,
        ))
    };
    let b = make(ids.pop().unwrap().pop().unwrap());
    let a = make(ids.pop().unwrap().pop().unwrap());
    let dir = tempfile::tempdir().unwrap();
    let n = node(dir.path());
    n.set_sync(Some(a.clone()));
    n.set_sync(Some(b.clone()));
    let full = format!("{}:123", b.z32());
    n.library
        .lock()
        .unwrap()
        .set_setting("storage.full_at", &full)
        .unwrap();
    {
        let mut st = n.storage_save.lock().unwrap();
        st.error = Some("B's existing failure".into());
        st.last_saved_at = Some(123);
    }
    let outcomes = || {
        [
            Ok(pm_api::types::SyncReport::default()),
            Ok(pm_api::types::SyncReport {
                errors: vec!["507 Insufficient Storage".into()],
                ..Default::default()
            }),
            Err(pm_api::sync::SyncError::Store("ordinary read failure".into())),
            Err(pm_api::sync::SyncError::Store("507 Insufficient Storage".into())),
        ]
    };
    for result in outcomes() {
        assert!(pm_api::storage::complete_save_for_test(&n, &a, result).is_ok());
        let st = n.storage_save.lock().unwrap();
        assert_eq!(st.error.as_deref(), Some("B's existing failure"));
        assert_eq!(st.last_saved_at, Some(123));
        drop(st);
        assert_eq!(
            n.library
                .lock()
                .unwrap()
                .get_setting("storage.full_at")
                .unwrap()
                .as_deref(),
            Some(full.as_str())
        );
    }
    // A current owner still receives its own ordinary error and its own quota attribution.
    assert!(pm_api::storage::complete_save_for_test(
        &n,
        &b,
        Err(pm_api::sync::SyncError::Store("507 Insufficient Storage".into()))
    )
    .is_err());
    assert!(n
        .library
        .lock()
        .unwrap()
        .get_setting("storage.full_at")
        .unwrap()
        .unwrap()
        .starts_with(&format!("{}:", b.z32())));
    assert!(pm_api::storage::complete_save_for_test(&n, &b, Ok(Default::default())).is_ok());
    let st = n.storage_save.lock().unwrap();
    assert!(st.error.is_none());
    assert!(st.last_saved_at.is_some());
}
