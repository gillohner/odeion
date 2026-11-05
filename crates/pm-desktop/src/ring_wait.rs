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
//!
//! A relay that answers 5xx gets one more listen, a few seconds later. The wait ends after
//! [`DEADLINE`] or on the page's Cancel ([`cancel`]).

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

/// Whether `fut` finished on its first poll, i.e. before it waited for anything: with the result.
///
/// [`Pending::poll`] uses it to tell the relay listener's own failure from the grant exchange's.
/// pubky's `try_poll_once` returns a dead listener's error without awaiting anything; an exchange
/// has to wait for the network (homeserver lookup, then the grant POST) before any error, so an
/// error that comes at once was never preceded by a grant being sent.
pub async fn at_once<T>(fut: impl std::future::Future<Output = T>) -> (T, bool) {
    let mut fut = std::pin::pin!(fut);
    let mut first = true;
    let mut ready_at_once = false;
    let out = std::future::poll_fn(|cx| {
        let p = fut.as_mut().poll(cx);
        if first {
            first = false;
            ready_at_once = p.is_ready();
        }
        p
    })
    .await;
    (out, ready_at_once)
}

/// Why one [`Pending::poll`] failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Failed {
    /// The relay listener stopped before it read an approval (no grant was sent): the relay or the
    /// network did not answer (`server`: the relay answered 5xx). Listening again is safe.
    Relay { error: String, server: bool },
    /// Anything else (the grant exchange, the approval itself): final, never retried.
    Final(String),
}

impl Failed {
    /// Sorts a poll error: `at_once` from [`at_once`], `transient` when it is a transport error or
    /// a 5xx answer, `server` when it is a 5xx answer, `relay` the relay host. A transport error
    /// must also name the relay (`https://<relay>/…`): the error then cannot be the exchange's,
    /// whatever pubky does in a later version.
    pub fn sort(error: String, at_once: bool, transient: bool, server: bool, relay: &str) -> Self {
        let names_relay = error.contains(&format!("://{relay}/")) || error.contains(&format!("://{relay})"));
        if at_once && (server || (transient && names_relay)) {
            Failed::Relay { error, server }
        } else {
            Failed::Final(error)
        }
    }
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
    fn poll(&self) -> impl std::future::Future<Output = Result<Option<Self::Done>, Failed>> + Send + '_;
    /// The same sign-in listening again on its relay inbox (a fresh client).
    fn resume(&self) -> Result<Self, String>;
}

/// How long a sign-in waits for Pubky Ring before it gives up.
pub const DEADLINE: Duration = Duration::from_secs(10 * 60);
/// The error of a sign-in the user cancelled (the page shows no error for it).
pub const CANCELLED: &str = "Sign-in cancelled";
/// How long a relay that answered 5xx gets before the one listen-again.
const RELAY_5XX_PAUSE: Duration = if cfg!(test) {
    Duration::from_secs(1)
} else {
    Duration::from_secs(5)
};
/// The error of a sign-in that ran out of time ([`DEADLINE`]).
pub const TIMED_OUT: &str = "Pubky Ring did not answer within 10 minutes. Sign in again.";

/// The latest sign-in, and the latest one the user cancelled.
static SIGNIN: AtomicU64 = AtomicU64::new(0);
static CANCELLED_SIGNIN: AtomicU64 = AtomicU64::new(0);

/// When a sign-in stops waiting: its deadline, or the user's Cancel.
pub struct Stop {
    id: u64,
    deadline: tokio::time::Instant,
}

impl Stop {
    /// A new sign-in that waits at most `within`; it replaces any earlier one for [`cancel`].
    pub fn begin(within: Duration) -> Self {
        Stop {
            id: SIGNIN.fetch_add(1, Ordering::AcqRel) + 1,
            deadline: tokio::time::Instant::now() + within,
        }
    }

    /// Why the sign-in stops now, if it does.
    fn reason(&self) -> Option<&'static str> {
        if CANCELLED_SIGNIN.load(Ordering::Acquire) == self.id {
            Some(CANCELLED)
        } else if tokio::time::Instant::now() >= self.deadline {
            Some(TIMED_OUT)
        } else {
            None
        }
    }
}

