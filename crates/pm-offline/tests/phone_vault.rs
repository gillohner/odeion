//! Phone mode and a node share TIDAL through the sources vault on a real homeserver (VAULT.md,
//! ADR 0017): the phone takes a node's sign-in, writes its own in the node's form (one a node takes
//! as its own), renews under the blob's lock, and a sign-out on the phone leaves the vault too.
//!
//! Needs the homeserver's admin API in `PM_BUNDLED_HS_ADMIN` and `PM_BUNDLED_HS_PASSWORD` (CI's
//! `bundle` job); skipped otherwise. A test signer stands in for Pubky Ring, and a plain vault
//! client writing exactly what a node writes (`pm_sources::vault_form`) stands in for the node.

use axum::routing::post;
use axum::{Json, Router};
use pm_appdata::vault::ClientSecret;
use pm_offline::phone::account::Account;
use pm_offline::phone::{self, Config, Phone, TidalConf};
use pm_pubky::homeserver::NodeHomeserver;
use pm_pubky::vault::Vault;
use pm_sources::tidal::full::{self, Client, ModeState, Session};
use pm_sources::vault_form::{self, TIDAL_FULL};
use pubky::{Keypair, Pubky, PublicKey};
use serde_json::{json, Map, Value};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

#[path = "../../pm-api/tests/support/local_relays.rs"]
mod local_relays;

struct Admin {
    url: String,
    password: String,
}

fn admin() -> Option<Admin> {
    Some(Admin {
        url: std::env::var("PM_BUNDLED_HS_ADMIN").ok()?,
        password: std::env::var("PM_BUNDLED_HS_PASSWORD").ok()?,
    })
}

impl Admin {
    async fn call(&self, method: reqwest::Method, path: &str, body: Option<Value>) -> Value {
        let mut r = reqwest::Client::new()
            .request(method, format!("{}{path}", self.url))
            .header("X-Admin-Password", &self.password);
        if let Some(b) = body {
            r = r.json(&b);
        }
        let text = r
            .send()
            .await
            .expect("admin API")
            .error_for_status()
            .expect("admin API answers")
            .text()
            .await
            .unwrap();
        serde_json::from_str(&text).unwrap_or(Value::String(text))
    }
}

fn pubky() -> Pubky {
    local_relays::pubky()
}

fn signing_in() -> NodeHomeserver {
    local_relays::node_homeserver().with_auth_relay(local_relays::inbox())
}

/// A person on the homeserver and two approved clients: the node's and the phone's.
async fn person(a: &Admin) -> Vec<pubky::PubkySession> {
    let info = a.call(reqwest::Method::GET, "/info", None).await;
    let hs = PublicKey::try_from(info["public_key"].as_str().expect("homeserver key")).unwrap();
    let token = a
        .call(
            reqwest::Method::POST,
            "/generate_signup_token",
            Some(json!({"storage_quota_mb": "unlimited"})),
        )
        .await;
    let signer = pubky().signer(Keypair::random());
    signer
        .signup(&hs, Some(token.as_str().expect("a signup token")))
        .await
        .expect("sign-up");
    let mut sessions = Vec::new();
    for (i, caps) in [
        pm_pubky::homeserver::LOGIN_CAPABILITIES,
        pm_pubky::homeserver::PHONE_CAPABILITIES,
    ]
    .into_iter()
    .enumerate()
    {
        let flow = signing_in()
            .start_login_with(&format!("phone-vault-{i}.pubky.music"), caps)
            .unwrap();
        signer.approve_auth(flow.url()).await.expect("approve");
        sessions.push(flow.await_approval().await.expect("session"));
    }
    sessions
}

/// What a node's TIDAL full-tracks sign-in carries.
fn travels(token: &str, user: &str, expires_at: i64) -> Map<String, Value> {
    let mut m = Map::new();
    m.insert(
        "tidal.unofficial".into(),
        json!({"session": {"access_token": token, "refresh_token": "rt", "expires_at": expires_at,
                           "user_id": user, "country": "CH"}}),
    );
    m
}

fn session_of(phone: &Phone) -> Option<Session> {
    full::load(phone.store().as_ref()).session
}

