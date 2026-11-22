//! TIDAL full tracks through TIDAL's private API (the way tidal-dl-ng and python-tidal play):
//! the code sign-in (a device client) and Sign in with TIDAL (OAuth with PKCE, the browser
//! client), renewing a session with the client that made it, and the private playback endpoint
//! (`playbackinfopostpaywall`): one file (BTS) or DASH segments, never an encrypted stream.
//!
//! The state is one person's [`ModeState`] under [`STATE_KEY`] in a [`SessionStore`]; the node
//! keeps it in its settings for whoever asks, the phone in its own file.

use super::ClientTag;
use crate::types::CheckResult;
use crate::{form, now_unix as now, SessionStore, SourceError, StatusCode};
use base64::Engine;
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const STATE_KEY: &str = "tidal.unofficial";

pub const CLIENT_KEY: &str = "tidal.unofficial_client";

pub const SCOPE: &str = "r_usr w_usr w_sub";

/// The browser sign-in's scope, exactly as python-tidal's `pkce_get_auth_token` sends it.
pub const PKCE_SCOPE: &str = "r_usr+w_usr+w_sub";

/// Where TIDAL sends the browser after the browser sign-in (python-tidal's `pkce_uri_redirect`):
/// a tidal.com page whose address carries the code.
pub const PKCE_REDIRECT: &str = "https://tidal.com/android/login/auth";

/// How long a browser sign-in waits for its pasted address.
pub const BROWSER_WAIT_SECS: i64 = 600;

/// The health line of a code sign-in that gets AAC where lossless was asked for.
pub const AAC_ONLY_DEVICE: &str =
    "TIDAL sends this sign-in AAC only (lossy). Sign in with TIDAL again for lossless";

/// The same for a device session imported from another app or pasted (it wasn't signed in here).
pub const AAC_ONLY_IMPORTED: &str =
    "TIDAL sends this sign-in AAC only (lossy). Sign in with TIDAL for lossless";

/// The same on a build without the browser client: what is possible then.
pub const AAC_ONLY_NO_BROWSER: &str = "TIDAL sends this sign-in AAC only (lossy). Lossless needs Sign in with TIDAL, which this build doesn't have (a release build carries it)";

/// The health line of a browser sign-in whose last track still came as AAC.
pub const AAC_TRACK: &str = "TIDAL sent the last track as AAC (lossy): it may have no lossless version";

/// What the page says before the mode is switched on.
pub const WARNING: &str = "This uses TIDAL's private API, which TIDAL has not approved for other apps. It can stop working at any time, and TIDAL could act on the account. It needs a paid TIDAL plan.";

/// TIDAL's answer when the client comes from its developer portal, in plain words.
pub const NOT_A_DEVICE_CLIENT: &str = "This client can't use the code sign-in. Apps from TIDAL's developer portal are not device clients; enter a TIDAL device client instead.";

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Session {
    pub access_token: String,
    #[serde(default)]
    pub refresh_token: Option<String>,
    pub expires_at: i64,
    #[serde(default)]
    pub user_id: Option<String>,
    #[serde(default)]
    pub country: String,
    #[serde(default)]
    pub session_id: Option<String>,
    /// The app this sign-in was imported from (`tidal-dl-ng`, `pasted`, …); none for the node's
    /// own device sign-in (the node's `imports`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from: Option<String>,
    /// The client that made it, and renews it. Absent in a session stored before there were two:
    /// a device session.
    #[serde(default, skip_serializing_if = "ClientKind::is_device")]
    pub client: ClientKind,
    /// The client an imported file says made it, when it names one (only for the import's
    /// "renews here" check; never stored).
    #[serde(skip)]
    pub named_client: Option<ClientTag>,
}

/// Which client a session belongs to (TIDAL renews a session only with the client that made it).
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ClientKind {
    /// The code sign-in's device client.
    #[default]
    Device,
    /// The browser sign-in's client (OAuth with PKCE).
    Pkce,
}

impl ClientKind {
    pub fn is_device(&self) -> bool {
        *self == ClientKind::Device
    }

    pub fn words(self) -> &'static str {
        match self {
            ClientKind::Device => "code sign-in",
            ClientKind::Pkce => "browser sign-in",
        }
    }
}

/// A browser sign-in on its way: what the token request needs once the address is pasted back.
/// Short-lived and per person; never shown.
#[derive(Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct BrowserPending {
    pub code_verifier: String,
    pub client_unique_key: String,
    pub expires_at: i64,
}

impl std::fmt::Debug for BrowserPending {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BrowserPending")
            .field("expires_at", &self.expires_at)
            .finish_non_exhaustive()
    }
}

/// The browser sign-in's client (built in; tests set their own). Its Debug names no value.
#[derive(Clone, PartialEq, Eq)]
pub struct BrowserClient {
    pub id: String,
    pub secret: String,
}

impl std::fmt::Debug for BrowserClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("BrowserClient { .. }")
    }
}

