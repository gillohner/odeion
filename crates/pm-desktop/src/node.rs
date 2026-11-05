//! The few node calls the shell makes itself; everything else is the web UI.

use crate::settings::Settings;
use serde::Deserialize;
use std::time::Duration;

fn client() -> reqwest::Result<reqwest::Client> {
    reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(4))
        .timeout(Duration::from_secs(8))
        .user_agent(concat!("pubky-music-desktop/", env!("CARGO_PKG_VERSION")))
        .build()
}

fn api(s: &Settings, path: &str) -> String {
    format!("{}/v1/{path}", s.base().trim_end_matches('/'))
}

/// What the node says about itself.
#[derive(Debug, Deserialize, Default)]
pub struct Info {
    #[serde(default)]
    pub name: String,
    /// Stable node id (nodes before LAN discovery have none).
    #[serde(default)]
    pub id: String,
}

/// Why a probe failed.
#[derive(Debug)]
pub enum ProbeError {
    /// Nothing answered at that address (maybe the node moved).
    Unreachable(String),
    /// The node answered and wants this device signed in first (no token yet).
    NeedsSignIn,
    /// The node answered and refused the device's sign-in: it ended there (Make a new token, the
    /// node signed in as someone else). The app signs in again.
    Refused,
    /// The node answered and said no, or something else went wrong.
    Other(String),
}

/// What [`ProbeError::Refused`] reads as; the connect page's reconnect doesn't wait on it.
pub const REFUSED: &str = "The node refused the token";
/// What [`ProbeError::NeedsSignIn`] reads as.
pub const NEEDS_SIGN_IN: &str = "Sign in to the node with Pubky Ring";

impl From<ProbeError> for String {
    fn from(e: ProbeError) -> String {
        match e {
            ProbeError::Unreachable(m) | ProbeError::Other(m) => m,
            ProbeError::NeedsSignIn => NEEDS_SIGN_IN.into(),
            ProbeError::Refused => REFUSED.into(),
        }
    }
}

/// Check the node answers and accepts the token; returns its name and id.
pub async fn probe(s: &Settings) -> Result<Info, ProbeError> {
    let resp = client()
        .map_err(|e| ProbeError::Other(e.to_string()))?
        .get(api(s, "info"))
        .bearer_auth(&s.token)
        .send()
        .await
        .map_err(|e| {
            if e.is_timeout() || e.is_connect() {
                ProbeError::Unreachable(format!("Can't reach {}", s.node_url))
            } else {
                ProbeError::Unreachable(format!("Can't reach {}: {e}", s.node_url))
            }
        })?;
    match resp.status().as_u16() {
        200..=299 => Ok(resp.json::<Info>().await.unwrap_or_default()),
        401 | 403 if s.token.is_empty() => Err(ProbeError::NeedsSignIn),
        401 | 403 => Err(ProbeError::Refused),
        code => Err(ProbeError::Other(format!("The node answered {code}"))),
    }
}

/// The node's own Pubky Ring sign-in, as the web page uses it (`/v1/auth/pubky/start`).
#[derive(Debug, Deserialize)]
pub struct SignInStarted {
    pub id: String,
    /// `pubkyauth://…`, for Ring.
    pub url: String,
    /// Whose node it says it is (z32).
    #[serde(default)]
    pub owner: String,
}

/// `/v1/auth/pubky/poll`'s answer.
#[derive(Debug, Default, Deserialize)]
struct SignInStatus {
    status: String,
    #[serde(default)]
    token: Option<String>,
    #[serde(default)]
    reason: Option<String>,
    /// Who approved (z32).
    #[serde(default)]
    user: Option<String>,
    /// `owner` or `listener`.
    #[serde(default)]
    role: Option<String>,
}

/// A z32 key without the `pubky` prefix some write it with.
fn bare(key: &str) -> &str {
    key.trim().trim_start_matches("pubky")
}

