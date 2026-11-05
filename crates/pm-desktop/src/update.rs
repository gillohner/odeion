//! App updates. A release build checks the feed of the channel it was built for
//! (`ODEION_UPDATE_CHANNEL`, or its pre-Odeion name `PM_UPDATE_CHANNEL`, at build time: `edge`
//! for builds of main, `latest` for tagged releases) once at start. When a newer version is there
//! it asks; on yes it installs. Local builds have no channel and never check.
//!
//! Desktops use the Tauri updater: the feed (`latest.json`, written by the release workflow) lists
//! a signed bundle per platform; the app installs only bundles signed with the
//! project's key, and only for the version the signature names
//! (`tauri.{linux,macos,windows}.conf.json`, `plugins.updater`: kept out of
//! the shared config, which Android builds read too), then restarts. Linux updates
//! itself only as an AppImage: a .deb is the package manager's to update.
//!
//! Android (the Tauri updater has no Android support) reads the same feed's `android` entry
//! (`update_apk`): a higher versionCode is offered, the APK is downloaded to the app's cache,
//! checked against the feed's SHA-256 and handed to Android's installer (`android::Updater`),
//! which asks the user to confirm and installs only over an app signed with the same key.
//! Builds without the installer part (the .aab, for stores) never check.

#[cfg(desktop)]
use tauri::AppHandle;
#[cfg(desktop)]
use tauri_plugin_dialog::{DialogExt, MessageDialogButtons, MessageDialogKind};
#[cfg(desktop)]
use tauri_plugin_updater::UpdaterExt;

// compat: pre-Odeion repository name: installed apps read this feed, so it stays where it is
const RELEASES: &str = "https://github.com/gillohner/pubky-music-node/releases";

/// The update dialog's title.
const TITLE: &str = "Update Odeion";

/// The update dialog's question.
#[cfg_attr(not(desktop), allow(dead_code))]
fn question(new: &str, current: &str) -> String {
    format!("Odeion {new} is available (this is {current}).\n\nInstall it now? The app restarts.")
}

/// The question on Android, where the system asks once more and the app does not restart itself.
#[cfg_attr(not(target_os = "android"), allow(dead_code))]
fn question_apk(new: &str, current: &str) -> String {
    format!("Odeion {new} is available (this is {current}).\n\nInstall it now?")
}

/// The channel this build follows, if any.
pub fn channel() -> Option<&'static str> {
    pm_model::brand::build_env(
        option_env!("ODEION_UPDATE_CHANNEL"),
        option_env!("PM_UPDATE_CHANNEL"),
    )
    .filter(|c| !c.is_empty())
}

/// Where a channel's feed is. `latest` is GitHub's latest release, never a pre-release.
pub fn feed(channel: &str) -> String {
    match channel {
        "edge" => format!("{RELEASES}/download/edge/latest.json"),
        _ => format!("{RELEASES}/latest/download/latest.json"),
    }
}

/// The feed a build with this channel checks; `None` for a local build (no channel).
/// CI's update test (feature `update-feed-fixture`, never in releases) names its own feed.
pub fn feed_for(channel: Option<&str>) -> Option<String> {
    let channel = channel?;
    #[cfg(feature = "update-feed-fixture")]
    if let Some(f) = option_env!("ODEION_UPDATE_FEED") {
        return Some(f.to_string());
    }
    Some(feed(channel))
}

/// Check once in the background; ask before installing anything.
#[cfg(desktop)]
pub fn spawn(app: &AppHandle) {
    let Some(channel) = channel() else {
        tracing::info!("updates: off (a local build)");
        return;
    };
    #[cfg(target_os = "linux")]
    if std::env::var_os("APPIMAGE").is_none() {
        tracing::info!("updates: through the package manager (not an AppImage)");
        return;
    }
    let Some(url) = feed_for(Some(channel)) else {
        return;
    };
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        if let Err(e) = check(&app, channel, &url).await {
            tracing::warn!(channel, "update check failed: {e}");
        }
    });
}

#[cfg(desktop)]
async fn check(app: &AppHandle, channel: &str, url: &str) -> anyhow::Result<()> {
    let updater = app.updater_builder().endpoints(vec![url.parse()?])?.build()?;
    let Some(update) = updater.check().await? else {
        tracing::info!(channel, "updates: this is the newest version");
        return Ok(());
    };
    tracing::info!(channel, version = %update.version, "updates: a newer version is available");
    let (tx, rx) = tokio::sync::oneshot::channel();
    app.dialog()
        .message(question(
            &update.version.to_string(),
            &update.current_version.to_string(),
        ))
        .title(TITLE)
        .kind(MessageDialogKind::Info)
        .buttons(MessageDialogButtons::OkCancelCustom(
            "Install and restart".into(),
            "Later".into(),
        ))
        .show(move |yes| {
            let _ = tx.send(yes);
        });
    if !rx.await.unwrap_or(false) {
        tracing::info!("updates: later");
        return Ok(());
    }
    update.download_and_install(|_, _| {}, || {}).await?;
    tracing::info!(version = %update.version, "updates: installed; restarting");
    app.restart();
}