/// The Cancel button: the running sign-in stops before its next check (an exchange already under
/// way finishes first; it is never cut off halfway).
pub fn cancel() {
    CANCELLED_SIGNIN.store(SIGNIN.load(Ordering::Acquire), Ordering::Release);
}

/// Returns once the app is on screen and `host:port` answers, or with why `stop` ended the wait.
/// `state` gets [`COME_BACK`] or [`NO_NETWORK`] whenever the wait changes reason, and
/// [`WAITING`] once ready after either.
pub async fn until_ready(
    host: &str,
    port: u16,
    stop: &Stop,
    mut state: impl FnMut(&'static str),
) -> Result<(), &'static str> {
    let mut shown: Option<&'static str> = None;
    loop {
        if let Some(r) = stop.reason() {
            return Err(r);
        }
        let reason = if !on_screen() {
            Some(COME_BACK)
        } else if !online(host, port).await {
            Some(NO_NETWORK)
        } else {
            if shown.is_some() {
                state(WAITING);
            }
            return Ok(());
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

/// Waits for Ring's approval and exchanges the grant only while the app is on screen and online.
/// `since_trip` is [`trips`] when `flow`'s relay listener started; `relay` is the relay host (the
/// network check connects to it on `port`, 443 in the app); `stop` ends the wait (deadline,
/// Cancel); `state` updates the page.
///
/// A relay listener that stopped before it read an approval listens again once per trip away
/// (the network was cut), and once if the relay answered 5xx. Nothing is retried after a grant
/// was sent.
pub async fn approval<F: Pending>(
    mut flow: F,
    since_trip: u64,
    relay: &str,
    port: u16,
    stop: Stop,
    mut state: impl FnMut(&'static str),
) -> Result<F::Done, String> {
    // Trips away already covered by the current relay listener.
    let mut listening_since = since_trip;
    // Whether the one listen-again for a relay 5xx was used.
    let mut resumed_after_5xx = false;
    // The trip after which the network was last seen answering: checked once per return, not
    // before every poll.
    let mut ready_after: Option<u64> = None;
    loop {
        if let Some(r) = stop.reason() {
            return Err(r.to_string());
        }
        // The app can leave between this check and the poll: an exchange then starts within the few
        // seconds Android still leaves it the network.
        if GATED && (!on_screen() || ready_after != Some(trips())) {
            until_ready(relay, port, &stop, &mut state)
                .await
                .map_err(str::to_string)?;
            ready_after = Some(trips());
        }
        match flow.poll().await {
            Ok(Some(done)) => return Ok(done),
            Ok(None) => {}
            Err(Failed::Relay { .. }) if trips() > listening_since => {
                tracing::info!("Ring: the relay poll stopped while the app was away; listening again");
                flow = flow.resume()?;
                listening_since = trips();
            }
            Err(Failed::Relay { server: true, error }) if !resumed_after_5xx => {
                tracing::info!("Ring: the relay answered {error}; listening again once");
                // pubky's own three polls ran back to back: give the relay a moment first.
                let paused = tokio::time::Instant::now();
                while paused.elapsed() < RELAY_5XX_PAUSE {
                    if let Some(r) = stop.reason() {
                        return Err(r.to_string());
                    }
                    tokio::time::sleep(Duration::from_millis(250)).await;
                }
                flow = flow.resume()?;
                resumed_after_5xx = true;
            }
            Err(Failed::Relay { error, .. } | Failed::Final(error)) => return Err(explain(&error)),
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

    #[tokio::test]
    async fn at_once_tells_an_immediate_result_from_one_that_waited() {
        assert_eq!(at_once(async { 1 }).await, (1, true));
        let waited = at_once(async {
            tokio::task::yield_now().await;
            2
        })
        .await;
        assert_eq!(waited, (2, false));
    }

    #[test]
    fn only_an_immediate_transient_error_is_the_relay_listener() {
        let relay = "httprelay.pubky.app";
        let dns = "error sending request for url (https://httprelay.pubky.app/inbox/abc) <- dns error";
        assert_eq!(
            Failed::sort("503".into(), true, true, true, relay),
            Failed::Relay {
                error: "503".into(),
                server: true
            }
        );
        assert_eq!(
            Failed::sort(dns.into(), true, true, false, relay),
            Failed::Relay {
                error: dns.into(),
                server: false
            }
        );
        // A 5xx after the exchange waited for the network: a grant may have been sent.
        assert_eq!(
            Failed::sort("503".into(), false, true, true, relay),
            Failed::Final("503".into())
        );
        // A transport error at once that does not name the relay: not the listener's.
        let raw_key = "error sending request for url (https://p815/auth/grant/session) <- dns error";
        assert_eq!(
            Failed::sort(raw_key.into(), true, true, false, relay),
            Failed::Final(raw_key.into())
        );
        // A bad approval, at once: not the network.
        assert_eq!(
            Failed::sort("invalid".into(), true, false, false, relay),
            Failed::Final("invalid".into())
        );
    }

    #[test]
    fn the_raw_key_error_reads_as_a_lookup_failure() {
        let e = "error sending request for url (https://p815/auth/grant/session) <- dns error <- pkarr could not resolve host";
        assert!(explain(e).starts_with("Pubky Ring approved, but Odeion could not look up"));
        assert_eq!(explain("403 forbidden"), "403 forbidden");
    }

    /// A sign-in whose relay listeners fail with `fails` (in turn); then one delivers.
    struct Fake {
        generation: usize,
        polls: std::sync::Arc<Mutex<Vec<bool>>>,
        fails: Vec<Failed>,
    }

    impl Fake {
        fn new(polls: &std::sync::Arc<Mutex<Vec<bool>>>, fails: Vec<Failed>) -> Self {
            Fake {
                generation: 0,
                polls: polls.clone(),
                fails,
            }
        }
    }

    impl Pending for Fake {
        type Done = usize;
        async fn poll(&self) -> Result<Option<usize>, Failed> {
            self.polls.lock().unwrap().push(on_screen());
            match self.fails.get(self.generation) {
                Some(f) => Err(f.clone()),
                None => Ok(Some(self.generation)),
            }
        }
        fn resume(&self) -> Result<Self, String> {
            Ok(Fake {
                generation: self.generation + 1,
                polls: self.polls.clone(),
                fails: self.fails.clone(),
            })
        }
    }

    fn dns() -> Failed {
        Failed::Relay {
            error: "error sending request for url (https://127.0.0.1/inbox/x) <- dns error".into(),
            server: false,
        }
    }

    fn relay_5xx() -> Failed {
        Failed::Relay {
            error: "Server responded with an error: 502 Bad Gateway - Bad Gateway".into(),
            server: true,
        }
    }

    /// A local port that takes connections: the "relay" for the network check.
    async fn relay_port() -> u16 {
        let relay = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = relay.local_addr().unwrap().port();
        tokio::spawn(async move { while relay.accept().await.is_ok() {} });
        port
    }

    /// The tests share the screen flags and the sign-in counter: one at a time.
    static SERIAL: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

    fn soon() -> Stop {
        Stop::begin(Duration::from_secs(60))
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn the_approval_waits_for_the_screen_and_a_dead_relay_poll_listens_again() {
        let _one = SERIAL.lock().await;
        let port = relay_port().await;
        let polls = std::sync::Arc::new(Mutex::new(Vec::<bool>::new()));
        // The listener starts on screen, then the user leaves for Ring: nothing may be polled
        // until the app is back.
        let since = trips();
        left();
        let flow = Fake::new(&polls, vec![dns()]);
        let task = tokio::spawn(async move {
            let states = std::sync::Arc::new(Mutex::new(Vec::new()));
            let st = states.clone();
            let r = approval(flow, since, "127.0.0.1", port, soon(), move |s| {
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
        assert_eq!(r, Ok(1), "listens again after the relay poll died while away");
        assert_eq!(polls.lock().unwrap().as_slice(), &[true, true]);
        assert_eq!(states.first(), Some(&COME_BACK));
        assert_eq!(states.last(), Some(&WAITING), "the page text is reset once back");

        // On screen and no trip away since: a dead relay poll (not a 5xx) is final.
        let flow = Fake::new(&polls, vec![dns()]);
        let r = tokio::time::timeout(
            Duration::from_secs(10),
            approval(flow, trips(), "127.0.0.1", port, soon(), |_| {}),
        )
        .await
        .unwrap();
        assert!(r.is_err());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_relay_5xx_listens_again_once() {
        let _one = SERIAL.lock().await;
        let port = relay_port().await;
        let polls = std::sync::Arc::new(Mutex::new(Vec::<bool>::new()));
        // On screen, no trip away: the relay answered 502, the second listener delivers after a
        // pause.
        let started = std::time::Instant::now();
        let flow = Fake::new(&polls, vec![relay_5xx()]);
        let r = tokio::time::timeout(
            Duration::from_secs(10),
            approval(flow, trips(), "127.0.0.1", port, soon(), |_| {}),
        )
        .await
        .unwrap();
        assert_eq!(r, Ok(1));
        assert!(
            started.elapsed() >= RELAY_5XX_PAUSE,
            "listens again only after the pause"
        );
        // A second 5xx after that one listen-again is final.
        let flow = Fake::new(&polls, vec![relay_5xx(), relay_5xx()]);
        let r = tokio::time::timeout(
            Duration::from_secs(10),
            approval(flow, trips(), "127.0.0.1", port, soon(), |_| {}),
        )
        .await
        .unwrap();
        assert_eq!(r, Err(relay_5xx_text()));
        // A 5xx of the grant exchange is never retried.
        let flow = Fake::new(&polls, vec![Failed::Final(relay_5xx_text())]);
        let r = tokio::time::timeout(
            Duration::from_secs(10),
            approval(flow, trips(), "127.0.0.1", port, soon(), |_| {}),
        )
        .await
        .unwrap();
        assert_eq!(r, Err(relay_5xx_text()));
    }

    fn relay_5xx_text() -> String {
        match relay_5xx() {
            Failed::Relay { error, .. } => error,
            Failed::Final(e) => e,
        }
    }

    /// A sign-in Ring never answers.
    struct Silent;

    impl Pending for Silent {
        type Done = ();
        async fn poll(&self) -> Result<Option<()>, Failed> {
            Ok(None)
        }
        fn resume(&self) -> Result<Self, String> {
            Ok(Silent)
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn cancel_and_the_deadline_end_the_wait() {
        let _one = SERIAL.lock().await;
        let port = relay_port().await;
        // Cancel while polling (on screen).
        let task = tokio::spawn(approval(Silent, trips(), "127.0.0.1", port, soon(), |_| {}));
        tokio::time::sleep(Duration::from_millis(700)).await;
        cancel();
        let r = tokio::time::timeout(Duration::from_secs(5), task)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(r, Err(CANCELLED.to_string()));

        // Cancel while away (waiting for the screen).
        left();
        let task = tokio::spawn(approval(Silent, trips(), "127.0.0.1", port, soon(), |_| {}));
        tokio::time::sleep(Duration::from_millis(700)).await;
        cancel();
        let r = tokio::time::timeout(Duration::from_secs(5), task)
            .await
            .unwrap()
            .unwrap();
        back();
        assert_eq!(r, Err(CANCELLED.to_string()));

        // An earlier Cancel does not stop the next sign-in; its deadline does.
        let started = std::time::Instant::now();
        let r = tokio::time::timeout(
            Duration::from_secs(5),
            approval(
                Silent,
                trips(),
                "127.0.0.1",
                port,
                Stop::begin(Duration::from_millis(1500)),
                |_| {},
            ),
        )
        .await
        .unwrap();
        assert_eq!(r, Err(TIMED_OUT.to_string()));
        assert!(started.elapsed() >= Duration::from_millis(1500));
    }
}
