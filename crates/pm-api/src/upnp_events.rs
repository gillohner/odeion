//! A UPnP renderer's volume, live (TASK-volume-readback): the node subscribes to the renderer's
//! events (RenderingControl's `LastChange`, the service the level is set and read through;
//! OpenHome `Volume` when that is all the renderer events) and each NOTIFY moves the zone's
//! `volume`, so a change made in the Linn app, on its remote or on the device shows at once. The
//! renderer calls `/v1/upnp/events/{token}` (no bearer token, LAN only); the token is random per
//! zone and names it.
//!
//! The 10 s volume poll leaves a zone alone once its events arrive (`Following::live`, set by a
//! NOTIFY the node could read, cleared on every new subscription and when one is lost).
//! A renderer that accepts the subscription but whose events never reach the node (a callback
//! behind a proxy, an https address) stays on the poll.

use crate::state::NodeState;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, Method, StatusCode};
use pm_out_upnp::gena::{self, Source};
use pm_zones::Zone;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, Weak};
use std::time::Duration;

struct Sub {
    /// This zone, not whichever zone has its name later (a zone made again under the same name
    /// on another output must not get this renderer's level).
    zone: Weak<Zone>,
    source: Source,
    /// The subscription held now, from the SUBSCRIBE answer; `None` while subscribing (the
    /// renderer's first NOTIFY may come before the answer, and is taken).
    sid: Option<String>,
    /// The last `SEQ` applied for `sid`: an older event arriving late is dropped.
    seq: Option<u32>,
    /// What this subscription published last; an event merges into it, in `SEQ` order.
    last: Option<pm_model::ZoneVolume>,
    following: Arc<Following>,
}

/// The subscriptions this node holds, by callback token.
#[derive(Default)]
pub struct Gena {
    subs: Mutex<HashMap<String, Sub>>,
}

/// Whether a zone's volume arrives by event right now (the poll then leaves it alone): set by a
/// NOTIFY on the current subscription, cleared when a new one starts or one is lost.
#[derive(Default)]
pub struct Following {
    pub live: AtomicBool,
}

/// The zone behind `weak` if it is still the one in the registry under its id.
fn current(node: &NodeState, weak: &Weak<Zone>) -> Option<Arc<Zone>> {
    let z = weak.upgrade()?;
    let now = node.zones.get(z.id())?;
    Arc::ptr_eq(&z, &now).then_some(z)
}

/// `NOTIFY /v1/upnp/events/{token}`: a renderer's event. 200 for the subscription the node
/// holds (a renderer that gets an error may drop it); 412 for a token, SID or zone it doesn't
/// hold any more, as GENA says, so the renderer stops sending.
pub(crate) async fn notify(
    State(s): State<Arc<NodeState>>,
    Path(token): Path<String>,
    method: Method,
    headers: HeaderMap,
    body: String,
) -> StatusCode {
    if method.as_str() != "NOTIFY" {
        return StatusCode::METHOD_NOT_ALLOWED;
    }
    let header = |n: &str| {
        headers
            .get(n)
            .and_then(|v| v.to_str().ok())
            .map(|v| v.trim().to_string())
    };
    let (sid, seq) = (header("sid"), header("seq").and_then(|v| v.parse::<u32>().ok()));
    let weak = match s.gena.subs.lock().expect("gena subs").get(&token) {
        Some(sub) => sub.zone.clone(),
        None => return StatusCode::PRECONDITION_FAILED,
    };
    let Some(z) = current(&s, &weak) else {
        return StatusCode::PRECONDITION_FAILED;
    };
    let mut subs = s.gena.subs.lock().expect("gena subs");
    let Some(sub) = subs.get_mut(&token) else {
        return StatusCode::PRECONDITION_FAILED;
    };
    // An event of a subscription given up (renewal failed, subscribed again). While the
    // SUBSCRIBE is unanswered the SID isn't known yet: the event is taken, its SID not kept
    if let (Some(have), Some(got)) = (&sub.sid, &sid) {
        if have != got {
            return StatusCode::PRECONDITION_FAILED;
        }
    }
    if let Some(n) = seq {
        // 0 is a subscription's first event: the count starts again
        if n != 0 && sub.seq.is_some_and(|prev| n <= prev) {
            return StatusCode::OK;
        }
        sub.seq = Some(n);
    }
    let change = gena::read_notify(sub.source, &body);
    if change.percent.is_none() && change.muted.is_none() {
        // Nothing the node could read (a channel it doesn't follow): the poll stays on
        return StatusCode::OK;
    }
    sub.following.live.store(true, Ordering::Relaxed);
    let was = sub.last.or(z.state().volume);
    if let Some(v) = merge(was, change) {
        sub.last = Some(v);
        // Still under the lock: events reach the zone in the order the renderer sent them (the
        // engine leaves an unchanged level alone)
        z.announce_volume(v);
    }
    StatusCode::OK
}