/// Check once in the background (Android); ask before installing anything.
#[cfg(target_os = "android")]
pub fn spawn(app: &tauri::AppHandle) {
    let Some(url) = feed_for(channel()) else {
        tracing::info!("updates: off (a local build)");
        return;
    };
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        // The feed is fetched over TLS, whose verifier is set up on the UI thread.
        crate::android::tls_ready().await;
        if let Err(e) = apk::check(&app, &url).await {
            tracing::warn!("update check failed: {e:#}");
        }
    });
}

#[cfg(target_os = "android")]
mod apk {
    use crate::update_apk::{self, Offer, MAX_APK, MAX_FEED};
    use std::io::Write;
    use std::path::PathBuf;
    use tauri::Manager;

    /// Plain http only for CI's fixture feed.
    const ALLOW_HTTP: bool = cfg!(feature = "update-feed-fixture");

    pub async fn check(app: &tauri::AppHandle, url: &str) -> anyhow::Result<()> {
        let dir = app.path().app_cache_dir()?.join("updates");
        // An APK from an earlier update (installed or not) is not needed any more.
        let _ = std::fs::remove_dir_all(&dir);
        let Some(phone) = crate::android::Updater::get(app).await else {
            tracing::info!("updates: off (this build has no installer)");
            return Ok(());
        };
        let installed = phone.installed_version_code().await?;
        #[cfg(feature = "update-feed-fixture")]
        probe_marker(app, installed);
        let client = reqwest::Client::builder()
            .connect_timeout(std::time::Duration::from_secs(15))
            .read_timeout(std::time::Duration::from_secs(60))
            .build()?;
        let feed = read_capped(
            client
                .get(url)
                .timeout(std::time::Duration::from_secs(30))
                .send()
                .await?,
            MAX_FEED,
        )
        .await?;
        let arm64 = cfg!(target_arch = "aarch64");
        let Some(offer) = update_apk::offer(&feed, installed, arm64, ALLOW_HTTP)? else {
            tracing::info!(installed, "updates: this is the newest version");
            return Ok(());
        };
        tracing::info!(version = %offer.version, code = offer.version_code, installed, "updates: a newer version is available");
        let current = app.package_info().version.to_string();
        let yes = phone
            .ask(
                super::TITLE,
                &super::question_apk(&offer.version, &current),
                "Install",
                "Later",
            )
            .await;
        if !yes {
            tracing::info!("updates: later");
            return Ok(());
        }
        phone
            .tell(&format!("Downloading Odeion {}…", offer.version))
            .await;
        let file = match download(&client, &offer, &dir).await {
            Ok(f) => f,
            Err(e) => {
                tracing::warn!("updates: not installed: {e:#}");
                phone
                    .tell("Odeion was not updated: the download failed or did not match the update.")
                    .await;
                return Ok(());
            }
        };
        tracing::info!(version = %offer.version, "updates: downloaded and checked; handing it to Android");
        if let Some(why) = phone.install(&file, &offer.sha256).await? {
            tracing::warn!("updates: not installed: {why}");
            phone.tell(&format!("Odeion was not updated: {why}.")).await;
        }
        Ok(())
    }

    /// A response body, refused once it is longer than `max`.
    async fn read_capped(mut resp: reqwest::Response, max: usize) -> anyhow::Result<Vec<u8>> {
        let resp_status = resp.status();
        if !resp_status.is_success() {
            anyhow::bail!("the feed answered {resp_status}");
        }
        let mut body = Vec::new();
        while let Some(chunk) = resp.chunk().await? {
            if body.len() + chunk.len() > max {
                anyhow::bail!("the feed is over {max} bytes");
            }
            body.extend_from_slice(&chunk);
        }
        Ok(body)
    }

    /// The offered APK in the app's cache, kept only when its SHA-256 is the feed's.
    async fn download(
        client: &reqwest::Client,
        offer: &Offer,
        dir: &std::path::Path,
    ) -> anyhow::Result<PathBuf> {
        std::fs::create_dir_all(dir)?;
        let path = dir.join(format!("odeion-{}.apk", offer.version_code));
        let r = fetch(client, offer, &path).await;
        if r.is_err() {
            let _ = std::fs::remove_file(&path);
        }
        r.map(|()| path)
    }

