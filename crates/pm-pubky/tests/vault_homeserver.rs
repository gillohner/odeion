//! The sources vault on a real pubky-homeserver (VAULT.md, STEWARD-VAULT.md item 2): two clients of
//! one person read and write sign-ins that the homeserver only sees encrypted.
//!
//! Needs the homeserver's admin API in `PM_BUNDLED_HS_ADMIN` and `PM_BUNDLED_HS_PASSWORD` (CI's
//! `bundle` job, against `docker compose up`); skipped otherwise. With `PM_TESTNET=1` too, the
//! clients resolve the homeserver on a local `pubky-testnet` (its admin API is at :6288, password
//! `admin`). A test signer stands in for Pubky Ring: it creates the account and approves each
//! client's sign-in.

use pm_appdata::vault::{blob_path, ClientSecret, SignIn, VaultError};
use pm_pubky::homeserver::NodeHomeserver;
use pm_pubky::vault::{Vault, VaultStoreError};
use pubky::{Keypair, Pubky, PublicKey};
use serde_json::json;

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
    async fn call(
        &self,
        method: reqwest::Method,
        path: &str,
        body: Option<serde_json::Value>,
    ) -> serde_json::Value {
        let mut r = reqwest::Client::new()
            .request(method, format!("{}{path}", self.url))
            .header("X-Admin-Password", &self.password);
        if let Some(b) = body {
            r = r.json(&b);
        }
        let resp = r
            .send()
            .await
            .expect("admin API")
            .error_for_status()
            .expect("admin API answers");
        let text = resp.text().await.unwrap();
        serde_json::from_str(&text).unwrap_or(serde_json::Value::String(text))
    }
}