impl BrowserClient {
    /// This build's, if it carries one.
    pub fn built_in() -> Option<Self> {
        super::built_in_pkce_client().map(|(id, secret)| Self { id, secret })
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Pending {
    pub device_code: String,
    pub user_code: String,
    pub uri: String,
    pub expires_at: i64,
    pub interval: u64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Health {
    pub ok: bool,
    pub detail: String,
    pub at: i64,
    /// Working, but TIDAL sent AAC (lossy) where lossless was asked for.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub lossy: bool,
}

/// What the node keeps for this mode (the session is a sign-in: a backup carries it only under
/// the owner's passphrase).
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct ModeState {
    #[serde(default)]
    pub enabled: bool,
    /// `max` (up to 24-bit/192 kHz) or `high` (16-bit/44.1 kHz).
    #[serde(default)]
    pub quality: String,
    #[serde(default)]
    pub session: Option<Session>,
    #[serde(default)]
    pub login: Option<Pending>,
    /// A browser sign-in waiting for its pasted address.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub browser: Option<BrowserPending>,
    #[serde(default)]
    pub health: Option<Health>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Client {
    pub id: String,
    pub secret: String,
    /// `settings`, `config` or `builtin` (this build's).
    pub from: String,
}

pub fn session_from(v: &Value, previous: Option<&Session>) -> Option<Session> {
    Some(Session {
        access_token: v["access_token"].as_str()?.to_string(),
        refresh_token: v["refresh_token"]
            .as_str()
            .map(str::to_string)
            .or_else(|| previous.and_then(|p| p.refresh_token.clone())),
        expires_at: now() + v["expires_in"].as_i64().unwrap_or(3600),
        user_id: previous.and_then(|p| p.user_id.clone()),
        country: previous.map(|p| p.country.clone()).unwrap_or_default(),
        session_id: previous.and_then(|p| p.session_id.clone()),
        from: previous.and_then(|p| p.from.clone()),
        client: previous.map(|p| p.client).unwrap_or_default(),
        named_client: None,
    })
}

/// Why a build can't sign in with TIDAL in the browser.
pub const NO_BROWSER_CLIENT: &str = "This build has no client for Sign in with TIDAL (a release build carries it). Use the code sign-in instead.";

/// PKCE's S256 challenge of a verifier: base64url, no padding, of its SHA-256.
pub fn code_challenge(verifier: &str) -> String {
    use sha2::Digest;
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(sha2::Sha256::digest(verifier.as_bytes()))
}

/// A new attempt: python-tidal's verifier (32 random bytes, base64url without padding) and
/// client unique key (64 random bits in hex).
pub fn new_browser_pending() -> BrowserPending {
    use rand::RngCore;
    let mut rng = rand::thread_rng();
    let mut bytes = [0u8; 32];
    rng.fill_bytes(&mut bytes);
    BrowserPending {
        code_verifier: base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes),
        client_unique_key: format!("{:02x}", rng.next_u64()),
        expires_at: now() + BROWSER_WAIT_SECS,
    }
}

/// The login.tidal.com address for an attempt, with python-tidal's `pkce_login_url` parameters
/// in its order.
pub fn authorize_url(login_base: &str, client_id: &str, p: &BrowserPending) -> String {
    let challenge = code_challenge(&p.code_verifier);
    format!(
        "{login_base}/authorize?{}",
        form(&[
            ("response_type", "code"),
            ("redirect_uri", PKCE_REDIRECT),
            ("client_id", client_id),
            ("lang", "EN"),
            ("appMode", "android"),
            ("client_unique_key", p.client_unique_key.as_str()),
            ("code_challenge", challenge.as_str()),
            ("code_challenge_method", "S256"),
            ("restrict_signup", "true"),
        ])
    )
}

/// The sign-in code from what was pasted back: the address of the tidal.com page TIDAL showed
/// (https, on tidal.com or one of its subdomains, checked before anything is read from it), or the
/// bare code. The error is for the person, in plain words.
pub fn code_from_paste(text: &str) -> Result<String, String> {
    let t = text.trim();
    if t.is_empty() {
        return Err("Paste the address of the TIDAL page you landed on after signing in.".into());
    }
    let looks_like_address = t.contains("://") || t.contains('/') || t.contains('?');
    if !looks_like_address {
        // A bare code: what TIDAL issues is URL-safe
        let fine = t.len() <= 2048
            && t.chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '~'));
        return if fine {
            Ok(t.to_string())
        } else {
            Err("That is neither a TIDAL address nor a sign-in code. Paste the whole address of the TIDAL page you landed on.".into())
        };
    }
    let with_scheme = if t.contains("://") {
        t.to_string()
    } else {
        format!("https://{t}")
    };
    let url = url::Url::parse(&with_scheme).map_err(|_| {
        "That address can't be read. Paste the whole address of the TIDAL page you landed on.".to_string()
    })?;
    if url.scheme() != "https" {
        return Err("That address doesn't start with https://. Paste the address of the TIDAL page exactly as your browser shows it.".into());
    }
    let host = url
        .host_str()
        .unwrap_or_default()
        .trim_end_matches('.')
        .to_ascii_lowercase();
    if host != "tidal.com" && !host.ends_with(".tidal.com") {
        return Err(format!(
            "That isn't a TIDAL address ({}). Paste the address of the tidal.com page you landed on after signing in.",
            if host.is_empty() { "no host" } else { host.as_str() }
        ));
    }
    let q = |k: &str| {
        url.query_pairs()
            .find(|(n, _)| n == k)
            .map(|(_, v)| v.into_owned())
    };
    if let Some(e) = q("error") {
        let why = q("error_description").unwrap_or(e);
        return Err(format!(
            "TIDAL didn't sign you in: {}. Press Sign in with TIDAL to try again.",
            why.chars().take(200).collect::<String>()
        ));
    }
    match q("code").map(|c| c.trim().to_string()).filter(|c| !c.is_empty()) {
        Some(code) => Ok(code),
        None => Err("That TIDAL address has no sign-in code in it. Sign in first; the page TIDAL shows afterwards carries it (its address has ?code=…).".into()),
    }
}

/// What TIDAL's answer to the token request of the client check means.
pub fn judge_client_check(status: u16, v: &Value) -> CheckResult {
    let err = v["error"].as_str().unwrap_or("");
    let (ok, detail) = match (status, err) {
        (_, "authorization_pending" | "slow_down") | (200..=299, _) => (
            true,
            "OK: TIDAL accepts this device client. Sign in with a code next.".to_string(),
        ),
        (401, _) | (_, "invalid_client" | "unauthorized_client") => (
            false,
            "TIDAL knows this client ID, but the secret doesn't go with it. Check the secret.".to_string(),
        ),
        _ => (
            false,
            format!(
                "TIDAL answered {status}{}",
                v["error_description"]
                    .as_str()
                    .or(v["error"].as_str())
                    .map(|d| format!(": {d}"))
                    .unwrap_or_default()
            ),
        ),
    };
    CheckResult { ok, detail }
}

/// A full stream: the reader, the extension the decoder probes with, and the media type.
pub struct FullStream {
    pub reader: Box<dyn std::io::Read + Send>,
    /// A segmented (DASH) stream's segments, so a zone can seek in it; `reader` reads them all.
    pub segments: Option<crate::segmented::Segments>,
    pub ext: &'static str,
    pub mime: &'static str,
    /// TIDAL sent AAC (or another lossy codec) where lossless was asked for.
    pub lossy: bool,
    /// The sign-in it was asked with.
    pub client: ClientKind,
    /// The sign-in was imported from another app or pasted (not made on this node).
    pub imported: bool,
    /// This build has the browser client (Sign in with TIDAL is possible).
    pub browser: bool,
    /// What TIDAL answered.
    pub answer: Answer,
}

impl FullStream {
    /// The health line for how it played: lossless asked for (Max or High) and AAC came means
    /// the code sign-in for a device session (Sign in with TIDAL is the way to lossless, where
    /// this build has it), and a track with no lossless version for a browser one.
    pub fn health_line(&self) -> &'static str {
        match (self.lossy, self.client) {
            (false, _) => "full tracks are playing",
            (true, ClientKind::Pkce) => AAC_TRACK,
            (true, ClientKind::Device) if !self.browser => AAC_ONLY_NO_BROWSER,
            (true, ClientKind::Device) if self.imported => AAC_ONLY_IMPORTED,
            (true, ClientKind::Device) => AAC_ONLY_DEVICE,
        }
    }
}

