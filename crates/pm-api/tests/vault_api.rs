//! Settings › Accounts' vault routes (STEWARD-VAULT.md item 5) on a real pubky-homeserver: the
//! device list, registering the node, "Sign out this device" and "Sign out everywhere".
//!
//! Needs the homeserver's admin API in `PM_BUNDLED_HS_ADMIN` and `PM_BUNDLED_HS_PASSWORD` (CI's
//! `bundle` job); skipped otherwise. A test signer stands in for Pubky Ring.

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
            token: None,
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
