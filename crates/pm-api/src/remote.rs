//! Remote access over iroh (ADR 0013). The binary configures it; the tunnel starts when the node
//! has a Pubky session, because only then can the owner's devices learn its key (from the private
//! node record, ADR 0008). Without a session nobody could dial it, so nothing runs.

use std::net::SocketAddr;
use std::path::PathBuf;

use crate::state::NodeState;

/// How the binary wants remote access run.
#[derive(Debug, Clone)]
pub struct RemoteConfig {
    /// The node's iroh key (made on first start, mode 600).
    pub key_path: PathBuf,
    pub relays: pm_remote::Relays,
    /// The tunnel's own loopback listener (`peer::serve_remote`), so its requests are known.
    pub target: SocketAddr,
}

#[derive(Default)]
pub struct Remote {
    config: Option<RemoteConfig>,
    tunnel: Option<pm_remote::NodeTunnel>,
}

impl NodeState {
    /// Turn remote access on (the binary calls this at start when `[remote] enabled`).
    pub async fn configure_remote(&self, config: RemoteConfig) {
        self.remote.lock().await.config = Some(config);
    }

    /// Start the tunnel if remote access is configured and not running yet; its key as a string.
    pub async fn ensure_remote(&self) -> Option<String> {
        let mut r = self.remote.lock().await;
        if let Some(t) = &r.tunnel {
            return Some(t.id().to_string());
        }
        let cfg = r.config.clone()?;
        let key = match pm_remote::load_or_create_key(&cfg.key_path) {
            Ok(k) => k,
            Err(e) => {
                tracing::warn!(path = %cfg.key_path.display(), "remote access off: {e:#}");
                return None;
            }
        };
        match pm_remote::NodeTunnel::start(key, &cfg.relays, cfg.target).await {
            Ok(t) => {
                let id = t.id().to_string();
                tracing::info!(id = %id, "remote access on (iroh)");
                *self.remote_id.write().expect("remote id poisoned") = Some(id.clone());
                r.tunnel = Some(t);
                Some(id)
            }
            Err(e) => {
                tracing::warn!("remote access off: the iroh endpoint did not start: {e:#}");
                None
            }
        }
    }

    /// Where the tunnel is reachable now (its key and addresses), while it runs.
    pub async fn remote_addr(&self) -> Option<pm_remote::EndpointAddr> {
        self.remote.lock().await.tunnel.as_ref().map(|t| t.addr())
    }

    /// Where the node reaches the player of the app with iroh key `peer` (it registered as
    /// `tunnel:<peer>` over the tunnel): a loopback relay into that app's connection, kept for
    /// [`NodeState::add_endpoint_address`].
    pub async fn tunnel_player(&self, peer: &str) -> Result<std::net::SocketAddr, String> {
        let id: pm_remote::EndpointId = peer.trim().parse().map_err(|_| "not an app key".to_string())?;
        let r = self.remote.lock().await;
        let t = r.tunnel.as_ref().ok_or("remote access is off on this node")?;
        let relay = t.player_relay(id).await.map_err(|e| e.to_string())?;
        self.tunnel_players
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(format!("{}{peer}", crate::state::TUNNEL_PREFIX), relay);
        Ok(relay)
    }
}