/// Whether a failure is the mode's (switch it off) or only this track's.
#[derive(Debug)]
pub enum FullError {
    Mode(String),
    Track(String),
}

/// What TIDAL answered for a track, as the log and the health line say it: never a URL or token.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Answer {
    /// The `audioquality` asked for.
    pub asked: String,
    /// `audioQuality` (`HI_RES_LOSSLESS`, `LOSSLESS`, `HIGH`, `LOW`).
    pub audio_quality: String,
    /// `manifestMimeType`.
    pub manifest: String,
    /// The BTS manifest's `codecs`, or the DASH representation's `codecs` attribute.
    pub codecs: String,
    pub bit_depth: Option<u64>,
    pub sample_rate: Option<u64>,
}

impl Answer {
    /// AAC (or another lossy codec) instead of FLAC.
    pub fn lossy(&self) -> bool {
        let c = self.codecs.to_ascii_lowercase();
        if c.is_empty() {
            return matches!(self.audio_quality.as_str(), "HIGH" | "LOW");
        }
        !(c.starts_with("flac") || c.starts_with("alac") || c.starts_with("mqa"))
    }
}

impl std::fmt::Display for Answer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let or = |v: &str| {
            if v.is_empty() {
                "-".to_string()
            } else {
                v.to_string()
            }
        };
        write!(
            f,
            "audioQuality={} manifestMimeType={} codecs={}",
            or(&self.audio_quality),
            or(&self.manifest),
            or(&self.codecs)
        )?;
        if let Some(b) = self.bit_depth {
            write!(f, " bitDepth={b}")?;
        }
        if let Some(r) = self.sample_rate {
            write!(f, " sampleRate={r}")?;
        }
        Ok(())
    }
}

/// Where the audio of one answer is.
pub enum Media {
    /// One file (BTS): its URL, the decoder's extension, the media type.
    File(String, &'static str, &'static str),
    /// DASH: the initialization segment, then the media segments, with their times.
    Segments(crate::segmented::Segments),
}