/// The sign-in's end, for the owner's app: `Ok(Some(token))` only when the node's owner approved
/// (role `owner`, the key the node named at the start). Anyone else's approval, a listener's
/// included, is refused: the app is the owner's, and a credential of someone else's would show
/// their library and keep their sign-ins.
fn accepted(st: &SignInStatus, owner: &str) -> Result<Option<String>, String> {
    match st.status.as_str() {
        "ok" => {
            let who = st.user.as_deref().map(bare).unwrap_or_default();
            if st.role.as_deref() != Some("owner") || who.is_empty() || who != bare(owner) {
                return Err(
                    "That approval wasn't by this node's owner; approve as the owner in Pubky Ring".into(),
                );
            }
            st.token
                .clone()
                .filter(|t| !t.is_empty())
                .map(Some)
                .ok_or_else(|| "The node sent no sign-in".to_string())
        }
        "pending" => Ok(None),
        "denied" => Err(match st.reason.as_deref() {
            Some("no_access") => "That identity has no music access on this node".into(),
            _ => "That isn't the identity this node belongs to".into(),
        }),
        "expired" => Err("The sign-in expired: try again".into()),
        other => Err(format!("The node answered {other}")),
    }
}

/// Starts a Pubky Ring sign-in at the node: this device gets a credential of its own from the
/// node (ADR 0008), never one stored on the homeserver. As a device's, it is kept apart from the
/// browser sessions of the page: the page's Log out and the browsers' cap leave it. With
/// `expect_owner` (the identity whose record named this node), a node that says it belongs to
/// someone else is refused before anything is shown.
pub async fn start_sign_in(s: &Settings, expect_owner: &str) -> Result<SignInStarted, String> {
    let resp = client()
        .map_err(|e| e.to_string())?
        .post(api(s, "auth/pubky/start"))
        .json(&serde_json::json!({"device": true}))
        .send()
        .await
        .map_err(|e| format!("Can't reach the node: {e}"))?;
    if !resp.status().is_success() {
        let code = resp.status().as_u16();
        let body: serde_json::Value = resp.json().await.unwrap_or_default();
        return Err(body["error"]
            .as_str()
            .map(str::to_string)
            .unwrap_or_else(|| format!("The node answered {code}")));
    }
    let started: SignInStarted = resp.json().await.map_err(|e| e.to_string())?;
    if started.owner.trim().is_empty() {
        return Err("The node didn't say whose it is; update the node".into());
    }
    if !expect_owner.trim().is_empty() && bare(&started.owner) != bare(expect_owner) {
        return Err("This node says it belongs to someone else; nothing was sent to Pubky Ring".into());
    }
    ring_request_is_a_sign_in(&started.url)?;
    Ok(started)
}

/// The address the node was reached at isn't tied to its key, so the request it hands back is
/// checked before Ring shows it: a sign-in asks Ring for exactly
/// [`pm_pubky::homeserver::CONTROLLER_CAPABILITIES`] (read the public profile, nothing more).
pub fn ring_request_is_a_sign_in(url: &str) -> Result<(), String> {
    let refuse = || "The node asked Pubky Ring for more than a sign-in; nothing was sent to Ring".to_string();
    let parsed = url::Url::parse(url).map_err(|_| refuse())?;
    if parsed.scheme() != "pubkyauth" {
        return Err(refuse());
    }
    let caps: Vec<String> = parsed
        .query_pairs()
        .filter(|(k, _)| k == "caps")
        .map(|(_, v)| v.into_owned())
        .collect();
    if caps != [pm_pubky::homeserver::CONTROLLER_CAPABILITIES] {
        return Err(refuse());
    }
    Ok(())
}

/// One look at a sign-in started with [`start_sign_in`]: `Ok(Some(token))` once the owner
/// (`started.owner`) approved, `Ok(None)` while it waits (or the node can't be reached for a
/// moment), `Err` when it ended or someone else approved ([`accepted`]).
pub async fn poll_sign_in(s: &Settings, started: &SignInStarted) -> Result<Option<String>, String> {
    let url = reqwest::Url::parse_with_params(&api(s, "auth/pubky/poll"), &[("id", &started.id)])
        .map_err(|e| e.to_string())?;
    let Ok(resp) = client().map_err(|e| e.to_string())?.get(url).send().await else {
        return Ok(None);
    };
    let Ok(st) = resp.json::<SignInStatus>().await else {
        return Ok(None);
    };
    accepted(&st, &started.owner)
}