    async fn fetch(client: &reqwest::Client, offer: &Offer, path: &std::path::Path) -> anyhow::Result<()> {
        let mut resp = client.get(&offer.url).send().await?.error_for_status()?;
        if resp.content_length().is_some_and(|n| n > MAX_APK) {
            anyhow::bail!("the APK is over {MAX_APK} bytes");
        }
        let mut file = std::fs::File::create(path)?;
        let mut hash = update_apk::Hasher::default();
        let mut len = 0u64;
        while let Some(chunk) = resp.chunk().await? {
            len += chunk.len() as u64;
            if len > MAX_APK {
                anyhow::bail!("the APK is over {MAX_APK} bytes");
            }
            hash.update(&chunk);
            file.write_all(&chunk)?;
        }
        file.sync_all()?;
        hash.check(&offer.sha256)
    }

    /// CI's update test: a file in the app's data that the next build must still find there.
    #[cfg(feature = "update-feed-fixture")]
    fn probe_marker(app: &tauri::AppHandle, installed: u64) {
        let Ok(dir) = app.path().app_config_dir() else {
            return;
        };
        let path = dir.join("update-probe.txt");
        let found = std::fs::read_to_string(&path).ok();
        if found.is_none() {
            let _ = std::fs::create_dir_all(&dir);
            let _ = std::fs::write(&path, format!("written by versionCode {installed}"));
        }
        tracing::info!(
            "UPDATEPROBE {}",
            serde_json::json!({"installed": installed, "marker": found})
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn feeds() {
        assert_eq!(
            feed("edge"),
            "https://github.com/gillohner/pubky-music-node/releases/download/edge/latest.json"
        );
        assert_eq!(
            feed("latest"),
            "https://github.com/gillohner/pubky-music-node/releases/latest/download/latest.json"
        );
        // A local build (no ODEION_UPDATE_CHANNEL or PM_UPDATE_CHANNEL at build time) never checks.
        if option_env!("ODEION_UPDATE_CHANNEL").is_none() && option_env!("PM_UPDATE_CHANNEL").is_none() {
            assert_eq!(channel(), None);
        }
    }

    #[test]
    fn a_build_checks_its_own_channel_and_a_local_build_never_checks() {
        assert_eq!(feed_for(None), None, "no channel: a local build");
        #[cfg(not(feature = "update-feed-fixture"))]
        {
            assert_eq!(feed_for(Some("edge")).as_deref(), Some(feed("edge").as_str()));
            assert_eq!(feed_for(Some("latest")).as_deref(), Some(feed("latest").as_str()));
            // Stable stays stable: anything but edge reads GitHub's latest release.
            assert_eq!(feed_for(Some("v0.3.0")).as_deref(), Some(feed("latest").as_str()));
        }
        if option_env!("ODEION_UPDATE_CHANNEL").is_none() && option_env!("PM_UPDATE_CHANNEL").is_none() {
            assert_eq!(feed_for(channel()), None);
        }
    }

    #[test]
    fn the_android_question_matches_the_desktop_one() {
        let q = question_apk("0.3.0-12", "0.3.0-11");
        assert!(
            q.starts_with("Odeion 0.3.0-12 is available (this is 0.3.0-11)."),
            "{q}"
        );
        assert_eq!(q.lines().next(), question("0.3.0-12", "0.3.0-11").lines().next());
    }

    /// The feed with its Android entry (tools/release/updater-manifest.py) still reads as
    /// before on installed desktops: Tauri's own parser, the same version and platforms.
    #[cfg(desktop)]
    #[test]
    fn desktops_read_a_feed_with_an_android_entry() {
        let feed = r#"{"version": "0.3.0-12", "notes": "See the release notes on GitHub.",
          "pub_date": "2026-11-05T00:00:00Z",
          "platforms": {"linux-x86_64": {"signature": "c2ln", "url": "https://x/pubky-music-linux-x86_64.AppImage"}},
          "android": {"version": "0.3.0-12", "versionCode": 3000012, "apks": {
            "arm64-v8a": {"url": "https://x/pubky-music-android-arm64.apk", "sha256": "00"},
            "universal": {"url": "https://x/pubky-music-android-universal.apk", "sha256": "00"}}}}"#;
        let r: tauri_plugin_updater::RemoteRelease = serde_json::from_str(feed).expect("Tauri reads it");
        assert_eq!(r.version.to_string(), "0.3.0-12");
        assert_eq!(
            r.download_url("linux-x86_64").unwrap().as_str(),
            "https://x/pubky-music-linux-x86_64.AppImage"
        );
        assert!(r.download_url("android").is_err(), "android is no Tauri platform");
    }

    #[test]
    fn the_update_question_says_odeion() {
        assert_eq!(TITLE, "Update Odeion");
        let q = question("0.3.0", "0.2.2");
        assert!(q.starts_with("Odeion 0.3.0 is available (this is 0.2.2)."), "{q}");
        assert!(!q.contains("Pubky Music"), "{q}"); // compat: pre-Odeion name must be gone
    }
}