/// Asks the private playback endpoint for one track at one quality (python-tidal's
/// `Track.get_stream`, with its Android web view's User-Agent and client version).
pub fn playback_info(
    client: &reqwest::blocking::Client,
    v1: &str,
    sess: &Session,
    id: &str,
    quality: &str,
) -> Result<Value, FullError> {
    let mut q: Vec<(&str, String)> = vec![
        ("playbackmode", "STREAM".into()),
        ("audioquality", quality.into()),
        ("assetpresentation", "FULL".into()),
        ("countryCode", sess.country.clone()),
    ];
    if let Some(sid) = &sess.session_id {
        q.push(("sessionId", sid.clone()));
    }
    let resp = client
        .get(format!("{v1}/tracks/{id}/playbackinfopostpaywall"))
        .bearer_auth(&sess.access_token)
        .header(reqwest::header::USER_AGENT, PLAYBACK_USER_AGENT)
        .header("x-tidal-client-version", PLAYBACK_CLIENT_VERSION)
        .query(&q)
        .send()
        .map_err(|e| FullError::Track(format!("playback info: {}", e.without_url())))?;
    let status = resp.status();
    let v: Value = resp.json().unwrap_or(Value::Null);
    match status.as_u16() {
        200..=299 => Ok(v),
        404 => Err(FullError::Track("TIDAL has no stream for this track".into())),
        401 | 403 => Err(FullError::Mode(format!(
            "TIDAL refused the private playback ({status}): {}",
            v["userMessage"].as_str().or(v["error"].as_str()).unwrap_or("")
        ))),
        _ => Err(FullError::Track(format!("playback info answered {status}"))),
    }
}

/// python-tidal's request headers for the private API (`tidalapi/request.py`).
pub const PLAYBACK_USER_AGENT: &str = "Mozilla/5.0 (Linux; Android 12; wv) AppleWebKit/537.36 (KHTML, like Gecko) Version/4.0 Chrome/91.0.4472.114 Safari/537.36";

pub const PLAYBACK_CLIENT_VERSION: &str = "2025.7.16";

/// What one playback-info answer says and where its audio is. An encrypted stream is refused,
/// never decrypted.
pub fn read_answer(v: &Value, asked: &str) -> Result<(Answer, Media), FullError> {
    if v["assetPresentation"].as_str().is_some_and(|p| p != "FULL") {
        return Err(FullError::Track(
            "TIDAL sent a preview: the plan may not include this track".into(),
        ));
    }
    let mut answer = Answer {
        asked: asked.into(),
        audio_quality: v["audioQuality"].as_str().unwrap_or_default().into(),
        manifest: v["manifestMimeType"].as_str().unwrap_or_default().into(),
        codecs: String::new(),
        bit_depth: v["bitDepth"].as_u64(),
        sample_rate: v["sampleRate"].as_u64(),
    };
    let manifest = base64::engine::general_purpose::STANDARD
        .decode(v["manifest"].as_str().unwrap_or_default())
        .map_err(|_| FullError::Mode("playback info without a readable manifest".into()))?;
    let manifest = String::from_utf8_lossy(&manifest).into_owned();
    if answer.manifest == "application/vnd.tidal.bts" {
        let m: Value = serde_json::from_str(&manifest)
            .map_err(|_| FullError::Mode("BTS manifest is not JSON".into()))?;
        answer.codecs = m["codecs"].as_str().unwrap_or_default().to_string();
        let enc = m["encryptionType"].as_str().unwrap_or("NONE");
        if enc != "NONE" {
            // Never decrypted: no DRM circumvention
            return Err(FullError::Track(format!(
                "TIDAL sends this track encrypted ({enc})"
            )));
        }
        let url = m["urls"][0]
            .as_str()
            .ok_or_else(|| FullError::Mode("BTS manifest without a URL".into()))?;
        let (ext, mime) = if answer.codecs.to_ascii_lowercase().starts_with("flac") {
            ("flac", "audio/flac")
        } else {
            ("m4a", "audio/mp4")
        };
        return Ok((answer, Media::File(url.to_string(), ext, mime)));
    }
    if answer.manifest == "application/dash+xml" {
        if manifest.contains("ContentProtection") {
            return Err(FullError::Track("TIDAL sends this track encrypted".into()));
        }
        let d = crate::dash::parse(&manifest).map_err(FullError::Mode)?;
        let segments = d.segments();
        answer.codecs = d.codecs;
        if answer.sample_rate.is_none() {
            answer.sample_rate = d.sample_rate.map(u64::from);
        }
        return Ok((answer, Media::Segments(segments)));
    }
    Err(FullError::Mode(format!(
        "unknown manifest type {:?}",
        answer.manifest
    )))
}

// ---- the state in a store --------------------------------------------------------------------

/// This person's full-tracks state from `store` (nothing kept: the default, off).
pub fn load(store: &dyn SessionStore) -> ModeState {
    store
        .get(STATE_KEY)
        .and_then(|v| serde_json::from_str(&v).ok())
        .unwrap_or_default()
}

/// Change this person's state in one step: `f` sees the state there now and answers whether to
/// write it. Answers whether it was written.
pub fn update_if(store: &dyn SessionStore, f: impl FnOnce(&mut ModeState) -> bool) -> bool {
    let mut f = Some(f);
    let mut written = false;
    store.update(STATE_KEY, &mut |now| {
        let mut st: ModeState = now
            .and_then(|v| serde_json::from_str(&v).ok())
            .unwrap_or_default();
        let f = f.take()?;
        if !f(&mut st) {
            return None;
        }
        written = true;
        Some(serde_json::to_string(&st).expect("state serializes"))
    });
    written
}

/// [`update_if`] that always writes.
pub fn update(store: &dyn SessionStore, f: impl FnOnce(&mut ModeState)) {
    update_if(store, |st| {
        f(st);
        true
    });
}

// ---- sign-in steps (pure HTTP; the caller keeps what they answer) -----------------------------