/// Register this machine's player with its key: one zone per DAC appears in the UI.
#[cfg_attr(any(target_os = "android", target_os = "ios"), allow(dead_code))]
pub async fn add_endpoint(s: &Settings, address: &str, key: &pm_stream::Key) -> Result<(), String> {
    let resp = client()
        .map_err(|e| e.to_string())?
        .post(api(s, "outputs/endpoints"))
        .bearer_auth(&s.token)
        .json(&serde_json::json!({ "address": address, "key": key.expose() }))
        .send()
        .await
        .map_err(|e| e.to_string())?;
    resp.error_for_status().map(|_| ()).map_err(|e| e.to_string())
}

/// Forget an old player address, so the zone list does not fill with ghosts.
#[cfg_attr(any(target_os = "android", target_os = "ios"), allow(dead_code))]
pub async fn remove_endpoint(s: &Settings, address: &str) -> Result<(), String> {
    let enc = address
        .replace('%', "%25")
        .replace('/', "%2F")
        .replace(':', "%3A");
    let resp = client()
        .map_err(|e| e.to_string())?
        .delete(api(s, &format!("outputs/endpoints/{enc}")))
        .bearer_auth(&s.token)
        .send()
        .await
        .map_err(|e| e.to_string())?;
    resp.error_for_status().map(|_| ()).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ring_gets_only_a_sign_in_request() {
        let caps = pm_pubky::homeserver::CONTROLLER_CAPABILITIES;
        let with = |c: &str| {
            let mut u = url::Url::parse("pubkyauth:///").unwrap();
            u.query_pairs_mut()
                .append_pair("relay", "https://httprelay.example/link")
                .append_pair("secret", "AAAA")
                .append_pair("caps", c);
            u.to_string()
        };
        assert!(ring_request_is_a_sign_in(&with(caps)).is_ok());
        for bad in [
            "/:rw",
            "/pub/pubky.app/:rw",
            "/pub/pubky.app/:r,/pub/odeion/:rw",
            "",
        ] {
            assert!(ring_request_is_a_sign_in(&with(bad)).is_err(), "{bad}");
        }
        let twice = format!("{}&caps=%2F%3Arw", with(caps));
        assert!(ring_request_is_a_sign_in(&twice).is_err(), "a second caps");
        let none = with(caps).replace("caps=", "c=");
        assert!(ring_request_is_a_sign_in(&none).is_err(), "no caps");
        let other = with(caps).replace("pubkyauth:", "https:");
        assert!(ring_request_is_a_sign_in(&other).is_err(), "not a Ring request");
    }

    /// Only the owner's approval ends the sign-in with a credential: a listener's `ok`, an owner
    /// role with another key, or no key, is refused; the key may carry the `pubky` prefix.
    #[test]
    fn only_the_owners_approval_signs_the_app_in() {
        const OWNER: &str = "o3ftkuxjd9k6zgnc4hhbhbg5jusubr4zqyb1dtzi8hhm3hf5wbmy";
        const OTHER: &str = "gujx6qd8ksydh1makdphd3bxu351d9b8waqka8hfg6q7hnqkxexo";
        let ok = |user: &str, role: &str| SignInStatus {
            status: "ok".into(),
            token: Some("pmo_t".into()),
            user: Some(user.into()),
            role: Some(role.into()),
            ..Default::default()
        };
        assert_eq!(accepted(&ok(OWNER, "owner"), OWNER), Ok(Some("pmo_t".into())));
        assert_eq!(
            accepted(&ok(&format!("pubky{OWNER}"), "owner"), OWNER),
            Ok(Some("pmo_t".into()))
        );
        assert!(
            accepted(&ok(OTHER, "listener"), OWNER).is_err(),
            "a listener's credential"
        );
        assert!(
            accepted(&ok(OWNER, "listener"), OWNER).is_err(),
            "role says listener"
        );
        assert!(
            accepted(&ok(OTHER, "owner"), OWNER).is_err(),
            "another key as owner"
        );
        assert!(accepted(&ok("", "owner"), OWNER).is_err(), "no key");
        let no_who = SignInStatus {
            status: "ok".into(),
            token: Some("pmo_t".into()),
            ..Default::default()
        };
        assert!(
            accepted(&no_who, OWNER).is_err(),
            "an older node that names nobody"
        );
        assert_eq!(
            accepted(
                &SignInStatus {
                    status: "pending".into(),
                    ..Default::default()
                },
                OWNER
            ),
            Ok(None)
        );
        assert!(accepted(
            &SignInStatus {
                status: "denied".into(),
                ..Default::default()
            },
            OWNER
        )
        .is_err());
    }
}
