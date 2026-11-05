//! App updates on Android: what the feed offers this phone, and whether a download is that
//! file. The feed is the desktops' `latest.json` (`update::feed`), whose `android` entry
//! (tools/release/updater-manifest.py) names the APKs with their SHA-256:
//!
//! ```json
//! "android": {"version": "0.3.0", "versionCode": 3000999,
//!   "apks": {"arm64-v8a": {"url": "…/pubky-music-android-arm64.apk", "sha256": "…"},
//!            "universal": {"url": "…/pubky-music-android-universal.apk", "sha256": "…"}}}
//! ```
//!
//! An update is offered only when its versionCode is above the installed one. A download is
//! handed to Android's installer only when its SHA-256 is the feed's; Android then installs it
//! only over an app signed with the same key. The steps that need the phone (the question, the
//! installer) are in `src/android.rs` and `tools/release/android/AppUpdate.kt`.

use serde::Deserialize;
use sha2::{Digest, Sha256};

/// The largest APK the app downloads (the universal one is about 100 MB).
pub const MAX_APK: u64 = 512 * 1024 * 1024;
/// The largest feed the app reads.
pub const MAX_FEED: usize = 1024 * 1024;

#[derive(Debug, Deserialize)]
struct Feed {
    android: Option<Android>,
}

#[derive(Debug, Deserialize)]
struct Android {
    version: String,
    #[serde(rename = "versionCode")]
    version_code: u64,
    apks: std::collections::HashMap<String, Apk>,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
struct Apk {
    url: String,
    sha256: String,
}

/// An update this phone can install.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Offer {
    /// The version people see (versionName), e.g. `0.3.0` or an edge build's `0.3.0-412`.
    pub version: String,
    pub version_code: u64,
    pub url: String,
    /// Lower-case hex.
    pub sha256: String,
}

/// The APK for a phone: the arm64-only one when the app runs as arm64 (the phone can run it),
/// otherwise the universal one.
fn pick(apks: &std::collections::HashMap<String, Apk>, arm64: bool) -> Option<&Apk> {
    if arm64 {
        if let Some(a) = apks.get("arm64-v8a") {
            return Some(a);
        }
    }
    apks.get("universal")
}

/// Is this a SHA-256 in hex?
fn is_sha256(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit())
}

/// What the feed offers a phone whose installed app has `installed` as its versionCode.
/// `None` when the feed has no Android entry, nothing newer, or no APK for this phone;
/// an error when the feed is not what the release workflow writes.
pub fn offer(feed: &[u8], installed: u64, arm64: bool, allow_http: bool) -> anyhow::Result<Option<Offer>> {
    let feed: Feed = serde_json::from_slice(feed)?;
    let Some(android) = feed.android else {
        return Ok(None);
    };
    if android.version_code <= installed {
        return Ok(None);
    }
    let Some(apk) = pick(&android.apks, arm64) else {
        return Ok(None);
    };
    let url = url::Url::parse(&apk.url)?;
    match url.scheme() {
        "https" => {}
        "http" if allow_http => {}
        s => anyhow::bail!("the feed's APK address is {s}:, not https:"),
    }
    if !is_sha256(&apk.sha256) {
        anyhow::bail!("the feed's APK has no SHA-256");
    }
    Ok(Some(Offer {
        version: android.version,
        version_code: android.version_code,
        url: url.into(),
        sha256: apk.sha256.to_ascii_lowercase(),
    }))
}

/// A SHA-256 over a download, fed as it arrives.
#[derive(Default)]
pub struct Hasher(Sha256);

impl Hasher {
    pub fn update(&mut self, chunk: &[u8]) {
        self.0.update(chunk);
    }

    /// Ok when the download is the file the feed names.
    pub fn check(self, expected: &str) -> anyhow::Result<()> {
        let got: String = self.0.finalize().iter().map(|b| format!("{b:02x}")).collect();
        if !is_sha256(expected) || !got.eq_ignore_ascii_case(expected) {
            anyhow::bail!("the download's SHA-256 is {got}, not the feed's {expected}");
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const A: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const U: &str = "BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB";

    fn feed(code: u64) -> String {
        format!(
            r#"{{"version":"0.3.0","notes":"n","pub_date":"2026-11-05T00:00:00Z",
            "platforms":{{"linux-x86_64":{{"signature":"s","url":"https://x/a.AppImage"}}}},
            "android":{{"version":"0.3.0","versionCode":{code},"apks":{{
              "arm64-v8a":{{"url":"https://x/pubky-music-android-arm64.apk","sha256":"{A}"}},
              "universal":{{"url":"https://x/pubky-music-android-universal.apk","sha256":"{U}"}}}}}}}}"#
        )
    }

    #[test]
    fn only_a_higher_version_code_is_offered() {
        let f = feed(3000999);
        assert_eq!(
            offer(f.as_bytes(), 3000999, true, false).unwrap(),
            None,
            "same build"
        );
        assert_eq!(
            offer(f.as_bytes(), 3001000, true, false).unwrap(),
            None,
            "installed is newer"
        );
        let o = offer(f.as_bytes(), 3000412, true, false).unwrap().expect("newer");
        assert_eq!(o.version, "0.3.0");
        assert_eq!(o.version_code, 3000999);
    }

    #[test]
    fn arm64_phones_get_the_arm64_apk_others_the_universal_one() {
        let f = feed(2);
        let arm = offer(f.as_bytes(), 1, true, false).unwrap().unwrap();
        assert_eq!(arm.url, "https://x/pubky-music-android-arm64.apk");
        assert_eq!(arm.sha256, A);
        let other = offer(f.as_bytes(), 1, false, false).unwrap().unwrap();
        assert_eq!(other.url, "https://x/pubky-music-android-universal.apk");
        assert_eq!(other.sha256, U.to_ascii_lowercase());
        // A feed with only the universal APK still updates arm64 phones.
        let only = f.replace("arm64-v8a", "x86");
        let arm = offer(only.as_bytes(), 1, true, false).unwrap().unwrap();
        assert!(arm.url.ends_with("universal.apk"), "{}", arm.url);
    }

    #[test]
    fn a_feed_without_android_offers_nothing() {
        // Every feed before this change: desktops only.
        let f = r#"{"version":"0.2.2","platforms":{"linux-x86_64":{"signature":"s","url":"https://x/a"}}}"#;
        assert_eq!(offer(f.as_bytes(), 1, true, false).unwrap(), None);
        assert!(offer(b"not json", 1, true, false).is_err());
    }

    #[test]
    fn plain_http_and_missing_hashes_are_refused() {
        let http = feed(2).replace("https://x/", "http://x/");
        assert!(offer(http.as_bytes(), 1, true, false).is_err());
        assert!(
            offer(http.as_bytes(), 1, true, true).unwrap().is_some(),
            "the CI fixture feed"
        );
        let nohash = feed(2).replace(A, "").replace(U, "");
        assert!(offer(nohash.as_bytes(), 1, true, false).is_err());
    }

    #[test]
    fn a_download_must_match_the_feeds_sha256() {
        let expected = "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824"; // "hello"
        let mut h = Hasher::default();
        h.update(b"hel");
        h.update(b"lo");
        h.check(expected).expect("same bytes");
        let mut h = Hasher::default();
        h.update(b"hellO");
        assert!(h.check(expected).is_err(), "a changed file is refused");
        let mut h = Hasher::default();
        h.update(b"hello");
        assert!(
            h.check(&expected.to_ascii_uppercase()).is_ok(),
            "hex case does not matter"
        );
        let h = Hasher::default();
        assert!(h.check("").is_err(), "no hash is never a match");
    }
}
