//! A UPnP renderer's volume, live (TASK-volume-readback): the node subscribes to the renderer's
//! events (OpenHome `Volume` first, RenderingControl's `LastChange` as the fallback) and each
//! NOTIFY moves the zone's `volume`, so a change made in the Linn app, on its remote or on the
//! device shows at once. The renderer calls `/v1/upnp/events/{token}` (no bearer token, LAN only);
//! the token is random per subscription and names the zone.
//!
//! While a zone's subscription works, the 10 s volume poll leaves it alone (`Following::live`).

use crate::state::NodeState;
use axum::extract::{Path, State};
use axum::http::{Method, StatusCode};
use pm_out_upnp::gena::{self, Source};
use pm_zones::{Zone, ZoneId};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

struct Sub {
    zone: ZoneId,
    source: Source,
    /// OpenHome's `VolumeMax` as last told (100 until then).
    volume_max: u32,
}

/// The subscriptions this node holds, by callback token.
#[derive(Default)]
pub struct Gena {
    subs: Mutex<HashMap<String, Sub>>,
}

/// Whether a zone's volume arrives by event right now (the poll then leaves it alone).
#[derive(Default)]
pub struct Following {
    pub live: AtomicBool,
}

/// `NOTIFY /v1/upnp/events/{token}`: a renderer's event. Always 200 for a known token (a renderer
/// that gets an error may drop the subscription); 412 for one the node doesn't hold, as GENA says.
pub(crate) async fn notify(
    State(s): State<Arc<NodeState>>,
    Path(token): Path<String>,
    method: Method,
    body: String,
) -> StatusCode {
    if method.as_str() != "NOTIFY" {
        return StatusCode::METHOD_NOT_ALLOWED;
    }
    let (zone, change) = {
        let mut subs = s.gena.subs.lock().expect("gena subs");
        let Some(sub) = subs.get_mut(&token) else {
            return StatusCode::PRECONDITION_FAILED;
        };
        let c = gena::read_notify(sub.source, &body, sub.volume_max);
        if let Some(m) = c.volume_max {
            sub.volume_max = m;
        }
        (sub.zone.clone(), c)
    };
    if let Some(z) = s.zones.get(&zone) {
        apply(&z, change);
    }
    StatusCode::OK
}

/// Merge what an event said into the zone's volume: a field it didn't carry keeps its value.
pub(crate) fn apply(z: &Zone, c: gena::VolumeChange) {
    let was = z.state().volume;
    let level = c
        .percent
        .map(pm_model::VolumeLevel::Percent)
        .or(was.map(|w| w.level));
    let Some(level) = level else { return };
    let muted = c.muted.or(was.map(|w| w.muted)).unwrap_or(false);
    z.publish_volume(Some(pm_model::ZoneVolume { level, muted }));
}

/// Follow `zone`'s renderer by event for as long as the zone is in the registry: subscribe,
/// renew at 80 % of what the renderer grants, subscribe again when it forgets the subscription
/// (412), drops it or restarts; back off while it refuses. `following.live` says when it works.
pub(crate) fn follow(
    node: &Arc<NodeState>,
    zone: &Arc<Zone>,
    out: Arc<pm_out_upnp::UpnpOutput>,
    following: Arc<Following>,
) {
    let r = out.renderer();
    let (url, source) = match (&r.events.openhome_volume, &r.events.rendering_control) {
        (Some(u), _) => (u.clone(), Source::OpenHomeVolume),
        (None, Some(u)) => (u.clone(), Source::RenderingControl),
        _ => return,
    };
    let token = format!("{:032x}", rand::random::<u128>());
    let callback = format!("{}/v1/upnp/events/{token}", node.public_url.trim_end_matches('/'));
    node.gena.subs.lock().expect("gena subs").insert(
        token.clone(),
        Sub {
            zone: zone.id().clone(),
            source,
            volume_max: 100,
        },
    );
    let weak = Arc::downgrade(node);
    let zid = zone.id().clone();
    let name = r.friendly_name.clone();
    tokio::spawn(async move {
        let here = || weak.upgrade().is_some_and(|n| n.zones.get(&zid).is_some());
        let mut backoff = Duration::from_secs(30);
        let mut sid: Option<String> = None;
        while here() {
            let (u, cb) = (url.clone(), callback.clone());
            match tokio::task::spawn_blocking(move || gena::subscribe(&u, &cb)).await {
                Ok(Ok(sub)) => {
                    tracing::debug!(renderer = %name, ?source, "following its volume by event");
                    following.live.store(true, Ordering::Relaxed);
                    backoff = Duration::from_secs(30);
                    sid = Some(sub.sid.clone());
                    let mut timeout = sub.timeout;
                    loop {
                        tokio::time::sleep(renew_after(timeout)).await;
                        if !here() {
                            break;
                        }
                        let (u, id) = (url.clone(), sub.sid.clone());
                        match tokio::task::spawn_blocking(move || gena::renew(&u, &id)).await {
                            Ok(Ok(Some(again))) => timeout = again.timeout,
                            // Forgotten (412), refused or unreachable: subscribe afresh
                            _ => break,
                        }
                    }
                    following.live.store(false, Ordering::Relaxed);
                }
                Ok(Err(e)) => {
                    tracing::debug!(renderer = %name, "no volume events ({e}); the poll reads it");
                    following.live.store(false, Ordering::Relaxed);
                    tokio::time::sleep(backoff).await;
                    backoff = (backoff * 2).min(Duration::from_secs(300));
                }
                Err(_) => break,
            }
        }
        following.live.store(false, Ordering::Relaxed);
        if let Some(n) = weak.upgrade() {
            n.gena.subs.lock().expect("gena subs").remove(&token);
        }
        if let Some(id) = sid {
            let _ = tokio::task::spawn_blocking(move || gena::unsubscribe(&url, &id)).await;
        }
    });
}

/// Renew at 80 % of the granted time, never sooner than 5 s (1 s in tests).
fn renew_after(timeout: Duration) -> Duration {
    let floor = if cfg!(test) {
        Duration::from_secs(1)
    } else {
        Duration::from_secs(5)
    };
    (timeout * 4 / 5).max(floor)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renews_before_the_subscription_runs_out() {
        assert_eq!(renew_after(Duration::from_secs(300)), Duration::from_secs(240));
        assert_eq!(
            renew_after(Duration::from_secs(1)),
            Duration::from_secs(1),
            "the floor (5 s outside tests)"
        );
    }
}