#[tokio::test]
async fn the_phone_and_a_node_share_tidal_through_the_vault() {
    let Some(a) = admin() else {
        eprintln!("PM_BUNDLED_HS_ADMIN / PM_BUNDLED_HS_PASSWORD not set: skipped");
        return;
    };
    let sessions = person(&a).await;
    let now = pm_sources::now_unix();
    // TIDAL's token endpoint, for the renewal
    let renewed = Arc::new(AtomicUsize::new(0));
    let r2 = renewed.clone();
    let tidal = Router::new().route(
        "/v1/oauth2/token",
        post(move || {
            let r2 = r2.clone();
            async move {
                r2.fetch_add(1, Ordering::SeqCst);
                Json(json!({"access_token": "renewed-on-the-phone", "expires_in": 3600}))
            }
        }),
    );
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let tbase = format!("http://{}", l.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(l, tidal).await.unwrap() });

    // The node: a plain vault client writing what a node writes
    let node_key = ClientSecret::generate();
    let node = Vault::new(&sessions[0], &node_key).device("node");
    node.register("Living room node").await.unwrap();
    let node_copy = |t: &Map<String, Value>| {
        vault_form::sign_in_for(
            "tidal-full",
            TIDAL_FULL.carry,
            true,
            t,
            None,
            Some("device-client".into()),
            "1-0123456789abcdef",
        )
    };
    node.write_shared("tidal-full", &node_copy(&travels("node-token", "42", now + 3600)))
        .await
        .unwrap();

    // The phone
    let dir = tempfile::tempdir().unwrap();
    let p = Phone::open(Config {
        dir: dir.path().to_path_buf(),
        cache: dir.path().join("streams"),
        version: "test".into(),
        pubky: None,
        tidal: TidalConf {
            auth_base: tbase.clone(),
            login_base: tbase.clone(),
            v1_base: format!("{tbase}/v1"),
            browser: None,
            device: Some(Client {
                id: "device-client".into(),
                secret: "device-secret".into(),
                from: "builtin".into(),
            }),
        },
    })
    .unwrap();
    let acct = Account::of(signing_in(), sessions[1].clone());
    // First pass: the phone lists itself; the node then shares with it; the next pass takes it
    phone::vault::pass(&p, &acct).await.unwrap();
    assert_eq!(node.share_with_trusted().await.unwrap(), 1);
    let r = phone::vault::pass(&p, &acct).await.unwrap();
    assert_eq!(r.pulled, ["tidal-full"], "{r:?}");
    let s = session_of(&p).expect("the node's sign-in on the phone");
    assert_eq!(s.access_token, "node-token");
    assert!(full::load(p.store().as_ref()).enabled, "taken, it plays");
    // In step: nothing more to do
    assert_eq!(
        phone::vault::pass(&p, &acct).await.unwrap(),
        phone::vault::Report::default()
    );

    // A new sign-in on the phone goes out in the node's form: a node takes it as its own
    full::update(p.store().as_ref(), |st: &mut ModeState| {
        st.session = Some(Session {
            access_token: "phone-token".into(),
            refresh_token: Some("rt2".into()),
            expires_at: now + 7200,
            user_id: Some("43".into()),
            country: "CH".into(),
            ..Default::default()
        })
    });
    phone::vault::signed_in(&p, "tidal-full");
    let r = phone::vault::pass(&p, &acct).await.unwrap();
    assert_eq!(r.pushed, ["tidal-full"], "{r:?}");
    let seen = node.read("tidal-full").await.unwrap().expect("the node reads it");
    let settings = vault_form::node_settings(&seen.sign_in).expect("a node's own form (std matches)");
    assert_eq!(
        settings["tidal.unofficial"]["session"]["access_token"],
        "phone-token"
    );
    assert_eq!(
        seen.sign_in.client_id.as_deref(),
        Some("device-client"),
        "the app the node renews with"
    );
    assert_ne!(
        vault_form::node_lineage(&seen.sign_in).as_deref(),
        Some("1-0123456789abcdef"),
        "another account: a new sign-in"
    );

    // Due: the phone renews under the blob's lock, and the node reads the renewed one
    node.write_shared("tidal-full", &node_copy(&travels("due-token", "43", now + 5)))
        .await
        .unwrap();
    phone::vault::pass(&p, &acct).await.unwrap();
    assert_eq!(session_of(&p).unwrap().access_token, "due-token");
    assert!(phone::vault::renew_tidal(&p, &acct).await.unwrap());
    assert_eq!(renewed.load(Ordering::SeqCst), 1);
    assert_eq!(session_of(&p).unwrap().access_token, "renewed-on-the-phone");
    let seen = node.read("tidal-full").await.unwrap().unwrap();
    assert_eq!(seen.sign_in.tokens["access_token"], "renewed-on-the-phone");
    assert!(vault_form::node_settings(&seen.sign_in).is_some());

    // Signed out on the phone: out of the vault too
    full::update(p.store().as_ref(), |st: &mut ModeState| st.session = None);
    let r = phone::vault::pass(&p, &acct).await.unwrap();
    assert_eq!(r.removed, ["tidal-full"], "{r:?}");
    assert!(node.read("tidal-full").await.unwrap().is_none());

    // A node signs in anew and the phone, renewing, takes that one (not due, so as it is): the
    // phone holds the new sign-in's id, so the node signing out later is a sign-out here too
    node.write_shared("tidal-full", &node_copy(&travels("node-again", "44", now + 3600)))
        .await
        .unwrap();
    phone::vault::pass(&p, &acct).await.unwrap();
    assert_eq!(session_of(&p).unwrap().access_token, "node-again");
    let newer = format!("{}-fedcba9876543210", now + 60);
    node.write_shared(
        "tidal-full",
        &vault_form::sign_in_for(
            "tidal-full",
            TIDAL_FULL.carry,
            true,
            &travels("node-new-sign-in", "45", now + 3600),
            None,
            Some("device-client".into()),
            &newer,
        ),
    )
    .await
    .unwrap();
    assert!(phone::vault::renew_tidal(&p, &acct).await.unwrap());
    assert_eq!(session_of(&p).unwrap().access_token, "node-new-sign-in");
    assert_eq!(
        pm_sources::SessionStore::get(p.store().as_ref(), "vault.lineage.tidal-full").as_deref(),
        Some(newer.as_str()),
        "the id of the sign-in it took"
    );
    assert!(node.remove_if("tidal-full", &|_| true).await.unwrap());
    let r = phone::vault::pass(&p, &acct).await.unwrap();
    assert_eq!(r.dropped, ["tidal-full"], "signed out on the node: {r:?}");
    assert!(r.pushed.is_empty(), "never written back: {r:?}");
    assert!(session_of(&p).is_none());
    assert!(node.read("tidal-full").await.unwrap().is_none());
}

