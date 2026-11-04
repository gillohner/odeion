//! The Pubky Ring sign-in while Odeion is off screen.
//!
//! The user leaves Odeion for Pubky Ring to approve. Android 15 and later cut a backgrounded app
//! off the network a few seconds after it leaves the screen (every app, whatever its target SDK).
//! A grant exchange made then cannot look up the user's homeserver: pubky 0.16.2 then sends the
//! grant to the raw key (`https://<key>/auth/grant/session` … "dns error <- pkarr could not resolve
//! host") and keeps that choice for 60 s. The relay poll that waits for the approval gives up the
//! same way (three failed polls).
//!
//! So [`approval`] exchanges the grant only while the app is on screen and the network answers;
//! until then the page says to come back. (The SDK's background listener may still take Ring's
//! approval from the relay while the app is away; only the exchange waits.) A relay poll that
//! died while the app was away listens again on the same relay inbox, where the approval waits
//! until it is read (no grant has been sent then). Nothing is retried after a grant was sent.
//! Only Android reports leaving the screen; on desktop the wait is skipped.

// Only Android reports leaving the screen; elsewhere the app is always "on screen".
#![cfg_attr(not(target_os = "android"), allow(dead_code))]

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;

static AWAY: AtomicBool = AtomicBool::new(false);
/// How often the app has left the screen since it started.
static TRIPS: AtomicU64 = AtomicU64::new(0);

/// The activity paused (Android `onPause`).
pub fn left() {
    AWAY.store(true, Ordering::Release);
    TRIPS.fetch_add(1, Ordering::AcqRel);
}

/// The activity resumed (Android `onResume`).
pub fn back() {
    AWAY.store(false, Ordering::Release);
}

pub fn on_screen() -> bool {
    !AWAY.load(Ordering::Acquire)
}

pub fn trips() -> u64 {
    TRIPS.load(Ordering::Acquire)
}

/// What the connect page shows while the sign-in waits for the app to be on screen again.
pub const COME_BACK: &str = "Come back to Odeion to finish signing in";
/// … and while it waits for the network after that.
pub const NO_NETWORK: &str = "Finishing sign-in: waiting for the network…";
/// … and once the app is back and online, as before the wait.
pub const WAITING: &str = "Waiting for Pubky Ring…";

/// Whether the wait applies: on Android (and in this module's tests), not on desktop, where a
/// direct TCP check could fail behind a proxy-only network.
const GATED: bool = cfg!(any(target_os = "android", test));

/// Whether `host:port` takes a TCP connection within 3 s (a backgrounded app on Android 15+ cannot
/// even resolve the name).
pub async fn online(host: &str, port: u16) -> bool {
    matches!(
        tokio::time::timeout(
            Duration::from_secs(3),
            tokio::net::TcpStream::connect((host, port))
        )
        .await,
        Ok(Ok(_))
    )
}

