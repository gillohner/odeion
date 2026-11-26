//! TIDAL: what the node and the phone share. [`full`] is full tracks through TIDAL's private API
//! (the code sign-in and Sign in with TIDAL, renewal, playback). The node's official-API source
//! (catalogue, collection, previews) stays in `pm-api`: its sign-in returns to the node's own
//! address.

pub mod catalog;
pub mod full;

pub const SOURCE_ID: &str = "tidal";
pub const TRACK_URI_PREFIX: &str = "tidal:track:";

mod built {
    include!(concat!(env!("OUT_DIR"), "/tidal_clients.rs"));
}

/// The build's TIDAL device client (id, secret) for the code sign-in, if it carries one.
pub fn built_in_device_client() -> Option<(String, String)> {
    crate::client_mask::unmask_pair(built::DEVICE)
}

/// The build's TIDAL browser sign-in client (id, secret; OAuth with PKCE), if it carries one.
pub fn built_in_pkce_client() -> Option<(String, String)> {
    crate::client_mask::unmask_pair(built::PKCE)
}

/// A client id, kept as its SHA-256 only: enough to tell two apart, and nothing to show or log.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct ClientTag(pub [u8; 32]);

impl ClientTag {
    pub fn of(id: &str) -> Self {
        use sha2::Digest;
        ClientTag(sha2::Sha256::digest(id.trim().as_bytes()).into())
    }
}

impl std::fmt::Debug for ClientTag {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ClientTag(..)")
    }
}