/// Who the session is: TIDAL's `/sessions` (user, country, session id). Unchanged when TIDAL
/// doesn't answer.
pub async fn fill_session(http: &reqwest::Client, v1_base: &str, mut sess: Session) -> Session {
    if let Ok(r) = http
        .get(format!("{}/sessions", v1_base.trim_end_matches('/')))
        .bearer_auth(&sess.access_token)
        .send()
        .await
    {
        if let Ok(v) = r.json::<Value>().await {
            sess.user_id = v["userId"].as_i64().map(|u| u.to_string()).or(sess.user_id);
            if let Some(c) = v["countryCode"].as_str() {
                sess.country = c.to_string();
            }
            sess.session_id = v["sessionId"].as_str().map(str::to_string).or(sess.session_id);
        }
    }
    sess
}

/// The code sign-in's first step: a device code for link.tidal.com.
pub async fn device_authorization(
    http: &reqwest::Client,
    auth_base: &str,
    c: &Client,
) -> Result<Pending, SourceError> {
    let resp = http
        .post(format!(
            "{}/v1/oauth2/device_authorization",
            auth_base.trim_end_matches('/')
        ))
        .header(reqwest::header::CONTENT_TYPE, "application/x-www-form-urlencoded")
        .body(form(&[("client_id", c.id.as_str()), ("scope", SCOPE)]))
        .send()
        .await
        .map_err(|e| SourceError::new(StatusCode::BAD_GATEWAY, format!("TIDAL: {e}")))?;
    let status = resp.status();
    let v: Value = resp.json().await.unwrap_or(Value::Null);
    if !status.is_success() {
        let detail = v["error_description"]
            .as_str()
            .or(v["error"].as_str())
            .unwrap_or("");
        // An app from TIDAL's developer portal is not a device client: TIDAL answers 400
        // "Client is not a Limited Input Device client" to the device-code request.
        if detail.to_ascii_lowercase().contains("limited input device") {
            return Err(SourceError::bad_request(NOT_A_DEVICE_CLIENT));
        }
        return Err(SourceError::new(
            StatusCode::BAD_GATEWAY,
            format!("TIDAL refused the sign-in ({status}): {detail}"),
        ));
    }
    let uri = v["verificationUriComplete"]
        .as_str()
        .or(v["verificationUri"].as_str())
        .unwrap_or("link.tidal.com")
        .to_string();
    let pending = Pending {
        device_code: v["deviceCode"].as_str().unwrap_or_default().to_string(),
        user_code: v["userCode"].as_str().unwrap_or_default().to_string(),
        uri: if uri.starts_with("http") {
            uri
        } else {
            format!("https://{uri}")
        },
        expires_at: now() + v["expiresIn"].as_i64().unwrap_or(300),
        interval: v["interval"].as_u64().unwrap_or(2).clamp(1, 30),
    };
    if pending.device_code.is_empty() {
        return Err(SourceError::new(
            StatusCode::BAD_GATEWAY,
            "TIDAL sent no device code",
        ));
    }
    Ok(pending)
}

/// One look at whether a device code was approved.
#[derive(Debug)]
pub enum Approval {
    /// Approved: the new session (not yet filled from `/sessions`); `None` when TIDAL's answer
    /// had no token (the wait ends, nothing is kept).
    Approved(Option<Session>),
    /// Not yet (or TIDAL didn't answer): ask again after the interval.
    Waiting,
    /// The code ran out.
    Expired,
}

/// Asks TIDAL once whether `p` was approved.
pub async fn poll_approval(http: &reqwest::Client, auth_base: &str, c: &Client, p: &Pending) -> Approval {
    let Ok(resp) = http
        .post(format!("{}/v1/oauth2/token", auth_base.trim_end_matches('/')))
        .header(reqwest::header::CONTENT_TYPE, "application/x-www-form-urlencoded")
        .body(form(&[
            ("client_id", c.id.as_str()),
            ("client_secret", c.secret.as_str()),
            ("device_code", p.device_code.as_str()),
            ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
            ("scope", SCOPE),
        ]))
        .send()
        .await
    else {
        return Approval::Waiting;
    };
    let ok = resp.status().is_success();
    let v: Value = resp.json().await.unwrap_or(Value::Null);
    if ok {
        return Approval::Approved(session_from(&v, None));
    }
    if v["error"].as_str() == Some("expired_token") {
        return Approval::Expired;
    }
    Approval::Waiting
}

/// Sign in with TIDAL's last step: the code from the pasted address for tokens (python-tidal's
/// `pkce_get_auth_token`: no client secret in the exchange). The session is a browser one, with
/// the user and country TIDAL's answer names (not yet filled from `/sessions`).
pub async fn exchange_browser_code(
    http: &reqwest::Client,
    auth_base: &str,
    c: &BrowserClient,
    p: &BrowserPending,
    code: &str,
) -> Result<Session, SourceError> {
    let resp = http
        .post(format!("{}/v1/oauth2/token", auth_base.trim_end_matches('/')))
        .header(reqwest::header::CONTENT_TYPE, "application/x-www-form-urlencoded")
        .body(form(&[
            ("code", code),
            ("client_id", c.id.as_str()),
            ("grant_type", "authorization_code"),
            ("redirect_uri", PKCE_REDIRECT),
            ("scope", PKCE_SCOPE),
            ("code_verifier", p.code_verifier.as_str()),
            ("client_unique_key", p.client_unique_key.as_str()),
        ]))
        .send()
        .await
        .map_err(|e| {
            SourceError::new(
                StatusCode::BAD_GATEWAY,
                format!("Couldn't reach TIDAL: {}", e.without_url()),
            )
        })?;
    let status = resp.status();
    let v: Value = resp.json().await.unwrap_or(Value::Null);
    if !status.is_success() {
        if status.as_u16() == 429 || status.is_server_error() {
            return Err(SourceError::new(
                StatusCode::BAD_GATEWAY,
                format!("TIDAL couldn't answer right now ({status}). Paste the address again in a moment."),
            ));
        }
        let why = v["error_description"]
            .as_str()
            .or(v["error"].as_str())
            .unwrap_or("")
            .chars()
            .take(200)
            .collect::<String>();
        return Err(SourceError::bad_request(format!(
            "TIDAL didn't accept that sign-in code ({status}{}). A code works once and only for a few minutes: press Sign in with TIDAL to get a new one.",
            if why.is_empty() { String::new() } else { format!(": {why}") }
        )));
    }
    let Some(mut sess) = session_from(&v, None) else {
        return Err(SourceError::new(
            StatusCode::BAD_GATEWAY,
            "TIDAL's answer had no access token",
        ));
    };
    sess.client = ClientKind::Pkce;
    // The token answer names the user; /sessions adds the session id and country
    sess.user_id = v["user_id"]
        .as_i64()
        .or(v["user"]["userId"].as_i64())
        .map(|u| u.to_string());
    if let Some(cc) = v["user"]["countryCode"].as_str() {
        sess.country = cc.to_string();
    }
    Ok(sess)
}