/// Returns once the app is on screen and `host:port` answers. `state` gets [`COME_BACK`] or
/// [`NO_NETWORK`] whenever the wait changes reason.
pub async fn until_ready(host: &str, port: u16, mut state: impl FnMut(&'static str)) {
    let mut shown: Option<&'static str> = None;
    loop {
        let reason = if !on_screen() {
            Some(COME_BACK)
        } else if !online(host, port).await {
            Some(NO_NETWORK)
        } else {
            if shown.is_some() {
                state(WAITING);
            }
            return;
        };
        if reason != shown {
            if let Some(r) = reason {
                state(r);
            }
            shown = reason;
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}

/// The relay host of a `pubkyauth://` link (its `relay=` parameter), or Pubky's default relay.
pub fn relay_host(auth_url: &str) -> String {
    url::Url::parse(auth_url)
        .ok()
        .and_then(|u| {
            u.query_pairs()
                .find(|(k, _)| k == "relay")
                .and_then(|(_, v)| url::Url::parse(&v).ok())
                .and_then(|r| r.host_str().map(str::to_string))
        })
        .unwrap_or_else(|| "httprelay.pubky.app".to_string())
}

/// An error with all its causes (the top one alone hides the URL).
pub fn chain(e: &(dyn std::error::Error + 'static)) -> String {
    let mut s = e.to_string();
    let mut cur = e.source();
    while let Some(c) = cur {
        s.push_str(" <- ");
        s.push_str(&c.to_string());
        cur = c.source();
    }
    s
}

/// The relay poll failed (its request went to the relay), as opposed to the grant exchange.
fn relay_failed(chain: &str, relay: &str) -> bool {
    chain.contains(&format!("://{relay}/")) || chain.contains(&format!("://{relay})"))
}

/// What the page says for an error of the grant exchange.
pub fn explain(chain: &str) -> String {
    if chain.contains("pkarr could not resolve host") || chain.contains("could not be resolved") {
        "Pubky Ring approved, but Odeion could not look up your homeserver (no network?). \
         Check the connection and sign in again."
            .to_string()
    } else {
        chain.to_string()
    }
}

/// A pending Ring sign-in.
pub trait Pending: Sized {
    type Done;
    /// One non-blocking check: takes a delivered approval and exchanges the grant.
    fn poll(&self) -> impl std::future::Future<Output = Result<Option<Self::Done>, String>> + Send + '_;
    /// The same sign-in listening again on its relay inbox (a fresh client).
    fn resume(&self) -> Result<Self, String>;
}

/// Waits for Ring's approval and exchanges the grant only while the app is on screen and online.
/// `since_trip` is [`trips`] when `flow`'s relay listener started; `relay` is the relay host (the
/// network check connects to it on `port`, 443 in the app); `state` updates the page.
pub async fn approval<F: Pending>(
    mut flow: F,
    since_trip: u64,
    relay: &str,
    port: u16,
    mut state: impl FnMut(&'static str),
) -> Result<F::Done, String> {
    // Trips away already covered by the current relay listener.
    let mut listening_since = since_trip;
    // The trip after which the network was last seen answering: checked once per return, not
    // before every poll.
    let mut ready_after: Option<u64> = None;
    loop {
        // The app can leave between this check and the poll: an exchange then starts within the few
        // seconds Android still leaves it the network.
        if GATED && (!on_screen() || ready_after != Some(trips())) {
            until_ready(relay, port, &mut state).await;
            ready_after = Some(trips());
        }
        match flow.poll().await {
            Ok(Some(done)) => return Ok(done),
            Ok(None) => {}
            Err(e) if relay_failed(&e, relay) && trips() > listening_since => {
                tracing::info!("Ring: the relay poll stopped while the app was away; listening again");
                flow = flow.resume()?;
                listening_since = trips();
            }
            Err(e) => return Err(explain(&e)),
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    #[test]
    fn relay_host_comes_from_the_link() {
        assert_eq!(
            relay_host("pubkyauth://signin?caps=/pub/x/:r&relay=https%3A%2F%2Frelay.example%2Finbox&cpk=x"),
            "relay.example"
        );
        assert_eq!(
            relay_host("pubkyauth:///?caps=/pub/x/:r&secret=abc"),
            "httprelay.pubky.app"
        );
    }

    #[test]
    fn only_relay_errors_count_as_relay_failures() {
        let relay = "httprelay.pubky.app";
        assert!(relay_failed(
            "error sending request for url (https://httprelay.pubky.app/inbox/abc) <- dns error",
            relay
        ));
        assert!(!relay_failed(
            "error sending request for url (https://p815irkx17jo489ba9ym56kb5zmcuf385w88jz8ncq5b38t7k1ay/auth/grant/session) <- client error (Connect) <- dns error <- pkarr could not resolve host",
            relay
        ));
    }

    #[test]
    fn the_raw_key_error_reads_as_a_lookup_failure() {
        let e = "error sending request for url (https://p815/auth/grant/session) <- dns error <- pkarr could not resolve host";
        assert!(explain(e).starts_with("Pubky Ring approved, but Odeion could not look up"));
        assert_eq!(explain("403 forbidden"), "403 forbidden");
    }

    /// A sign-in whose first relay listener died; the next one delivers.
    struct Fake {
        generation: u32,
        polls: std::sync::Arc<Mutex<Vec<bool>>>,
        dies: bool,
    }

    impl Pending for Fake {
        type Done = u32;
        async fn poll(&self) -> Result<Option<u32>, String> {
            self.polls.lock().unwrap().push(on_screen());
            if self.generation == 0 || self.dies {
                Err("error sending request for url (https://127.0.0.1/inbox/x) <- dns error".into())
            } else {
                Ok(Some(7))
            }
        }
        fn resume(&self) -> Result<Self, String> {
            Ok(Fake {
                generation: self.generation + 1,
                polls: self.polls.clone(),
                dies: self.dies,
            })
        }
    }

    /// One test drives the shared flags (tests in one binary share them).
    #[tokio::test(flavor = "multi_thread")]
    async fn the_approval_waits_for_the_screen_and_a_dead_relay_poll_listens_again() {
        // The "relay": a local port that takes connections.
        let relay = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = relay.local_addr().unwrap().port();
        tokio::spawn(async move { while relay.accept().await.is_ok() {} });
        let polls = std::sync::Arc::new(Mutex::new(Vec::<bool>::new()));
        // The listener starts on screen, then the user leaves for Ring: nothing may be polled
        // until the app is back.
        let since = trips();
        left();
        let flow = Fake {
            generation: 0,
            polls: polls.clone(),
            dies: false,
        };
        let task = tokio::spawn(async move {
            let states = std::sync::Arc::new(Mutex::new(Vec::new()));
            let st = states.clone();
            let r = approval(flow, since, "127.0.0.1", port, move |s| {
                st.lock().unwrap().push(s)
            })
            .await;
            let states = states.lock().unwrap().clone();
            (r, states)
        });
        tokio::time::sleep(Duration::from_millis(1200)).await;
        assert!(polls.lock().unwrap().is_empty(), "polled while away");
        back();
        let (r, states) = tokio::time::timeout(Duration::from_secs(10), task)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(r, Ok(7), "listens again after the relay poll died while away");
        assert_eq!(polls.lock().unwrap().as_slice(), &[true, true]);
        assert_eq!(states.first(), Some(&COME_BACK));
        assert_eq!(states.last(), Some(&WAITING), "the page text is reset once back");

        // On screen and no trip away since: a relay error is final (no second listen).
        let flow = Fake {
            generation: 1,
            polls,
            dies: true,
        };
        let r = tokio::time::timeout(
            Duration::from_secs(10),
            approval(flow, trips(), "127.0.0.1", port, |_| {}),
        )
        .await
        .unwrap();
        assert!(r.is_err());
    }
}