/// What an event said merged into the volume before it: a field it didn't carry keeps its value.
pub(crate) fn merge(
    was: Option<pm_model::ZoneVolume>,
    c: gena::VolumeChange,
) -> Option<pm_model::ZoneVolume> {
    let level = c
        .percent
        .map(pm_model::VolumeLevel::Percent)
        .or(was.map(|w| w.level))?;
    let muted = c.muted.or(was.map(|w| w.muted)).unwrap_or(false);
    Some(pm_model::ZoneVolume { level, muted })
}

/// Follow `zone`'s renderer by event for as long as the zone is in the registry: subscribe,
/// renew in time (and at least every minute, so a renderer that restarted and forgot the
/// subscription is subscribed again soon), subscribe again when a renewal fails; back off while
/// it refuses. The poll steps aside only while events arrive (`following.live`).
pub(crate) fn follow(
    node: &Arc<NodeState>,
    zone: &Arc<Zone>,
    out: Arc<pm_out_upnp::UpnpOutput>,
    following: Arc<Following>,
) {
    let r = out.renderer();
    let (url, source) = match (&r.events.rendering_control, &r.events.openhome_volume) {
        (Some(u), _) => (u.clone(), Source::RenderingControl),
        (None, Some(u)) => (u.clone(), Source::OpenHomeVolume),
        _ => return,
    };
    let token = format!("{:032x}", rand::random::<u128>());
    let callback = format!("{}/v1/upnp/events/{token}", node.public_url.trim_end_matches('/'));
    let zone_ref = Arc::downgrade(zone);
    node.gena.subs.lock().expect("gena subs").insert(
        token.clone(),
        Sub {
            zone: zone_ref.clone(),
            source,
            sid: None,
            seq: None,
            last: None,
            following: following.clone(),
        },
    );
    let weak = Arc::downgrade(node);
    let name = r.friendly_name.clone();
    tokio::spawn(async move {
        let here = || weak.upgrade().is_some_and(|n| current(&n, &zone_ref).is_some());
        // A new subscription: no event has arrived on it yet
        let start = || {
            following.live.store(false, Ordering::Relaxed);
            if let Some(n) = weak.upgrade() {
                if let Some(sub) = n.gena.subs.lock().expect("gena subs").get_mut(&token) {
                    (sub.sid, sub.seq, sub.last) = (None, None, None);
                }
            }
        };
        let mut backoff = Duration::from_secs(30);
        let mut sid: Option<String> = None;
        while here() {
            start();
            let (u, cb) = (url.clone(), callback.clone());
            match tokio::task::spawn_blocking(move || gena::subscribe(&u, &cb)).await {
                Ok(Ok(sub)) => {
                    tracing::debug!(renderer = %name, ?source, "subscribed to its volume events");
                    if let Some(n) = weak.upgrade() {
                        if let Some(held) = n.gena.subs.lock().expect("gena subs").get_mut(&token) {
                            // From now on only this subscription's events count (one taken before
                            // the answer is most likely its first)
                            held.sid = Some(sub.sid.clone());
                        }
                    }
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
                            // Forgotten (412: restarted), refused or unreachable: subscribe afresh
                            _ => break,
                        }
                    }
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

/// Renew at 80 % of the granted time but at least once a minute (a renderer that restarted has
/// forgotten the subscription; the failed renewal subscribes again, and its first event brings
/// the level), never sooner than 5 s (1 s in tests).
fn renew_after(timeout: Duration) -> Duration {
    let floor = if cfg!(test) {
        Duration::from_secs(1)
    } else {
        Duration::from_secs(5)
    };
    (timeout * 4 / 5).min(Duration::from_secs(60)).max(floor)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renews_before_the_subscription_runs_out() {
        assert_eq!(
            renew_after(Duration::from_secs(300)),
            Duration::from_secs(60),
            "a minute at most"
        );
        assert_eq!(renew_after(Duration::from_secs(50)), Duration::from_secs(40));
        assert_eq!(
            renew_after(Duration::from_secs(1)),
            Duration::from_secs(1),
            "the floor (5 s outside tests)"
        );
    }
}