// ---- playback (the audio thread and the browser proxy) -----------------------------------------

/// Where TIDAL's private API is, and the clients this side renews sessions with.
pub struct Endpoints<'a> {
    pub auth_base: &'a str,
    pub v1_base: &'a str,
    /// The browser sign-in's client (this build's, or a test's).
    pub browser: Option<&'a BrowserClient>,
    /// The code sign-in's device client, when this side has one.
    pub device: Option<Client>,
    /// What a missing device client says (where this side's person sets one up).
    pub no_device: &'a str,
}

/// This person's session and the `audioquality` to ask for, renewed first when it is about to
/// run out. `Ok(None)`: the mode is off or not signed in.
pub fn token(
    store: &dyn SessionStore,
    client: &reqwest::blocking::Client,
    ep: &Endpoints<'_>,
) -> Result<Option<(Session, String)>, FullError> {
    let st = load(store);
    if !st.enabled {
        return Ok(None);
    }
    let Some(sess) = st.session.clone() else {
        return Ok(None);
    };
    let quality = quality_of(&st).to_string();
    if sess.expires_at - now() > 60 {
        return Ok(Some((sess, quality)));
    }
    let fresh = renew(client, ep, &sess)?;
    // Only over the session this renewal started from (the vault may have changed it
    // meanwhile); otherwise the session there now is the one to use
    let mut there_now = None;
    let kept = update_if(store, |st| {
        if st
            .session
            .as_ref()
            .is_some_and(|c| c.access_token == sess.access_token)
        {
            st.session = Some(fresh.clone());
            true
        } else {
            there_now = Some(st.session.clone());
            false
        }
    });
    if !kept {
        tracing::info!("a renewed TIDAL full-tracks session was dropped: the sign-in changed meanwhile");
        return Ok(there_now.flatten().map(|c| (c, quality)));
    }
    Ok(Some((fresh, quality)))
}

/// Renews `sess` at TIDAL with the client that made it (python-tidal's `token_refresh`: the PKCE
/// pair for a browser sign-in, no scope; the device client with the scope for a code sign-in).
/// Only the HTTP step: where the new session is kept is the caller's.
pub fn renew(
    client: &reqwest::blocking::Client,
    ep: &Endpoints<'_>,
    sess: &Session,
) -> Result<Session, FullError> {
    renew_within(client, ep, sess, None)
}

/// [`renew`], the request (its answer read whole) given at most `limit` instead of the client's
/// own: a renewal under the vault's lock must end before the vault stops waiting for it, or its
/// new tokens are thrown away while the request still runs.
pub fn renew_within(
    client: &reqwest::blocking::Client,
    ep: &Endpoints<'_>,
    sess: &Session,
    limit: Option<std::time::Duration>,
) -> Result<Session, FullError> {
    let auth = ep.auth_base.trim_end_matches('/');
    let Some(refresh) = sess.refresh_token.clone() else {
        return Err(FullError::Mode("the session ran out: sign in again".into()));
    };
    // Renewed with the client that made it: python-tidal's `token_refresh` (the PKCE pair for a
    // browser sign-in, no scope), the device client with the scope for a code sign-in
    let body = match sess.client {
        ClientKind::Pkce => {
            let c = ep.browser.ok_or_else(|| {
                FullError::Mode(
                    "this build has no client for Sign in with TIDAL, so it can't renew that sign-in: sign in again with the code sign-in"
                        .into(),
                )
            })?;
            form(&[
                ("grant_type", "refresh_token"),
                ("refresh_token", refresh.as_str()),
                ("client_id", c.id.as_str()),
                ("client_secret", c.secret.as_str()),
            ])
        }
        ClientKind::Device => {
            let c = ep
                .device
                .as_ref()
                .ok_or_else(|| FullError::Mode(ep.no_device.to_string()))?;
            form(&[
                ("grant_type", "refresh_token"),
                ("refresh_token", refresh.as_str()),
                ("client_id", c.id.as_str()),
                ("client_secret", c.secret.as_str()),
                ("scope", SCOPE),
            ])
        }
    };
    let mut req = client
        .post(format!("{auth}/v1/oauth2/token"))
        .header(reqwest::header::CONTENT_TYPE, "application/x-www-form-urlencoded")
        .body(body);
    if let Some(limit) = limit {
        req = req.timeout(limit);
    }
    let resp = req
        .send()
        .map_err(|e| FullError::Track(format!("renewing the session: {}", e.without_url())))?;
    let status = resp.status();
    let v = resp.json::<Value>();
    if !status.is_success() {
        let v = v.unwrap_or(Value::Null);
        return Err(FullError::Mode(format!(
            "TIDAL did not renew the session ({status}): {}",
            v["error_description"]
                .as_str()
                .or(v["error"].as_str())
                .unwrap_or("")
        )));
    }
    // TIDAL said yes, and its answer didn't arrive whole (cut off, or past the time limit): it
    // may have renewed, so not a refusal (that would ask the person to sign in again)
    let v = v.map_err(|e| {
        FullError::Track(format!(
            "renewing the session: TIDAL's answer didn't arrive whole: {}",
            e.without_url()
        ))
    })?;
    session_from(&v, Some(sess)).ok_or_else(|| FullError::Mode("renewal without a token".into()))
}

