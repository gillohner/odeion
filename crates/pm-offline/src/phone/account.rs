//! The person's Pubky account in phone mode: the Ring session the app kept at sign-in
//! (`phone/pubky-session`, readable by the app alone), restored when the homeserver answers. Phone
//! mode works without it (the phone keeps what it has); writes wait for it.

use pm_pubky::homeserver::{NodeHomeserver, PubkySession};
use std::path::Path;

/// Where the app keeps the session's secret (`pm-desktop/src/phone.rs` writes it).
pub const SESSION_FILE: &str = "pubky-session";
/// The most restoring the session may take: the homeserver client has no time limit of its own,
/// and phone mode's sync waits for this first.
pub const RESTORE_FOR: std::time::Duration = std::time::Duration::from_secs(60);

pub struct Account {
    pub hs: NodeHomeserver,
    pub session: PubkySession,
    /// The person's Pubky (z32).
    pub pubky: String,
}

impl Account {
    /// The session from `dir`, restored on the homeserver; `Ok(None)` when the app has none.
    pub async fn restore(dir: &Path) -> Result<Option<Self>, String> {
        let Ok(secret) = std::fs::read_to_string(dir.join(SESSION_FILE)) else {
            return Ok(None);
        };
        let secret = secret.trim();
        if secret.is_empty() {
            return Ok(None);
        }
        let hs = NodeHomeserver::new(pm_pubky::Network::Mainnet).map_err(|e| e.to_string())?;
        Self::restore_with(hs, secret).await.map(Some)
    }

    /// The session from `secret` on `hs` (tests: a testnet), within [`RESTORE_FOR`]. Given up
    /// part way, nothing is lost: the secret stays where it is, and the next try starts again.
    pub async fn restore_with(hs: NodeHomeserver, secret: &str) -> Result<Self, String> {
        let session = tokio::time::timeout(RESTORE_FOR, hs.restore_session(secret))
            .await
            .map_err(|_| "Pubky: the homeserver didn't answer in time".to_string())?
            .map_err(|e| format!("Pubky: {e}"))?;
        Ok(Self::of(hs, session))
    }

    /// An account from a session already in hand.
    pub fn of(hs: NodeHomeserver, session: PubkySession) -> Self {
        let pubky = session.public_key().to_string();
        Self { hs, session, pubky }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn a_homeserver_that_never_answers_doesnt_hold_the_restore() {
        let s = super::super::stalled::homeserver().await;
        tokio::time::pause();
        let r = tokio::time::timeout(
            std::time::Duration::from_secs(3600),
            Account::restore_with(s.hs.clone(), &s.secret),
        )
        .await
        .expect("the restore ends by itself");
        assert!(r.is_err());
        assert!(s.taken.load(std::sync::atomic::Ordering::SeqCst) > 0);
    }
}