#[tokio::test]
async fn bookmarks_listens_and_prefs_reach_the_homeserver_as_a_node_reads_them() {
    let Some(a) = admin() else {
        eprintln!("PM_BUNDLED_HS_ADMIN / PM_BUNDLED_HS_PASSWORD not set: skipped");
        return;
    };
    let sessions = person(&a).await;
    let dir = tempfile::tempdir().unwrap();
    let p = Phone::open(Config {
        dir: dir.path().to_path_buf(),
        cache: dir.path().join("streams"),
        version: "test".into(),
        pubky: None,
        tidal: TidalConf::default(),
    })
    .unwrap();
    let (addr, _task) = phone::serve(p.clone()).await.unwrap();
    let base = format!("http://127.0.0.1:{}", addr.port());
    let http = reqwest::Client::new();
    // While the phone has no account yet, the page's changes wait on the phone
    let r = http
        .put(format!("{base}/v1/favourites"))
        .bearer_auth(p.key())
        .json(&json!({"uri": "tidal:track:12"}))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
    let url = r.json::<Value>().await.unwrap()["uri"]
        .as_str()
        .unwrap()
        .to_string();
    let r = http
        .post(format!("{base}/v1/library/history"))
        .bearer_auth(p.key())
        .json(&json!({"uri": "tidal:track:12", "duration_ms": 61000}))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
    // The node set a quality since: newer prefs.json
    let hs = signing_in();
    hs.write_prefs(
        &sessions[0],
        &json!({"version": 1, "updated_at": 4_102_444_800_000u64, "quality": {"tidal": "high"}, "home_layout": {"x": 1}}),
    )
    .await
    .unwrap();
    let acct = Account::of(signing_in(), sessions[1].clone());
    let r = phone::data::sync(&p, &acct).await;
    assert_eq!((r.written, r.left), (2, 0), "{r:?}");
    assert!(r.prefs_taken);
    assert_eq!(
        full::load(p.store().as_ref()).quality,
        "high",
        "the node's quality applies here"
    );
    // The node reads the Bookmark as its own
    let marks = hs.read_v1_bookmarks(&sessions[0]).await.unwrap();
    assert!(marks.iter().any(|(_, u, _)| *u == url), "{marks:?}");
    // The listen is a private music v1 post (the node's default)
    assert_eq!(posts(&sessions[0], "/priv/music/v1/posts/").await, 1);
    // The person keeps their listens public (on their node): the next one goes under /pub/
    let mut copy = hs
        .read_node_settings(&sessions[0])
        .await
        .unwrap()
        .unwrap_or_else(|| json!({"version": 1, "settings": {}}));
    copy["settings"]["listens.history"] = json!("public");
    hs.write_node_settings(&sessions[0], &copy).await.unwrap();
    let play = || {
        http.post(format!("{base}/v1/library/history"))
            .bearer_auth(p.key())
            .json(&json!({"uri": "tidal:track:13", "duration_ms": 61000}))
            .send()
    };
    assert_eq!(play().await.unwrap().status(), 200);
    phone::data::sync(&p, &acct).await;
    assert_eq!(posts(&sessions[0], "/pub/music/v1/posts/").await, 1);
    // No listen history: nothing written
    copy["settings"]["listens.history"] = json!("off");
    hs.write_node_settings(&sessions[0], &copy).await.unwrap();
    assert_eq!(play().await.unwrap().status(), 200);
    let r = phone::data::sync(&p, &acct).await;
    assert_eq!(r.left, 0, "{r:?}");
    assert_eq!(posts(&sessions[0], "/priv/music/v1/posts/").await, 1);
    assert_eq!(posts(&sessions[0], "/pub/music/v1/posts/").await, 1);
    // Removing the Bookmark on the phone removes it there
    let r = http
        .delete(format!("{base}/v1/favourites?uri={}", urlencode(&url)))
        .bearer_auth(p.key())
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
    phone::data::sync(&p, &acct).await;
    let marks = hs.read_v1_bookmarks(&sessions[0]).await.unwrap();
    assert!(!marks.iter().any(|(_, u, _)| *u == url));
    // Removed again (another device removed it meanwhile, say): nothing there is done, the queue
    // goes on
    let r = http
        .delete(format!("{base}/v1/favourites?uri={}", urlencode(&url)))
        .bearer_auth(p.key())
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
    let r = phone::data::sync(&p, &acct).await;
    assert_eq!((r.written, r.left), (1, 0), "{r:?}");
    // A quality set on the phone goes into prefs.json, keeping what else is there
    let r = http
        .put(format!("{base}/v1/sources/tidal/unofficial"))
        .bearer_auth(p.key())
        .json(&json!({"quality": "max"}))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
    phone::data::sync(&p, &acct).await;
    let prefs = hs.read_prefs(&sessions[0]).await.unwrap().unwrap();
    assert_eq!(prefs["quality"]["tidal"], "max");
    assert_eq!(prefs["home_layout"]["x"], 1);
}

/// How many objects the person has in `dir` (none when it was never written).
async fn posts(s: &pubky::PubkySession, dir: &str) -> usize {
    match s
        .storage()
        .list(dir)
        .unwrap()
        .limit(100)
        .shallow(true)
        .send()
        .await
    {
        Ok(l) => l.len(),
        Err(_) => 0,
    }
}

fn urlencode(s: &str) -> String {
    url::form_urlencoded::byte_serialize(s.as_bytes()).collect()
}