/// Open the full track through the private playback endpoint. `Ok(None)`: the mode is off or
/// not signed in (use the official path). AAC where HI_RES_LOSSLESS was asked for is asked once
/// more as LOSSLESS before it is played; one line in the log says what TIDAL answered.
pub fn open(
    store: &dyn SessionStore,
    client: &'static reqwest::blocking::Client,
    ep: &Endpoints<'_>,
    id: &str,
) -> Result<Option<FullStream>, FullError> {
    let Some((sess, quality)) = token(store, client, ep)? else {
        return Ok(None);
    };
    open_as(client, ep, &sess, &quality, id).map(Some)
}

/// The quality TIDAL is asked for, by the mode's setting (`high`: CD quality; else the best).
pub fn quality_of(st: &ModeState) -> &'static str {
    if st.quality == "high" {
        "LOSSLESS"
    } else {
        "HI_RES_LOSSLESS"
    }
}

/// [`open`] with `sess` as it is (the caller renewed it, or keeps it while another device renews
/// it): never renews.
pub fn open_as(
    client: &'static reqwest::blocking::Client,
    ep: &Endpoints<'_>,
    sess: &Session,
    quality: &str,
    id: &str,
) -> Result<FullStream, FullError> {
    let v1 = ep.v1_base.trim_end_matches('/');
    let quality = quality.to_string();
    let sess = sess.clone();
    let v = playback_info(client, v1, &sess, id, &quality)?;
    let (mut answer, mut media) = read_answer(&v, &quality)?;
    let mut first = None;
    if quality == "HI_RES_LOSSLESS" && answer.lossy() {
        let again =
            playback_info(client, v1, &sess, id, "LOSSLESS").and_then(|v| read_answer(&v, "LOSSLESS"));
        match again {
            Ok((a, m)) => {
                first = Some(format!("{answer}"));
                (answer, media) = (a, m);
            }
            Err(e) => {
                tracing::info!(
                    "TIDAL full track {id}: asking LOSSLESS after AAC failed ({}); playing the AAC answer",
                    match &e {
                        FullError::Mode(x) | FullError::Track(x) => x.as_str(),
                    }
                );
            }
        }
    }
    tracing::info!(
        "TIDAL full track {id} ({}): asked {}, TIDAL answered {answer}{}",
        sess.client.words(),
        answer.asked,
        first
            .map(|f| format!(" (HI_RES_LOSSLESS had answered {f})"))
            .unwrap_or_default()
    );
    let reader: Box<dyn std::io::Read + Send> = match &media {
        Media::File(url, ..) => Box::new(
            client
                .get(url)
                .send()
                .and_then(|r| r.error_for_status())
                .map_err(|e| FullError::Track(format!("stream: {}", e.without_url())))?,
        ),
        Media::Segments(segs) => Box::new(crate::segmented::SegmentReader::new(client, segs.urls())),
    };
    let (ext, mime, segments) = match media {
        Media::File(_, ext, mime) => (ext, mime, None),
        Media::Segments(segs) => ("m4a", "audio/mp4", Some(segs)),
    };
    Ok(FullStream {
        reader,
        segments,
        ext,
        mime,
        lossy: answer.lossy(),
        client: sess.client,
        imported: sess.from.is_some(),
        browser: ep.browser.is_some(),
        answer,
    })
}