/// A person on the homeserver, and `n` clients they approved (each its own grant session).
async fn person_with_clients(a: &Admin, n: usize) -> (pubky::PubkySigner, Vec<pubky::PubkySession>) {
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
    let node = NodeHomeserver::new(network()).unwrap();
    let mut sessions = Vec::new();
    for i in 0..n {
        let flow = node.start_login(&format!("vault-test-{i}.pubky.music")).unwrap();
        signer.approve_auth(flow.url()).await.expect("approve");
        sessions.push(flow.await_approval().await.expect("session"));
    }
    (signer, sessions)
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

fn sign_in(token: &str) -> SignIn {
    serde_json::from_value(json!({
        "service": "tidal", "way": "app", "label": "listener@example.org",
        "tokens": {"access_token": token, "refresh_token": format!("{token}-refresh")},
        "expires_at": 1761403600, "settings": {"quality": "lossless"}
    }))
    .unwrap()
}

#[tokio::test]
async fn two_clients_share_a_sign_in_the_homeserver_only_sees_encrypted() {
    let Some(a) = admin() else {
        eprintln!("PM_BUNDLED_HS_ADMIN / PM_BUNDLED_HS_PASSWORD not set: skipped");
        return;
    };
    let (_signer, sessions) = person_with_clients(&a, 2).await;
    let (ka, kb) = (ClientSecret::generate(), ClientSecret::generate());
    let va = Vault::new(&sessions[0], &ka);
    let vb = Vault::new(&sessions[1], &kb);
    assert_eq!(va.owner(), vb.owner());
    // Every client is in the device list (v2: a blob is taken only from a device listed there)
    va.register("A").await.unwrap();
    vb.register("B").await.unwrap();

    // Nothing yet
    assert!(va.read("tidal").await.unwrap().is_none());
    assert!(va.services().await.unwrap().is_empty());

    // A writes, wrapped to itself and B; B reads it
    assert_eq!(
        va.write("tidal", &sign_in("secret-access-1"), &[vb.client_key()])
            .await
            .unwrap(),
        1
    );
    let b_sees = vb.read("tidal").await.unwrap().expect("B opens it");
    assert_eq!(b_sees.sign_in.tokens["access_token"], "secret-access-1");
    assert_eq!(b_sees.sign_in.settings["quality"], "lossless");
    assert_eq!(b_sees.sign_in.created_by, Some(va.client_key()));
    // A blob written where there was none starts at the current time at least (above any copy
    // another node may have seen)
    let e1 = b_sees.epoch;
    assert!(e1 + 60 >= now(), "{e1}");

    // What the homeserver holds is ciphertext only
    let raw = sessions[0]
        .storage()
        .get(blob_path("tidal").unwrap().as_str())
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    for plain in ["secret-access-1", "listener@example.org", "lossless", "refresh"] {
        assert!(
            !raw.contains(plain),
            "{plain} is readable on the homeserver: {raw}"
        );
    }

    // B refreshes: same key and epoch while nobody is dropped, revision goes up
    assert_eq!(
        vb.write("tidal", &sign_in("secret-access-2"), &[va.client_key()])
            .await
            .unwrap(),
        2
    );
    let a_sees = va.read("tidal").await.unwrap().unwrap();
    assert_eq!((a_sees.epoch, a_sees.sign_in.revision), (e1, 2));
    assert_eq!(a_sees.sign_in.tokens["access_token"], "secret-access-2");
    assert_eq!(
        a_sees.sign_in.created_by,
        Some(va.client_key()),
        "created_by stays"
    );

    // A drops B: the key rotates and B can't read what comes next
    assert_eq!(
        va.write("tidal", &sign_in("secret-access-3"), &[]).await.unwrap(),
        3
    );
    let after = va.read("tidal").await.unwrap().unwrap();
    assert_eq!((after.epoch, after.recipients.len()), (e1 + 1, 1));
    match vb.read("tidal").await {
        Err(VaultStoreError::Vault(VaultError::NotARecipient)) => {}
        other => panic!("B still reads the vault: {other:?}"),
    }
    // ...and B, unable to read it, doesn't overwrite it
    match vb.write("tidal", &sign_in("from-b"), &[]).await {
        Err(VaultStoreError::NotOurs(s)) => assert_eq!(s, "tidal"),
        other => panic!("B overwrote a sign-in it can't read: {other:?}"),
    }
    assert_eq!(
        va.read("tidal").await.unwrap().unwrap().sign_in.tokens["access_token"],
        "secret-access-3"
    );

    // A second service, the listing, deleting
    va.write("qobuz", &sign_in("q"), &[]).await.unwrap();
    assert_eq!(va.services().await.unwrap(), ["qobuz", "tidal"]);
    va.delete("qobuz").await.unwrap();
    va.delete("qobuz").await.unwrap(); // nothing to delete is fine
    assert_eq!(va.services().await.unwrap(), ["tidal"]);
    assert!(matches!(
        va.write("../index", &sign_in("x"), &[]).await,
        Err(VaultStoreError::BadService(_))
    ));
}

/// The device list (item 3): clients prove their keys with their grants; only trusted entries get
/// keys; a new device is added by one that can read; forged, borrowed and signed-out entries don't.
#[tokio::test]
async fn devices_prove_their_keys_and_only_trusted_ones_get_sign_ins() {
    use pm_appdata::vault::{ClientEntry, Index, Proof, Revoked, INDEX_PATH};
    let Some(a) = admin() else {
        eprintln!("PM_BUNDLED_HS_ADMIN / PM_BUNDLED_HS_PASSWORD not set: skipped");
        return;
    };
    let (_signer, sessions) = person_with_clients(&a, 3).await;
    let keys: Vec<ClientSecret> = (0..3).map(|_| ClientSecret::generate()).collect();
    let v: Vec<Vault> = sessions
        .iter()
        .zip(&keys)
        .map(|(s, k)| Vault::new(s, k))
        .collect();

    // A (a node, signing its kind) and B register; both are trusted and each sees the other
    Vault::new(&sessions[0], &keys[0])
        .device("node")
        .register("Living room node")
        .await
        .unwrap();
    v[1].register("Phone").await.unwrap();
    v[1].touch("Phone").await.unwrap(); // fresh: nothing to do
    let ix = v[0].index().await.unwrap();
    assert_eq!(ix.clients.len(), 2);
    let trusted: Vec<_> = v[1].trusted(&ix).into_iter().map(|t| t.client_key).collect();
    assert_eq!(trusted, [v[0].client_key(), v[1].client_key()]);
    let devices = v[0].devices().await.unwrap();
    assert!(devices.iter().all(|d| d.trusted), "{devices:?}");
    assert_eq!(devices.iter().filter(|d| d.this_node).count(), 1);
    // The signed kind, and when each grant expires (for "approve again in Ring")
    assert_eq!(devices[0].device.as_deref(), Some("node"));
    assert_eq!(devices[1].device, None);
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    assert!(
        devices
            .iter()
            .all(|d| d.grant_expires_at.is_some_and(|e| e > now)),
        "{devices:?}"
    );

    // A writes for every trusted device; B reads, C (not registered) can't
    v[0].write_shared("tidal", &sign_in("shared-1")).await.unwrap();
    let e1 = v[0].envelope("tidal").await.unwrap().unwrap().epoch;
    assert_eq!(
        v[1].read("tidal").await.unwrap().unwrap().sign_in.tokens["access_token"],
        "shared-1"
    );
    assert!(v[2].read("tidal").await.is_err());

    // C registers; A adds it to what A can read, keeping the epoch
    v[2].register("Laptop").await.unwrap();
    assert_eq!(v[0].share_with_trusted().await.unwrap(), 1);
    assert_eq!(v[0].share_with_trusted().await.unwrap(), 0, "only once");
    let c_sees = v[2].read("tidal").await.unwrap().unwrap();
    assert_eq!(
        (c_sees.epoch, c_sees.sign_in.tokens["access_token"].as_str()),
        (e1, Some("shared-1"))
    );

    // Entries that don't prove themselves get nothing:
    // - another person's client, with its own (valid) grant and proof;
    let (_other, other_sessions) = person_with_clients(&a, 1).await;
    let ok = ClientSecret::generate();
    let ov = Vault::new(&other_sessions[0], &ok);
    ov.register("Someone else's laptop").await.unwrap();
    let foreign: ClientEntry = ov.index().await.unwrap().clients.remove(0);
    // - A's proof copied onto a key A's proof doesn't name;
    let mut borrowed: ClientEntry = ix.clients[0].clone();
    let stranger = ClientSecret::generate();
    borrowed.client_key = stranger.public();
    borrowed.name = "Borrowed".into();
    // - garbage.
    let mut garbage = borrowed.clone();
    garbage.client_key = ClientSecret::generate().public();
    garbage.proof = Proof {
        grant: "x.y.z".into(),
        pop: "a.b.c".into(),
    };
    let mut ix = v[0].index().await.unwrap();
    ix.clients
        .extend([foreign.clone(), borrowed.clone(), garbage.clone()]);
    sessions[0]
        .storage()
        .put(INDEX_PATH, serde_json::to_vec(&ix).unwrap())
        .await
        .unwrap();
    let ix = v[0].index().await.unwrap();
    let trusted: Vec<_> = v[0].trusted(&ix).into_iter().map(|t| t.client_key).collect();
    assert_eq!(trusted, [v[0].client_key(), v[1].client_key(), v[2].client_key()]);
    let devices = v[0].devices().await.unwrap();
    let problem = |k: &pm_appdata::vault::ClientKey| {
        devices
            .iter()
            .find(|d| d.client_key == k.to_hex())
            .unwrap()
            .problem
            .clone()
            .unwrap_or_default()
    };
    assert!(
        problem(&foreign.client_key).contains("someone else's"),
        "{}",
        problem(&foreign.client_key)
    );
    assert!(
        problem(&borrowed.client_key).contains("another key"),
        "{}",
        problem(&borrowed.client_key)
    );
    assert!(!problem(&garbage.client_key).is_empty());
    // - a trusted entry renamed on the homeserver ("Your phone" on the laptop): its name isn't
    //   the signed one, so it isn't trusted until the laptop registers again
    let mut ix = v[0].index().await.unwrap();
    let laptop = ix
        .clients
        .iter_mut()
        .find(|e| e.client_key == v[2].client_key())
        .unwrap();
    laptop.name = "Your phone".into();
    sessions[0]
        .storage()
        .put(INDEX_PATH, serde_json::to_vec(&ix).unwrap())
        .await
        .unwrap();
    let devices = v[0].devices().await.unwrap();
    let renamed = devices
        .iter()
        .find(|d| d.client_key == v[2].client_key().to_hex())
        .unwrap();
    assert!(
        !renamed.trusted && renamed.problem.as_deref().unwrap_or_default().contains("name"),
        "{renamed:?}"
    );
    v[2].register("Laptop").await.unwrap();
    assert!(v[0]
        .devices()
        .await
        .unwrap()
        .iter()
        .all(|d| d.client_key != v[2].client_key().to_hex() || d.trusted));
    // ...and share_with_trusted gives them nothing
    assert_eq!(v[0].share_with_trusted().await.unwrap(), 0);
    let env = v[0].envelope("tidal").await.unwrap().unwrap();
    assert_eq!(env.recipients.len(), 3);
    assert!(!env.has_recipient(&stranger.public()));

    // B is signed out (by grant id): no longer trusted, and it can't register again
    let b_jti = v[0]
        .trusted(&ix)
        .into_iter()
        .find(|t| t.client_key == v[1].client_key())
        .unwrap()
        .grant_id;
    let mut ix: Index = v[0].index().await.unwrap();
    ix.revoked.push(Revoked {
        client_key: None,
        grant_id: Some(b_jti),
        at: 1,
    });
    sessions[0]
        .storage()
        .put(INDEX_PATH, serde_json::to_vec(&ix).unwrap())
        .await
        .unwrap();
    let ix = v[0].index().await.unwrap();
    assert!(!v[0]
        .trusted(&ix)
        .iter()
        .any(|t| t.client_key == v[1].client_key()));
    assert!(matches!(
        v[1].register("Phone").await,
        Err(VaultStoreError::Revoked)
    ));
    // The next shared write leaves B out (and rotates, since B held a key)
    v[0].write_shared("tidal", &sign_in("shared-2")).await.unwrap();
    let after = v[0].read("tidal").await.unwrap().unwrap();
    assert_eq!(after.epoch, e1 + 1);
    assert!(v[1].read("tidal").await.is_err());
    assert_eq!(
        v[2].read("tidal").await.unwrap().unwrap().sign_in.tokens["access_token"],
        "shared-2"
    );
}

/// A service that rotates refresh tokens, as TIDAL and Spotify do: each refresh token works once,
/// so two clients refreshing the same copy would leave one of them signed out.
struct RotatingService {
    valid: std::sync::Mutex<String>,
    calls: std::sync::atomic::AtomicUsize,
    takes: std::time::Duration,
}

impl RotatingService {
    fn new(first: &str, takes: std::time::Duration) -> Self {
        Self {
            valid: std::sync::Mutex::new(first.into()),
            calls: Default::default(),
            takes,
        }
    }

    async fn refresh(&self, mut s: SignIn) -> Result<SignIn, pm_pubky::vault::RefreshFailed> {
        let n = self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
        tokio::time::sleep(self.takes).await;
        let mut valid = self.valid.lock().unwrap();
        if s.tokens["refresh_token"].as_str() != Some(valid.as_str()) {
            return Err(pm_pubky::vault::RefreshFailed("invalid_grant".into()));
        }
        *valid = format!("rt-{n}");
        s.tokens.insert("refresh_token".into(), json!(format!("rt-{n}")));
        s.tokens.insert("access_token".into(), json!(format!("at-{n}")));
        s.expires_at = Some(now() + 3600);
        Ok(s)
    }

    fn calls(&self) -> usize {
        self.calls.load(std::sync::atomic::Ordering::SeqCst)
    }
}

fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

fn expired() -> SignIn {
    serde_json::from_value(json!({
        "service": "tidal", "tokens": {"access_token": "at-0", "refresh_token": "rt-0"},
        "expires_at": now() - 10
    }))
    .unwrap()
}

/// Two clients find the same expired token at the same moment: one refreshes, the other uses
/// what it wrote (STEWARD-VAULT.md items 4 and 7), with the homeserver's lock or the lease file.
async fn two_clients_refresh_at_once(lease_only: bool, takes: std::time::Duration) {
    use pm_pubky::vault::Refreshed;
    let Some(a) = admin() else {
        eprintln!("PM_BUNDLED_HS_ADMIN / PM_BUNDLED_HS_PASSWORD not set: skipped");
        return;
    };
    let (_signer, sessions) = person_with_clients(&a, 2).await;
    let (ka, kb) = (ClientSecret::generate(), ClientSecret::generate());
    let (mut va, mut vb) = (Vault::new(&sessions[0], &ka), Vault::new(&sessions[1], &kb));
    if lease_only {
        (va, vb) = (va.lease_only(), vb.lease_only());
    }
    va.register("A").await.unwrap();
    vb.register("B").await.unwrap();
    va.write("tidal", &expired(), &[vb.client_key()]).await.unwrap();
    let svc = RotatingService::new("rt-0", takes);
    let (ra, rb) = tokio::join!(
        va.refresh("tidal", false, |s| svc.refresh(s)),
        vb.refresh("tidal", false, |s| svc.refresh(s)),
    );
    let (ra, rb) = (ra.expect("A"), rb.expect("B"));
    assert_eq!(svc.calls(), 1, "only one client asks the service");
    let ours = [&ra, &rb]
        .iter()
        .filter(|r| matches!(r, Refreshed::ByUs(_)))
        .count();
    assert_eq!(ours, 1, "{ra:?} / {rb:?}");
    // Both now hold the one valid token, and the vault has it too
    assert_eq!(ra.sign_in().tokens["refresh_token"], "rt-1");
    assert_eq!(rb.sign_in().tokens["refresh_token"], "rt-1");
    let stored = vb.read("tidal").await.unwrap().unwrap().sign_in;
    assert_eq!(stored.tokens["refresh_token"], "rt-1");
    assert_eq!(stored.revision, 2);
    assert!(stored.last_refresh.is_some());
    // Fresh now: nobody calls the service
    assert!(matches!(
        va.refresh("tidal", false, |s| svc.refresh(s)).await.unwrap(),
        Refreshed::Current(_)
    ));
    assert_eq!(svc.calls(), 1);
    // The service refuses a token (force): the refused copy isn't written over the vault's
    svc.valid
        .lock()
        .unwrap()
        .replace_range(.., "revoked-at-the-service");
    assert!(matches!(
        va.refresh("tidal", true, |s| svc.refresh(s)).await,
        Err(VaultStoreError::Refresh(e)) if e == "invalid_grant"
    ));
    assert_eq!(
        va.read("tidal").await.unwrap().unwrap().sign_in.tokens["refresh_token"],
        "rt-1"
    );
    // No lease is left behind
    let lease = sessions[0]
        .storage()
        .get(pm_appdata::vault::lease_path("tidal").unwrap().as_str())
        .await;
    assert!(lease.is_err(), "a lease was left behind");
}

#[tokio::test]
async fn two_clients_refresh_at_once_under_the_lock() {
    two_clients_refresh_at_once(false, std::time::Duration::from_millis(300)).await;
}

#[tokio::test]
async fn two_clients_refresh_at_once_with_the_lease_file() {
    two_clients_refresh_at_once(true, std::time::Duration::from_millis(300)).await;
}

/// A service slower than the lock's 30 s: the holder renews it, so at 31 s another client still
/// finds the sign-in locked (the Claude review of #92: 12 s didn't test renewal).
#[tokio::test]
async fn a_slow_refresh_keeps_its_lock() {
    use pm_pubky::vault::Refreshed;
    let Some(a) = admin() else {
        eprintln!("PM_BUNDLED_HS_ADMIN / PM_BUNDLED_HS_PASSWORD not set: skipped");
        return;
    };
    let (_signer, sessions) = person_with_clients(&a, 2).await;
    let ka = ClientSecret::generate();
    let va = Vault::new(&sessions[0], &ka);
    va.register("A").await.unwrap();
    va.write("tidal", &expired(), &[]).await.unwrap();
    let svc = RotatingService::new("rt-0", std::time::Duration::from_secs(33));
    let path = blob_path("tidal").unwrap();
    let (r, probe) = tokio::join!(va.refresh("tidal", false, |s| svc.refresh(s)), async {
        tokio::time::sleep(std::time::Duration::from_secs(31)).await;
        sessions[1]
            .storage()
            .lock(path.as_str(), std::time::Duration::from_secs(5))
            .await
    });
    assert!(probe.is_err(), "the lock lapsed after 30 s");
    let r = r.unwrap();
    assert!(matches!(r, Refreshed::ByUs(_)), "{r:?}");
    assert_eq!(r.sign_in().tokens["refresh_token"], "rt-1");
}

/// The same with the lease file: it is renewed, so a client that comes at 31 s waits for the
/// refresh instead of refreshing the used-up token itself.
#[tokio::test]
async fn a_slow_refresh_keeps_its_lease() {
    use pm_pubky::vault::Refreshed;
    let Some(a) = admin() else {
        eprintln!("PM_BUNDLED_HS_ADMIN / PM_BUNDLED_HS_PASSWORD not set: skipped");
        return;
    };
    let (_signer, sessions) = person_with_clients(&a, 2).await;
    let (ka, kb) = (ClientSecret::generate(), ClientSecret::generate());
    let va = Vault::new(&sessions[0], &ka).lease_only();
    let vb = Vault::new(&sessions[1], &kb).lease_only();
    va.register("A").await.unwrap();
    vb.register("B").await.unwrap();
    va.write("tidal", &expired(), &[vb.client_key()]).await.unwrap();
    let svc = RotatingService::new("rt-0", std::time::Duration::from_secs(33));
    let (ra, rb) = tokio::join!(va.refresh("tidal", false, |s| svc.refresh(s)), async {
        tokio::time::sleep(std::time::Duration::from_secs(31)).await;
        vb.refresh("tidal", false, |s| svc.refresh(s)).await
    });
    assert_eq!(svc.calls(), 1, "B took a lapsed lease");
    assert!(matches!(ra.unwrap(), Refreshed::ByUs(_)));
    let rb = rb.unwrap();
    assert!(matches!(rb, Refreshed::ByAnother(_)), "{rb:?}");
    assert_eq!(rb.sign_in().tokens["refresh_token"], "rt-1");
}

/// The Claude review of #92: once the service has rotated the token, the new one is the only one
/// that works. When it can't be written (here: too large for a blob), the caller gets it back.
#[tokio::test]
async fn a_refresh_that_cant_be_saved_hands_the_new_tokens_back() {
    let Some(a) = admin() else {
        eprintln!("PM_BUNDLED_HS_ADMIN / PM_BUNDLED_HS_PASSWORD not set: skipped");
        return;
    };
    let (_signer, sessions) = person_with_clients(&a, 1).await;
    let ka = ClientSecret::generate();
    let va = Vault::new(&sessions[0], &ka);
    va.register("A").await.unwrap();
    va.write("tidal", &expired(), &[]).await.unwrap();
    let svc = RotatingService::new("rt-0", std::time::Duration::ZERO);
    let r = va
        .refresh("tidal", false, |s| async {
            let mut s = svc.refresh(s).await?;
            s.tokens.insert("id_token".into(), json!("x".repeat(70 * 1024)));
            Ok(s)
        })
        .await;
    match r {
        Err(VaultStoreError::Unsaved { sign_in, .. }) => {
            assert_eq!(sign_in.tokens["refresh_token"], "rt-1")
        }
        other => panic!("{other:?}"),
    }
    // The vault still has the old copy, and no lock is left: a write goes through
    assert_eq!(
        va.read("tidal").await.unwrap().unwrap().sign_in.tokens["refresh_token"],
        "rt-0"
    );
    va.write("tidal", &expired(), &[]).await.unwrap();
}

/// The Claude review of #92: a write while the service answers that changes no tokens (here a
/// new device added, unlocked under the lease) is kept, and the new tokens go into it.
#[tokio::test]
async fn a_device_added_during_a_refresh_keeps_its_key() {
    use pm_pubky::vault::Refreshed;
    let Some(a) = admin() else {
        eprintln!("PM_BUNDLED_HS_ADMIN / PM_BUNDLED_HS_PASSWORD not set: skipped");
        return;
    };
    let (_signer, sessions) = person_with_clients(&a, 3).await;
    let keys: Vec<ClientSecret> = (0..3).map(|_| ClientSecret::generate()).collect();
    let va = Vault::new(&sessions[0], &keys[0]).lease_only();
    let vb = Vault::new(&sessions[1], &keys[1]);
    let vc = Vault::new(&sessions[2], &keys[2]);
    for (v, name) in [(&va, "A"), (&vb, "B"), (&vc, "C")] {
        v.register(name).await.unwrap();
    }
    va.write("tidal", &expired(), &[vb.client_key()]).await.unwrap();
    // B writes at 3.5 s: after A's lease (2 s to settle) and its read, while the service answers
    let svc = RotatingService::new("rt-0", std::time::Duration::from_secs(5));
    let (ra, wb) = tokio::join!(va.refresh("tidal", false, |s| svc.refresh(s)), async {
        tokio::time::sleep(std::time::Duration::from_millis(3500)).await;
        let cur = vb.read("tidal").await.unwrap().unwrap().sign_in;
        vb.write("tidal", &cur, &[va.client_key(), vc.client_key()]).await
    });
    wb.unwrap();
    let ra = ra.unwrap();
    assert!(matches!(ra, Refreshed::ByUs(_)), "{ra:?}");
    let c = vc.read("tidal").await.unwrap().unwrap();
    assert_eq!(c.sign_in.tokens["refresh_token"], "rt-1");
    assert_eq!(c.sign_in.revision, 3);
}

/// Revocation (item 5): "Sign out this device" rotates past it and keeps it out; a sign-in this
/// node can't read is reported for another device to rotate; "Sign out everywhere" removes them.
#[tokio::test]
async fn signing_a_device_out_and_signing_out_everywhere() {
    let Some(a) = admin() else {
        eprintln!("PM_BUNDLED_HS_ADMIN / PM_BUNDLED_HS_PASSWORD not set: skipped");
        return;
    };
    let (_signer, sessions) = person_with_clients(&a, 3).await;
    let keys: Vec<ClientSecret> = (0..3).map(|_| ClientSecret::generate()).collect();
    let v: Vec<Vault> = sessions
        .iter()
        .zip(&keys)
        .map(|(s, k)| Vault::new(s, k))
        .collect();
    for (i, name) in ["Node", "Phone", "Old laptop"].iter().enumerate() {
        v[i].register(name).await.unwrap();
    }
    v[0].write_shared("tidal", &sign_in("t")).await.unwrap();
    v[0].write_shared("qobuz", &sign_in("q")).await.unwrap();
    let mut first = std::collections::HashMap::new();
    for service in ["tidal", "qobuz"] {
        first.insert(service, v[0].envelope(service).await.unwrap().unwrap().epoch);
    }
    // One A can't read: the phone wrote it for itself and the laptop only
    v[1].write("deezer", &sign_in("d"), &[v[2].client_key()])
        .await
        .unwrap();
    assert!(v[2].read("tidal").await.unwrap().is_some());
    // Junk the laptop could write, and a sign-in another device is refreshing (locked): neither
    // stops the sign-out
    sessions[2]
        .storage()
        .put(blob_path("junk").unwrap().as_str(), b"not an envelope".to_vec())
        .await
        .unwrap();
    v[0].write_shared("spotify", &sign_in("s")).await.unwrap();
    let busy = sessions[1]
        .storage()
        .lock(
            blob_path("spotify").unwrap().as_str(),
            std::time::Duration::from_secs(30),
        )
        .await
        .unwrap();

    // The node can't sign itself out
    assert!(v[0]
        .sign_out_device(&v[0].client_key(), |_, _| async {})
        .await
        .is_err());

    // Sign the old laptop out
    let called = std::sync::Mutex::new(Vec::new());
    let report = v[0]
        .sign_out_device(&v[2].client_key(), |service, s| {
            called.lock().unwrap().push((service, s.revision));
            async {}
        })
        .await
        .unwrap();
    assert_eq!(report.rotated, ["qobuz", "tidal"]);
    assert_eq!(report.not_readable_here, ["deezer"]);
    let failed: Vec<&str> = report.failed.iter().map(|f| f.service.as_str()).collect();
    assert_eq!(failed, ["junk", "spotify"], "{report:?}");
    // Once the other device is done, signing out again rotates what was left
    sessions[1].storage().unlock(&busy).await.unwrap();
    let again = v[0]
        .sign_out_device(&v[2].client_key(), |_, _| async {})
        .await
        .unwrap();
    assert_eq!(again.rotated, ["spotify"], "{again:?}");
    assert_eq!(
        called
            .lock()
            .unwrap()
            .iter()
            .map(|(s, _)| s.as_str())
            .collect::<Vec<_>>(),
        ["qobuz", "tidal"]
    );
    // It reads nothing written from now on, the phone still does, and it can't come back
    for service in ["tidal", "qobuz"] {
        assert!(v[2].read(service).await.is_err(), "{service}");
        assert_eq!(
            v[1].read(service).await.unwrap().unwrap().epoch,
            first[service] + 1,
            "{service}"
        );
    }
    assert!(matches!(
        v[2].register("Old laptop").await,
        Err(VaultStoreError::Revoked)
    ));
    let ix = v[0].index().await.unwrap();
    assert_eq!(ix.clients.len(), 2);
    assert!(ix.is_revoked(&v[2].client_key(), None));
    // The phone, which can read deezer, rotates it past the laptop too
    let r = v[1]
        .sign_out_device(&v[2].client_key(), |_, _| async {})
        .await
        .unwrap();
    assert!(r.rotated.contains(&"deezer".to_string()), "{r:?}");
    assert!(v[2].read("deezer").await.is_err());

    // Sign out everywhere: every sign-in goes, the device list stays; one being refreshed by
    // another device is left and reported
    let busy = sessions[1]
        .storage()
        .lock(
            blob_path("tidal").unwrap().as_str(),
            std::time::Duration::from_secs(30),
        )
        .await
        .unwrap();
    let r = v[0].sign_out_everywhere(|_, _| async {}).await.unwrap();
    assert_eq!(r.removed, ["deezer", "junk", "qobuz", "spotify"]);
    assert_eq!(r.left.len(), 1);
    assert_eq!(r.left[0].service, "tidal");
    sessions[1].storage().unlock(&busy).await.unwrap();
    let r = v[0].sign_out_everywhere(|_, _| async {}).await.unwrap();
    assert_eq!(r.removed, ["tidal"]);
    assert!(v[0].services().await.unwrap().is_empty());
    assert_eq!(v[0].index().await.unwrap().clients.len(), 2);
}

/// The Claude review of #93: a restored node makes a new key under the same grant, and the
/// owner signs out the stale entry. Only that key goes; the node's grant stays trusted.
#[tokio::test]
async fn signing_out_an_old_key_of_this_node_keeps_the_node() {
    let Some(a) = admin() else {
        eprintln!("PM_BUNDLED_HS_ADMIN / PM_BUNDLED_HS_PASSWORD not set: skipped");
        return;
    };
    let (_signer, sessions) = person_with_clients(&a, 1).await;
    let (old, new) = (ClientSecret::generate(), ClientSecret::generate());
    Vault::new(&sessions[0], &old)
        .register("Living room node")
        .await
        .unwrap();
    let node = Vault::new(&sessions[0], &new);
    node.register("Living room node").await.unwrap();
    node.write_shared("tidal", &sign_in("t")).await.unwrap();
    assert_eq!(node.index().await.unwrap().clients.len(), 2);
    let report = node
        .sign_out_device(&old.public(), |_, _| async {})
        .await
        .unwrap();
    assert_eq!(report.rotated, ["tidal"]);
    let ix = node.index().await.unwrap();
    let trusted: Vec<_> = node.trusted(&ix).into_iter().map(|t| t.client_key).collect();
    assert_eq!(trusted, [new.public()]);
    assert!(
        ix.revoked.iter().all(|r| r.grant_id.is_none()),
        "{:?}",
        ix.revoked
    );
    node.register("Living room node").await.unwrap();
    assert!(Vault::new(&sessions[0], &old).read("tidal").await.is_err());
}

/// mn-app's review of #91: an app the person approved for all their storage (`/:rw`) is not a
/// vault client. It can't register, and an entry it writes itself, with its own valid proof, isn't
/// trusted, so `share_with_trusted` gives it nothing.
#[tokio::test]
async fn a_broad_grant_is_not_a_grant_for_the_sign_ins() {
    use pm_appdata::vault::{ClientEntry, ClientProofData, Proof, INDEX_PATH};
    let Some(a) = admin() else {
        eprintln!("PM_BUNDLED_HS_ADMIN / PM_BUNDLED_HS_PASSWORD not set: skipped");
        return;
    };
    let (signer, sessions) = person_with_clients(&a, 1).await;
    let node = NodeHomeserver::new(network()).unwrap();
    let flow = node.start_login_with("backup-tool.example", "/:rw").unwrap();
    signer.approve_auth(flow.url()).await.expect("approve");
    let broad = flow.await_approval().await.expect("session");
    let (kn, kb) = (ClientSecret::generate(), ClientSecret::generate());
    let vn = Vault::new(&sessions[0], &kn);
    vn.register("Node").await.unwrap();
    vn.write_shared("tidal", &sign_in("t")).await.unwrap();

    let vb = Vault::new(&broad, &kb);
    assert!(matches!(
        vb.register("Backup tool").await,
        Err(VaultStoreError::NeedsVaultGrant)
    ));
    // It writes an entry itself, with a proof its grant really signs
    let data = serde_json::to_value(ClientProofData::new(kb.public(), "Backup tool")).unwrap();
    let pop = broad.as_grant().unwrap().create_custom_pop(data).await.unwrap();
    let mut ix = vn.index().await.unwrap();
    ix.clients.push(ClientEntry {
        client_key: kb.public(),
        name: "Backup tool".into(),
        authorised_at: 1,
        last_used_at: 1,
        proof: Proof {
            grant: pop.grant.clone(),
            pop: pop.pop.clone(),
        },
        signer: None,
        extra: Default::default(),
    });
    broad
        .storage()
        .put(INDEX_PATH, serde_json::to_vec(&ix).unwrap())
        .await
        .unwrap();
    let ix = vn.index().await.unwrap();
    assert!(!vn.trusted(&ix).iter().any(|t| t.client_key == kb.public()));
    let d = vn.devices().await.unwrap();
    let e = d.iter().find(|d| d.client_key == kb.public().to_hex()).unwrap();
    assert!(
        e.problem
            .as_deref()
            .unwrap_or_default()
            .contains("doesn't name the vault"),
        "{e:?}"
    );
    assert_eq!(vn.share_with_trusted().await.unwrap(), 0);
    assert!(vb.read("tidal").await.is_err());
}

/// The Claude review of #91: `index.json` is writable by the devices it lists, the signed-out ones
/// too. A signed-out phone that writes the list back without its record, and its entry back in,
/// stays signed out for a node that remembers revocations; the node's next registration puts the
/// record back in the list.
#[tokio::test]
async fn a_signed_out_device_that_rewrites_the_list_stays_signed_out() {
    use pm_appdata::vault::{Index, Revoked, INDEX_PATH};
    let Some(a) = admin() else {
        eprintln!("PM_BUNDLED_HS_ADMIN / PM_BUNDLED_HS_PASSWORD not set: skipped");
        return;
    };
    let (_signer, sessions) = person_with_clients(&a, 2).await;
    let (kn, kd) = (ClientSecret::generate(), ClientSecret::generate());
    let file = std::env::temp_dir().join(format!("pm-revoked-{}.json", &kn.public().to_hex()[..12]));
    let node = Vault::new(&sessions[0], &kn).remember_revocations(&file);
    let phone = Vault::new(&sessions[1], &kd);
    node.register("Node").await.unwrap();
    phone.register("Phone").await.unwrap();
    node.write_shared("tidal", &sign_in("before")).await.unwrap();
    assert!(phone.read("tidal").await.is_ok());
    let full: Index = phone.index().await.unwrap();

    // The node signs the phone out (the index first, as #93's sign-out does)
    let jti = node
        .trusted(&full)
        .into_iter()
        .find(|t| t.client_key == phone.client_key())
        .unwrap()
        .grant_id;
    let mut ix = node.index().await.unwrap();
    ix.revoked.push(Revoked {
        client_key: Some(phone.client_key()),
        grant_id: Some(jti),
        at: 1,
    });
    sessions[0]
        .storage()
        .put(INDEX_PATH, serde_json::to_vec(&ix).unwrap())
        .await
        .unwrap();
    let seen = node.index().await.unwrap();
    assert!(!node
        .trusted(&seen)
        .iter()
        .any(|t| t.client_key == phone.client_key()));

    // The phone writes the list back as it was before, with its valid entry and no record
    sessions[1]
        .storage()
        .put(INDEX_PATH, serde_json::to_vec(&full).unwrap())
        .await
        .unwrap();
    assert!(phone.index().await.unwrap().revoked.is_empty());
    // The node still doesn't trust it, and gives it no key to what is written next
    let ix = node.index().await.unwrap();
    assert!(!node
        .trusted(&ix)
        .iter()
        .any(|t| t.client_key == phone.client_key()));
    node.write_shared("tidal", &sign_in("after")).await.unwrap();
    assert!(phone.read("tidal").await.is_err());
    assert_eq!(node.share_with_trusted().await.unwrap(), 0);
    assert!(phone.read("tidal").await.is_err());
    // The node's next registration writes the record back
    node.register("Node").await.unwrap();
    assert!(phone.index().await.unwrap().is_revoked(&phone.client_key(), None));
    let _ = std::fs::remove_file(&file);
}

/// VAULT.md v2: a blob is taken only when one of the person's devices signed it, as its index
/// entry proves; a copy older than one seen, or unsigned where a signed one was seen, is stale.
#[tokio::test]
async fn a_sign_in_is_taken_only_from_a_device_that_signed_it() {
    use pm_appdata::vault::{seal, sign_envelope, DataKey, Envelope, SigningKey};
    let Some(a) = admin() else {
        eprintln!("PM_BUNDLED_HS_ADMIN / PM_BUNDLED_HS_PASSWORD not set: skipped");
        return;
    };
    let (_signer, sessions) = person_with_clients(&a, 3).await;
    let keys: Vec<ClientSecret> = (0..3).map(|_| ClientSecret::generate()).collect();
    let dir = tempfile::tempdir().unwrap();
    let marks: Vec<std::path::PathBuf> = (0..3)
        .map(|i| dir.path().join(format!("marks-{i}.json")))
        .collect();
    let v: Vec<Vault> = (0..3)
        .map(|i| {
            Vault::new(&sessions[i], &keys[i])
                .device("node")
                .keep_marks(&marks[i])
        })
        .collect();
    let path = blob_path("tidal").unwrap();
    let owner = v[0].owner().to_string();
    let put = |body: Vec<u8>| {
        let s = &sessions[0];
        let path = path.clone();
        async move { s.storage().put(path.as_str(), body).await.unwrap() }
    };
    let raw = |env: &Envelope| serde_json::to_vec(env).unwrap();
    let plain = |s: &SignIn| serde_json::to_vec(s).unwrap();

    // A and B register (with signer proofs); A writes: signed by A, B reads it and sees so
    v[0].register("Living room node").await.unwrap();
    v[1].register("Kitchen node").await.unwrap();
    v[0].write_shared("tidal", &sign_in("s1")).await.unwrap();
    let first = v[0].envelope("tidal").await.unwrap().unwrap();
    assert_eq!(first.writer.as_ref().unwrap().client_key, v[0].client_key());
    let seen = v[1].read("tidal").await.unwrap().unwrap();
    assert_eq!(seen.signed_by, Some(v[0].client_key()));
    assert_eq!(seen.sign_in.tokens["access_token"], "s1");
    assert!(v[0].devices().await.unwrap().iter().all(|d| d.signs));

    // A blob sealed to A and B, signed by a key that is in no index entry: refused by both
    let stranger = ClientSecret::generate();
    let recipients = [v[0].client_key(), v[1].client_key()];
    let mut forged = seal(
        &owner,
        &path,
        1,
        &DataKey::generate(),
        &plain(&sign_in("evil")),
        &recipients,
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
    put(raw(&forged)).await;
    for x in [&v[0], &v[1]] {
        assert!(
            matches!(x.read("tidal").await, Err(VaultStoreError::NotADevice(_))),
            "a stranger's signature"
        );
    }
    // ...in A's name, with the stranger's key: the signature doesn't verify as A's
    let mut in_as_name = forged.clone();
    in_as_name.writer.as_mut().unwrap().client_key = v[0].client_key();
    put(raw(&in_as_name)).await;
    assert!(matches!(
        v[1].read("tidal").await,
        Err(VaultStoreError::NotADevice(_))
    ));
    // An unsigned copy where a signed one was seen: stale. A device that has seen nothing
    // signed (C, from before v2 as far as its record goes) still reads it: the migration window
    let mut unsigned = forged.clone();
    unsigned
        .recipients
        .push(pm_appdata::vault::wrap(&owner, &path, 1, &DataKey::generate(), &v[2].client_key()).unwrap());
    let unsigned = seal(
        &owner,
        &path,
        1,
        &DataKey::generate(),
        &plain(&sign_in("old-client")),
        &[v[0].client_key(), v[2].client_key()],
    )
    .unwrap();
    put(raw(&unsigned)).await;
    assert!(
        matches!(v[0].read("tidal").await, Err(VaultStoreError::Stale { .. })),
        "unsigned after signed"
    );
    // ...but only while some trusted device signs nothing: here A and B both sign, so C
    // refuses it as well
    assert!(
        matches!(v[2].read("tidal").await, Err(VaultStoreError::NotADevice(_))),
        "unsigned while every device signs"
    );
    let full = v[0].index().await.unwrap();
    let mut with_old = full.clone();
    with_old.clients[1].signer = None;
    let put_index = |ix: &pm_appdata::vault::Index| {
        let s = &sessions[0];
        let body = serde_json::to_vec(ix).unwrap();
        async move {
            s.storage()
                .put(pm_appdata::vault::INDEX_PATH, body)
                .await
                .unwrap()
        }
    };
    put_index(&with_old).await;
    // C has seen B's proof verify (its record says so), so B's entry without it is still a
    // client that signs: dropping a proof from the list doesn't reopen unsigned reads
    assert!(
        matches!(v[2].read("tidal").await, Err(VaultStoreError::NotADevice(_))),
        "a proof dropped from the list"
    );
    // A reader that never saw B sign (a fresh record) is in the migration window
    let fresh_marks = dir.path().join("marks-fresh.json");
    let fresh = Vault::new(&sessions[2], &keys[2])
        .device("node")
        .keep_marks(&fresh_marks);
    let c_sees = fresh.read("tidal").await.unwrap().unwrap();
    assert_eq!(
        (c_sees.signed_by, c_sees.sign_in.tokens["access_token"].as_str()),
        (None, Some("old-client"))
    );
    put_index(&full).await;

    // The real blob back; A writes again (revision 2), B reads it; the first copy served again
    // is older than what both have seen
    put(raw(&first)).await;
    v[0].write_shared("tidal", &sign_in("s2")).await.unwrap();
    let second = v[0].envelope("tidal").await.unwrap().unwrap();
    assert_eq!(v[1].read("tidal").await.unwrap().unwrap().sign_in.revision, 2);
    put(raw(&first)).await;
    for x in [&v[0], &v[1]] {
        assert!(
            matches!(x.read("tidal").await, Err(VaultStoreError::Stale { .. })),
            "an older copy"
        );
    }
    // ...and A's own write goes past it (above the newest copy A has seen), so sync never
    // stalls on it; the older copy served again afterwards is still older
    let past = v[0].write_shared("tidal", &sign_in("s3")).await.unwrap();
    assert!(past > 2, "{past}");
    assert!(v[0].read("tidal").await.unwrap().unwrap().epoch > second.epoch);
    put(raw(&first)).await;
    assert!(matches!(
        v[1].read("tidal").await,
        Err(VaultStoreError::Stale { .. })
    ));
    put(raw(&second)).await;
    // A signed blob changed after signing (a recipient dropped) is refused
    let current = {
        v[0].write_shared("tidal", &sign_in("s3")).await.unwrap();
        v[0].envelope("tidal").await.unwrap().unwrap()
    };
    let mut cut = current.clone();
    cut.recipients.retain(|r| r.client_key == v[0].client_key());
    put(raw(&cut)).await;
    assert!(
        matches!(v[0].read("tidal").await, Err(VaultStoreError::NotADevice(_))),
        "tampered"
    );
    put(raw(&current)).await;
    assert_eq!(
        v[0].read("tidal").await.unwrap().unwrap().sign_in.tokens["access_token"],
        "s3"
    );

    // B writes (signed by B); then B is signed out: what B signed is never taken afterwards,
    // and A's rewrite is signed by A
    v[1].write_shared("tidal", &sign_in("by-b")).await.unwrap();
    let by_b = v[0].envelope("tidal").await.unwrap().unwrap();
    assert_eq!(by_b.writer.as_ref().unwrap().client_key, v[1].client_key());
    assert_eq!(
        v[0].read("tidal").await.unwrap().unwrap().signed_by,
        Some(v[1].client_key())
    );
    let report = v[0]
        .sign_out_device(&v[1].client_key(), |_, _| async {})
        .await
        .unwrap();
    // What B wrote while trusted is rotated past B, now signed by A; B's copy served again is
    // refused (B is out, and it is older)
    assert_eq!(report.rotated, ["tidal"], "{report:?}");
    // B's copy is no longer trusted, so A writes past it: above its epoch, and at the current
    // time at least (not first.epoch + 1 once a second has passed)
    let after = v[0].read("tidal").await.unwrap().unwrap();
    assert_eq!(after.signed_by, Some(v[0].client_key()));
    assert!(after.epoch > by_b.epoch, "{} > {}", after.epoch, by_b.epoch);
    put(raw(&by_b)).await;
    assert!(v[0].read("tidal").await.is_err(), "B's blob after B's sign-out");
    // A blob signed by a key the list never held is not rotated from its content: A's own
    // copy goes over it instead (write_shared), past it
    let mut by_stranger = seal(
        &owner,
        &path,
        7,
        &DataKey::generate(),
        &plain(&sign_in("evil-again")),
        &[v[0].client_key()],
    )
    .unwrap();
    sign_envelope(
        &owner,
        &path,
        &mut by_stranger,
        stranger.public(),
        &SigningKey::derive(&stranger),
    )
    .unwrap();
    put(raw(&by_stranger)).await;
    assert!(matches!(
        v[0].read("tidal").await,
        Err(VaultStoreError::NotADevice(_))
    ));
    v[0].write_shared("tidal", &sign_in("mine-again")).await.unwrap();
    let mine = v[0].read("tidal").await.unwrap().unwrap();
    assert_eq!(mine.signed_by, Some(v[0].client_key()));
    assert!(
        mine.epoch > 7 && mine.epoch > after.epoch,
        "past the stranger's epoch and this node's newest: {}",
        mine.epoch
    );
    // A device from before v2 (no signer proof) signs nothing: its entry says so
    let mut ix = v[0].index().await.unwrap();
    ix.clients.iter_mut().for_each(|e| e.signer = None);
    sessions[0]
        .storage()
        .put(pm_appdata::vault::INDEX_PATH, serde_json::to_vec(&ix).unwrap())
        .await
        .unwrap();
    assert!(v[0].devices().await.unwrap().iter().all(|d| !d.signs));
    // ...and its own touch puts the proof back
    v[0].touch("Living room node").await.unwrap();
    assert!(v[0]
        .devices()
        .await
        .unwrap()
        .iter()
        .any(|d| d.this_node && d.signs));
}

/// A signer proof counts only under the entry's own grant: another device of the person (here
/// still trusted; a signed-out one holds its grant just as well) that puts a proof of its own key
/// into another device's entry can't sign blobs in that device's name.
#[tokio::test]
async fn a_signer_proof_under_another_grant_gives_no_signing_key() {
    use pm_appdata::vault::{seal, sign_envelope, DataKey, Proof, SignerProofData, INDEX_PATH};
    let Some(a) = admin() else {
        eprintln!("PM_BUNDLED_HS_ADMIN / PM_BUNDLED_HS_PASSWORD not set: skipped");
        return;
    };
    let (_signer, sessions) = person_with_clients(&a, 2).await;
    let (ka, kb) = (ClientSecret::generate(), ClientSecret::generate());
    let dir = tempfile::tempdir().unwrap();
    let (ma, mb) = (dir.path().join("a.json"), dir.path().join("b.json"));
    let va = Vault::new(&sessions[0], &ka).keep_marks(&ma);
    let vb = Vault::new(&sessions[1], &kb).keep_marks(&mb);
    va.register("A").await.unwrap();
    vb.register("B").await.unwrap();
    va.write_shared("tidal", &sign_in("by-a")).await.unwrap();
    assert!(vb.read("tidal").await.is_ok());

    // B proves its own signing key under its own grant, in A's name, into A's entry
    let data =
        serde_json::to_value(SignerProofData::new(va.client_key(), vb.signing_key().public())).unwrap();
    let theirs = sessions[1]
        .as_grant()
        .unwrap()
        .create_custom_pop(data)
        .await
        .unwrap();
    let mut ix = va.index().await.unwrap();
    for e in ix.clients.iter_mut().filter(|e| e.client_key == va.client_key()) {
        e.signer = Some(Proof {
            grant: theirs.grant.clone(),
            pop: theirs.pop.clone(),
        });
    }
    sessions[1]
        .storage()
        .put(INDEX_PATH, serde_json::to_vec(&ix).unwrap())
        .await
        .unwrap();
    // ...and writes a blob signed with its key in A's name
    let owner = va.owner().to_string();
    let path = blob_path("tidal").unwrap();
    let mut env = seal(
        &owner,
        &path,
        now() + 100,
        &DataKey::generate(),
        &serde_json::to_vec(&sign_in("by-b-as-a")).unwrap(),
        &[va.client_key(), vb.client_key()],
    )
    .unwrap();
    sign_envelope(&owner, &path, &mut env, va.client_key(), &vb.signing_key()).unwrap();
    sessions[1]
        .storage()
        .put(path.as_str(), serde_json::to_vec(&env).unwrap())
        .await
        .unwrap();
    for v in [&va, &vb] {
        assert!(
            matches!(v.read("tidal").await, Err(VaultStoreError::NotADevice(_))),
            "a signature in A's name by B's key"
        );
    }
    // A's entry gives no signing key now (until A publishes its own proof again)
    assert!(va
        .devices()
        .await
        .unwrap()
        .iter()
        .any(|d| d.this_node && !d.signs));
}

/// A blob gone (deleted by anyone with write access) keeps its mark: an older copy put back is
/// still refused, after a sign-out everywhere too, and a sign-in written anew starts above it. A
/// copy at the last epoch is refused instead of wrapping around.
#[tokio::test]
async fn a_blob_deleted_and_put_back_older_is_still_stale() {
    use pm_appdata::vault::{seal, sign_envelope, DataKey, SigningKey, INDEX_PATH};
    let Some(a) = admin() else {
        eprintln!("PM_BUNDLED_HS_ADMIN / PM_BUNDLED_HS_PASSWORD not set: skipped");
        return;
    };
    let (_signer, sessions) = person_with_clients(&a, 2).await;
    let (ka, kb) = (ClientSecret::generate(), ClientSecret::generate());
    let dir = tempfile::tempdir().unwrap();
    let (ma, mb) = (dir.path().join("a.json"), dir.path().join("b.json"));
    let va = Vault::new(&sessions[0], &ka).keep_marks(&ma);
    let vb = Vault::new(&sessions[1], &kb).keep_marks(&mb);
    va.register("A").await.unwrap();
    vb.register("B").await.unwrap();
    let path = blob_path("tidal").unwrap();
    va.write_shared("tidal", &sign_in("old")).await.unwrap();
    let old = va.envelope("tidal").await.unwrap().unwrap();
    va.write_shared("tidal", &sign_in("new")).await.unwrap();
    let newer = va.envelope("tidal").await.unwrap().unwrap();
    assert_eq!(vb.read("tidal").await.unwrap().unwrap().sign_in.revision, 2);
    let put = |body: Vec<u8>| {
        let s = &sessions[1];
        let path = path.clone();
        async move { s.storage().put(path.as_str(), body).await.unwrap() }
    };

    // Deleted, seen gone, then the older copy put back: still older than what both saw
    sessions[1].storage().delete(path.as_str()).await.unwrap();
    for v in [&va, &vb] {
        assert!(v.read("tidal").await.unwrap().is_none());
    }
    put(serde_json::to_vec(&old).unwrap()).await;
    for v in [&va, &vb] {
        assert!(
            matches!(v.read("tidal").await, Err(VaultStoreError::Stale { .. })),
            "the older copy put back"
        );
    }
    // ...after a sign-out everywhere too (the marks outlive the marker moving)
    let mut ix = va.index().await.unwrap();
    ix.extra
        .insert(pm_pubky::vault::SIGNED_OUT_EVERYWHERE.into(), json!(now()));
    sessions[0]
        .storage()
        .put(INDEX_PATH, serde_json::to_vec(&ix).unwrap())
        .await
        .unwrap();
    for v in [&va, &vb] {
        let ix = v.index().await.unwrap();
        assert!(v.marker(&ix).unwrap().is_some());
        assert!(matches!(
            v.read("tidal").await,
            Err(VaultStoreError::Stale { .. })
        ));
    }
    // A sign-in written where there is none starts above the newest copy seen: B takes it
    sessions[1].storage().delete(path.as_str()).await.unwrap();
    va.write_shared("tidal", &sign_in("anew")).await.unwrap();
    let anew = vb.read("tidal").await.unwrap().unwrap();
    assert!(anew.epoch > newer.epoch, "{} > {}", anew.epoch, newer.epoch);
    assert_eq!(anew.sign_in.tokens["access_token"], "anew");

    // A blob nobody trusted signed, one below the last revision: A's write past it lands at the
    // last one, and A's next rewrite is refused, not wrapped around to 0
    let stranger = ClientSecret::generate();
    let mut evil = sign_in("evil");
    evil.revision = u64::MAX - 1;
    let mut near = seal(
        va.owner(),
        &path,
        anew.epoch + 1,
        &DataKey::generate(),
        &serde_json::to_vec(&evil).unwrap(),
        &[va.client_key()],
    )
    .unwrap();
    sign_envelope(
        va.owner(),
        &path,
        &mut near,
        stranger.public(),
        &SigningKey::derive(&stranger),
    )
    .unwrap();
    put(serde_json::to_vec(&near).unwrap()).await;
    assert_eq!(
        va.write_shared("tidal", &sign_in("mine")).await.unwrap(),
        u64::MAX
    );
    assert_eq!(
        va.read("tidal").await.unwrap().unwrap().sign_in.revision,
        u64::MAX
    );
    assert!(
        va.write_shared("tidal", &sign_in("mine-2")).await.is_err(),
        "the last revision"
    );
    // ...and at the last epoch: A's write past it is refused too
    let mut last = seal(
        va.owner(),
        &path,
        u64::MAX,
        &DataKey::generate(),
        &serde_json::to_vec(&sign_in("evil")).unwrap(),
        &[va.client_key()],
    )
    .unwrap();
    sign_envelope(
        va.owner(),
        &path,
        &mut last,
        stranger.public(),
        &SigningKey::derive(&stranger),
    )
    .unwrap();
    put(serde_json::to_vec(&last).unwrap()).await;
    assert!(va.write_shared("tidal", &sign_in("mine")).await.is_err());
}

/// A sign-out everywhere dated later than now (a forged marker, a clock far ahead) is no
/// sign-out: it is neither used nor remembered, so a fixed device list puts things right.
#[tokio::test]
async fn a_sign_out_everywhere_dated_in_the_future_is_ignored() {
    use pm_appdata::vault::INDEX_PATH;
    let Some(a) = admin() else {
        eprintln!("PM_BUNDLED_HS_ADMIN / PM_BUNDLED_HS_PASSWORD not set: skipped");
        return;
    };
    let (_signer, sessions) = person_with_clients(&a, 1).await;
    let k = ClientSecret::generate();
    let dir = tempfile::tempdir().unwrap();
    let marks = dir.path().join("marks.json");
    let v = Vault::new(&sessions[0], &k).keep_marks(&marks);
    v.register("A").await.unwrap();
    let at = |t: Option<u64>| {
        let s = &sessions[0];
        let v = &v;
        async move {
            let mut ix = v.index().await.unwrap();
            match t {
                Some(t) => ix
                    .extra
                    .insert(pm_pubky::vault::SIGNED_OUT_EVERYWHERE.into(), json!(t)),
                None => ix.extra.remove(pm_pubky::vault::SIGNED_OUT_EVERYWHERE),
            };
            s.storage()
                .put(INDEX_PATH, serde_json::to_vec(&ix).unwrap())
                .await
                .unwrap();
            let ix = v.index().await.unwrap();
            v.marker(&ix).unwrap()
        }
    };
    assert_eq!(at(Some(u64::MAX)).await, None, "far in the future");
    assert_eq!(at(Some(now() + 86_400)).await, None, "a day ahead");
    assert_eq!(at(None).await, None, "not remembered");
    // Within the clock skew it counts, and is remembered
    let soon = now() + 30;
    assert_eq!(at(Some(soon)).await, Some(soon));
    assert_eq!(at(None).await, Some(soon));
}

/// A blob nobody signed (the migration window) is not given to a new device under this node's
/// signature: the node vouches only for what it signed or wrote itself.
#[tokio::test]
async fn an_unsigned_blob_is_not_shared_on_under_this_nodes_signature() {
    use pm_appdata::vault::{seal, DataKey, INDEX_PATH};
    let Some(a) = admin() else {
        eprintln!("PM_BUNDLED_HS_ADMIN / PM_BUNDLED_HS_PASSWORD not set: skipped");
        return;
    };
    let (_signer, sessions) = person_with_clients(&a, 3).await;
    let keys: Vec<ClientSecret> = (0..3).map(|_| ClientSecret::generate()).collect();
    let dir = tempfile::tempdir().unwrap();
    let marks: Vec<std::path::PathBuf> = (0..3).map(|i| dir.path().join(format!("m{i}.json"))).collect();
    let v: Vec<Vault> = (0..3)
        .map(|i| Vault::new(&sessions[i], &keys[i]).keep_marks(&marks[i]))
        .collect();
    v[0].register("A").await.unwrap();
    v[1].register("B").await.unwrap();
    // B is a client from before v2 (no signer proof): unsigned blobs are still read
    let mut ix = v[0].index().await.unwrap();
    for e in ix
        .clients
        .iter_mut()
        .filter(|e| e.client_key == v[1].client_key())
    {
        e.signer = None;
    }
    sessions[0]
        .storage()
        .put(INDEX_PATH, serde_json::to_vec(&ix).unwrap())
        .await
        .unwrap();
    // An unsigned copy sealed to A alone (one trusted device left out on purpose)
    let path = blob_path("qobuz").unwrap();
    let env = seal(
        v[0].owner(),
        &path,
        1,
        &DataKey::generate(),
        &serde_json::to_vec(&sign_in("planted")).unwrap(),
        &[v[0].client_key()],
    )
    .unwrap();
    sessions[1]
        .storage()
        .put(path.as_str(), serde_json::to_vec(&env).unwrap())
        .await
        .unwrap();
    assert!(v[0].read("qobuz").await.unwrap().unwrap().signed_by.is_none());
    // C joins; A shares what it can: not the unsigned blob
    v[2].register("C").await.unwrap();
    assert_eq!(v[0].share_with_trusted().await.unwrap(), 0);
    let after = v[0].envelope("qobuz").await.unwrap().unwrap();
    assert!(after.writer.is_none(), "not signed by A");
    assert!(!after.has_recipient(&v[2].client_key()));
}

/// Signing a device out rotates past it only what this node read and verified while the device
/// was listed, unchanged since. A blob the device wrote that this node never saw (it could have
/// written it after its entry left the list: its session still can), or one older than a copy
/// this node has seen (put back), is left as it is and reported as not readable here; the node's
/// own copy goes over it at its next sync.
#[tokio::test]
async fn signing_a_device_out_rotates_only_what_this_node_verified() {
    let Some(a) = admin() else {
        eprintln!("PM_BUNDLED_HS_ADMIN / PM_BUNDLED_HS_PASSWORD not set: skipped");
        return;
    };
    let (_signer, sessions) = person_with_clients(&a, 3).await;
    let keys: Vec<ClientSecret> = (0..3).map(|_| ClientSecret::generate()).collect();
    let dir = tempfile::tempdir().unwrap();
    let marks: Vec<std::path::PathBuf> = (0..3).map(|i| dir.path().join(format!("{i}.json"))).collect();
    let v: Vec<Vault> = sessions
        .iter()
        .zip(&keys)
        .zip(&marks)
        .map(|((s, k), m)| Vault::new(s, k).keep_marks(m))
        .collect();
    for (i, name) in ["Node", "Laptop", "Phone"].iter().enumerate() {
        v[i].register(name).await.unwrap();
    }
    // The laptop writes two sign-ins, which the node reads (and so verifies)...
    v[1].write_shared("tidal", &sign_in("t1")).await.unwrap();
    v[1].write_shared("qobuz", &sign_in("q1")).await.unwrap();
    for service in ["tidal", "qobuz"] {
        assert!(v[0].read(service).await.unwrap().is_some(), "{service}");
    }
    let qobuz_verified = v[0].envelope("qobuz").await.unwrap().unwrap();
    // ...then tidal again, which the node doesn't see before the laptop is signed out
    v[1].write_shared("tidal", &sign_in("t2")).await.unwrap();
    let tidal_unseen = v[0].envelope("tidal").await.unwrap().unwrap();
    let r = v[0]
        .sign_out_device(&v[1].client_key(), |_, _| async {})
        .await
        .unwrap();
    assert_eq!(r.rotated, ["qobuz"], "{r:?}");
    assert_eq!(r.not_readable_here, ["tidal"], "{r:?}");
    assert_eq!(
        v[0].envelope("tidal").await.unwrap().unwrap(),
        tidal_unseen,
        "the blob the node never verified is left as it is"
    );
    assert!(matches!(
        v[0].read("tidal").await,
        Err(VaultStoreError::NotADevice(_))
    ));
    // The phone reads qobuz rotated past the laptop; the laptop no longer does
    assert_eq!(
        v[2].read("qobuz").await.unwrap().unwrap().sign_in.tokens["access_token"],
        "q1"
    );
    assert!(v[1].read("qobuz").await.is_err());

    // Older copies put back: the one the laptop wrote (verified back then, but the laptop is out
    // now), and the node's own first copy of a sign-in it wrote again since (older than the copy
    // the node has seen). Signing the phone out writes neither again
    v[0].write_shared("spotify", &sign_in("s1")).await.unwrap();
    let spotify_first = v[0].envelope("spotify").await.unwrap().unwrap();
    v[0].write_shared("spotify", &sign_in("s2")).await.unwrap();
    for (service, env) in [("qobuz", &qobuz_verified), ("spotify", &spotify_first)] {
        sessions[0]
            .storage()
            .put(
                blob_path(service).unwrap().as_str(),
                serde_json::to_vec(env).unwrap(),
            )
            .await
            .unwrap();
    }
    assert!(matches!(
        v[0].read("qobuz").await,
        Err(VaultStoreError::NotADevice(_))
    ));
    assert!(matches!(
        v[0].read("spotify").await,
        Err(VaultStoreError::Stale { .. })
    ));
    let r = v[0]
        .sign_out_device(&v[2].client_key(), |_, _| async {})
        .await
        .unwrap();
    assert_eq!(r.rotated, Vec::<String>::new(), "{r:?}");
    assert_eq!(r.not_readable_here, ["qobuz", "spotify", "tidal"], "{r:?}");
    assert_eq!(v[0].envelope("qobuz").await.unwrap().unwrap(), qobuz_verified);
    assert_eq!(v[0].envelope("spotify").await.unwrap().unwrap(), spotify_first);
}

/// A device's envelope this node once verified, replayed over the node's own newer write before
/// the device is signed out, is not rotated past it: the node's own write is the newest verified
/// copy, and an older one is never written again from its content.
#[tokio::test]
async fn a_replayed_envelope_of_a_device_is_not_rotated_past_it() {
    let Some(a) = admin() else {
        eprintln!("PM_BUNDLED_HS_ADMIN / PM_BUNDLED_HS_PASSWORD not set: skipped");
        return;
    };
    let (_signer, sessions) = person_with_clients(&a, 2).await;
    let (ka, kb) = (ClientSecret::generate(), ClientSecret::generate());
    let dir = tempfile::tempdir().unwrap();
    let marks = dir.path().join("a.json");
    let va = Vault::new(&sessions[0], &ka).keep_marks(&marks);
    let vb = Vault::new(&sessions[1], &kb);
    va.register("Node").await.unwrap();
    vb.register("Laptop").await.unwrap();
    // The laptop writes a sign-in, which the node reads and so verifies...
    vb.write_shared("tidal", &sign_in("laptop")).await.unwrap();
    assert_eq!(
        va.read("tidal").await.unwrap().unwrap().sign_in.tokens["access_token"],
        "laptop"
    );
    let laptops = va.envelope("tidal").await.unwrap().unwrap();
    // ...then writes a newer one itself, and the laptop's old envelope comes back (a replay)
    va.write_shared("tidal", &sign_in("node")).await.unwrap();
    let path = blob_path("tidal").unwrap();
    sessions[0]
        .storage()
        .put(path.as_str(), serde_json::to_vec(&laptops).unwrap())
        .await
        .unwrap();
    assert!(matches!(
        va.read("tidal").await,
        Err(VaultStoreError::Stale { .. })
    ));
    // Signing the laptop out leaves it as it is: the node's own copy goes over it at its sync
    let r = va
        .sign_out_device(&vb.client_key(), |_, _| async {})
        .await
        .unwrap();
    assert_eq!(r.rotated, Vec::<String>::new(), "{r:?}");
    assert_eq!(r.not_readable_here, ["tidal"], "{r:?}");
    assert_eq!(va.envelope("tidal").await.unwrap().unwrap(), laptops);
    // The node writes past it as at any sync: a new key, the next epoch, its own content
    va.write_shared("tidal", &sign_in("node-again")).await.unwrap();
    let now = va.read("tidal").await.unwrap().unwrap();
    assert!(now.epoch > laptops.epoch);
    assert_eq!(now.sign_in.tokens["access_token"], "node-again");
    assert!(vb.read("tidal").await.is_err(), "the laptop reads nothing new");
}