/// Record how the mode did: a failure of the mode itself switches it off (the page says why;
/// Retry switches it on again).
pub fn record(store: &dyn SessionStore, r: &Result<Option<FullStream>, FullError>) {
    update_if(store, |st| {
        match r {
            Ok(Some(f)) => {
                st.health = Some(Health {
                    ok: true,
                    detail: f.health_line().into(),
                    at: now(),
                    lossy: f.lossy,
                });
            }
            Err(FullError::Mode(e)) => {
                tracing::warn!("TIDAL full tracks (unofficial) switched off: {e}");
                st.enabled = false;
                st.health = Some(Health {
                    ok: false,
                    detail: e.clone(),
                    at: now(),
                    lossy: false,
                });
            }
            _ => return false,
        }
        true
    });
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UnofficialStatus {
    pub service: String,
    pub enabled: bool,
    pub signed_in: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user: Option<String>,
    /// The app the sign-in was imported from, when it was.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from: Option<String>,
    pub quality: String,
    /// A sign-in waiting for the code to be entered.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pending: Option<PendingView>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub health: Option<Health>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_from: Option<String>,
    /// The client the session in use belongs to: `pkce` (Sign in with TIDAL) or `device`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_client: Option<String>,
    /// This build can sign in with TIDAL in the browser (it carries that client).
    pub browser_available: bool,
    /// A browser sign-in waiting for its pasted address, until then (Unix seconds).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub browser_pending: Option<i64>,
    pub warning: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PendingView {
    pub user_code: String,
    pub uri: String,
    pub expires_at: i64,
}

/// What the TIDAL full-tracks page shows for this state. `client_from`: where the code sign-in's
/// device client comes from (`settings`, `config`, `builtin`), if there is one.
pub fn status(st: &ModeState, client_from: Option<String>, browser_available: bool) -> UnofficialStatus {
    UnofficialStatus {
        service: "tidal".into(),
        enabled: st.enabled,
        signed_in: st.session.is_some(),
        user: st.session.as_ref().and_then(|x| x.user_id.clone()),
        from: st.session.as_ref().and_then(|x| x.from.clone()),
        quality: if st.quality.is_empty() {
            "max".into()
        } else {
            st.quality.clone()
        },
        pending: st
            .login
            .as_ref()
            .filter(|p| p.expires_at > now())
            .map(|p| PendingView {
                user_code: p.user_code.clone(),
                uri: p.uri.clone(),
                expires_at: p.expires_at,
            }),
        health: st.health.clone(),
        client_from,
        session_client: st.session.as_ref().map(|x| {
            match x.client {
                ClientKind::Pkce => "pkce",
                ClientKind::Device => "device",
            }
            .to_string()
        }),
        browser_available,
        browser_pending: st.browser.as_ref().map(|b| b.expires_at).filter(|e| *e > now()),
        warning: WARNING.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::MemoryStore;
    use std::sync::{Arc, Mutex};

    /// TIDAL's token endpoint on a thread of its own (the renewal is a blocking call): answers a
    /// new access token, records each form, and first runs `meanwhile` (another device's write).
    fn token_endpoint(meanwhile: Arc<dyn Fn() + Send + Sync>) -> (String, Arc<Mutex<Vec<String>>>) {
        let forms = Arc::new(Mutex::new(Vec::new()));
        let f2 = forms.clone();
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap();
            rt.block_on(async move {
                let app = axum::Router::new().route(
                    "/v1/oauth2/token",
                    axum::routing::post(move |body: String| {
                        let (f2, meanwhile) = (f2.clone(), meanwhile.clone());
                        async move {
                            f2.lock().unwrap().push(body);
                            meanwhile();
                            axum::Json(serde_json::json!({"access_token": "renewed", "expires_in": 3600}))
                        }
                    }),
                );
                let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
                tx.send(format!("http://{}", l.local_addr().unwrap())).unwrap();
                axum::serve(l, app).await.unwrap();
            });
        });
        (rx.recv().unwrap(), forms)
    }

    fn signed_in(store: &MemoryStore, token: &str, expires_at: i64) {
        update(store, |st| {
            st.enabled = true;
            st.session = Some(Session {
                access_token: token.into(),
                refresh_token: Some("refresh-1".into()),
                expires_at,
                country: "CH".into(),
                ..Default::default()
            });
        });
    }

    fn endpoints(base: &str) -> Endpoints<'_> {
        Endpoints {
            auth_base: base,
            v1_base: base,
            browser: None,
            device: Some(Client {
                id: "device-id".into(),
                secret: "device-secret".into(),
                from: "builtin".into(),
            }),
            no_device: "no device client",
        }
    }

    #[test]
    fn a_session_about_to_run_out_is_renewed_with_its_client_and_kept() {
        let (base, forms) = token_endpoint(Arc::new(|| {}));
        let store = MemoryStore::default();
        signed_in(&store, "old", now() + 30);
        let client = reqwest::blocking::Client::new();
        let (sess, quality) = token(&store, &client, &endpoints(&base)).unwrap().unwrap();
        assert_eq!(sess.access_token, "renewed");
        assert_eq!(
            sess.refresh_token.as_deref(),
            Some("refresh-1"),
            "TIDAL kept the refresh token"
        );
        assert_eq!(quality, "HI_RES_LOSSLESS");
        assert_eq!(load(&store).session.unwrap().access_token, "renewed");
        let form = forms.lock().unwrap().join("&");
        assert!(
            form.contains("grant_type=refresh_token") && form.contains("client_id=device-id"),
            "{form}"
        );
        // Not due: nothing asked
        token(&store, &client, &endpoints(&base)).unwrap().unwrap();
        assert_eq!(forms.lock().unwrap().len(), 1);
    }

    #[test]
    fn a_sign_in_written_meanwhile_wins_over_the_renewal() {
        let store = Arc::new(MemoryStore::default());
        signed_in(&store, "old", now() + 30);
        let s2 = store.clone();
        let (base, _) = token_endpoint(Arc::new(move || signed_in(&s2, "from-the-vault", now() + 3600)));
        let client = reqwest::blocking::Client::new();
        let (sess, _) = token(store.as_ref(), &client, &endpoints(&base))
            .unwrap()
            .unwrap();
        assert_eq!(sess.access_token, "from-the-vault");
        assert_eq!(
            load(store.as_ref()).session.unwrap().access_token,
            "from-the-vault"
        );
    }
}
