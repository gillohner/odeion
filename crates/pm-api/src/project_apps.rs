//! The project's own service apps, built in so a self-hoster configures nothing
//! (STEWARD-SHARED-APPS.md). Public values: client ids and the AcoustID application key. Every
//! official sign-in with a project app is a public-client flow (OAuth 2 authorization code with
//! PKCE, no secret) that returns through one static page, the [`RETURN_PAGE`], which forwards the
//! browser to the node named in `state`.
//!
//! The values come from the build (`ODEION_PROJECT_TIDAL_CLIENT_ID`, `ODEION_PROJECT_JAMENDO_CLIENT_ID`,
//! `ODEION_PROJECT_ACOUSTID_KEY`, …; the pre-Odeion `PM_PROJECT_` names still work, and the new
//! name wins even when empty), set by the release workflow, so the repository holds none of them.
//! A build without them simply has no built-in apps. A node's own `[tidal]`, `[jamendo]` or
//! `[acoustid]` values always win.
//!
//! TIDAL full tracks (owner, 2026-11-04) have two built-in clients, each an id and a secret:
//! the browser sign-in's ([`tidal_pkce_client`], "Sign in with TIDAL", the only one TIDAL serves
//! lossless and hi-res to) and the device-code sign-in's ([`tidal_device_client`]). They come
//! only from the build environment (`PM_TIDAL_PKCE_CLIENT_ID` / `_SECRET`,
//! `PM_TIDAL_DEVICE_CLIENT_ID` / `_SECRET`, release secrets), compiled in masked by `build.rs`
//! (`client_mask`), never plain in the repository or the binary's strings. Masking is not a
//! security boundary. A build without them has neither: the browser sign-in is then unavailable
//! and the code sign-in needs a device client the owner enters, which always wins over the
//! built-in one.

use pm_model::brand::build_env;

mod built {
    include!(concat!(env!("OUT_DIR"), "/tidal_clients.rs"));
}

/// The build's TIDAL device client (id, secret) for the code sign-in, if it carries one.
pub fn tidal_device_client() -> Option<(String, String)> {
    crate::client_mask::unmask_pair(built::DEVICE)
}

/// The build's TIDAL browser sign-in client (id, secret; OAuth with PKCE), if it carries one.
pub fn tidal_pkce_client() -> Option<(String, String)> {
    crate::client_mask::unmask_pair(built::PKCE)
}

/// The project's TIDAL app (public client id).
pub fn tidal_client_id() -> Option<&'static str> {
    build_env(
        option_env!("ODEION_PROJECT_TIDAL_CLIENT_ID"),
        option_env!("PM_PROJECT_TIDAL_CLIENT_ID"),
    )
    .filter(|v| !v.trim().is_empty())
}

/// The project's Jamendo app (public client id; streaming and the catalogue need nothing more).
pub fn jamendo_client_id() -> Option<&'static str> {
    build_env(
        option_env!("ODEION_PROJECT_JAMENDO_CLIENT_ID"),
        option_env!("PM_PROJECT_JAMENDO_CLIENT_ID"),
    )
    .filter(|v| !v.trim().is_empty())
}

/// The project's AcoustID application key (meant to ship inside the application).
pub fn acoustid_key() -> Option<&'static str> {
    build_env(
        option_env!("ODEION_PROJECT_ACOUSTID_KEY"),
        option_env!("PM_PROJECT_ACOUSTID_KEY"),
    )
    .filter(|v| !v.trim().is_empty())
}

/// The project's fanart.tv key (a public project key; artist portraits and backgrounds).
pub fn fanart_key() -> Option<&'static str> {
    build_env(
        option_env!("ODEION_PROJECT_FANART_KEY"),
        option_env!("PM_PROJECT_FANART_KEY"),
    )
    .filter(|v| !v.trim().is_empty())
}

/// The project's TheAudioDB key (a public project key; artist portraits and biographies). Without
/// one, TheAudioDB's public test key `123` is used, which allows a few requests a minute.
pub fn audiodb_key() -> &'static str {
    build_env(
        option_env!("ODEION_PROJECT_AUDIODB_KEY"),
        option_env!("PM_PROJECT_AUDIODB_KEY"),
    )
    .filter(|v| !v.trim().is_empty())
    .unwrap_or("123")
}

/// This build carries the project's TheAudioDB key (not only the public test key).
pub fn audiodb_built_in() -> bool {
    build_env(
        option_env!("ODEION_PROJECT_AUDIODB_KEY"),
        option_env!("PM_PROJECT_AUDIODB_KEY"),
    )
    .is_some_and(|v| !v.trim().is_empty())
}

/// The project's Google Drive app (client id).
pub fn google_client_id() -> Option<&'static str> {
    build_env(
        option_env!("ODEION_PROJECT_GOOGLE_CLIENT_ID"),
        option_env!("PM_PROJECT_GOOGLE_CLIENT_ID"),
    )
    .filter(|v| !v.trim().is_empty())
}

/// The project's OneDrive app (a public client id: no secret).
pub fn onedrive_client_id() -> Option<&'static str> {
    build_env(
        option_env!("ODEION_PROJECT_ONEDRIVE_CLIENT_ID"),
        option_env!("PM_PROJECT_ONEDRIVE_CLIENT_ID"),
    )
    .filter(|v| !v.trim().is_empty())
}

/// The static sign-in return page registered as the redirect URI on the project's apps
/// (source: github.com/gillohner/pubky-music-auth). `[auth] return_page` overrides it.
pub const RETURN_PAGE: &str = "https://gillohner.github.io/pubky-music-auth/callback";

/// `state` for a sign-in through the return page: `<nonce>.<base64url(callback URL)>`.
pub fn return_state(nonce: &str, callback: &str) -> String {
    use base64::Engine;
    format!(
        "{nonce}.{}",
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(callback.as_bytes())
    )
}

/// The nonce of a `state` (the whole value when it carries no callback).
pub fn state_nonce(state: &str) -> &str {
    state.split('.').next().unwrap_or(state)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn state_carries_the_nonce_and_the_callback() {
        let st = return_state("n0nce", "http://127.0.0.1:7788/tidal/callback");
        assert_eq!(state_nonce(&st), "n0nce");
        use base64::Engine;
        let back = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(st.split_once('.').unwrap().1)
            .unwrap();
        assert_eq!(back, b"http://127.0.0.1:7788/tidal/callback");
        assert_eq!(state_nonce("plain"), "plain");
    }

    /// TIDAL's built-in clients come only from the build environment, masked: the build script
    /// reads the four names, the release workflow passes them as secrets to the steps that build
    /// (never to the whole workflow), the image takes them as BuildKit secrets (never as build
    /// arguments or in the final image), and no built-in value is plain in the source. A test
    /// build without them has neither, and a node with no client of the owner's then has none.
    #[test]
    fn tidal_clients_only_from_the_build_environment_masked() {
        let build = include_str!("../build.rs");
        let release = include_str!("../../../.github/workflows/release.yml");
        let docker = include_str!("../../../Dockerfile");
        let armv7 = include_str!("../../../tools/release/build-armv7.sh");
        for name in [
            "PM_TIDAL_DEVICE_CLIENT_ID",
            "PM_TIDAL_DEVICE_CLIENT_SECRET",
            "PM_TIDAL_PKCE_CLIENT_ID",
            "PM_TIDAL_PKCE_CLIENT_SECRET",
        ] {
            assert!(build.contains(&format!("\"{name}\"")), "build.rs reads {name}");
            // Secrets, scoped to steps: never at the workflow's top level
            let top = release.split("\njobs:").next().unwrap();
            assert!(!top.contains(name), "release.yml sets {name} for every job");
            assert!(
                release.contains(&format!("{name}: ${{{{ secrets.{name} }}}}"))
                    && release.contains(&format!("{name}=${{{{ secrets.{name} }}}}")),
                "release.yml passes {name} to the builds and the image as a secret"
            );
            assert!(
                !release.contains(&format!("{name}: ${{{{ vars.")),
                "{name} is a secret, not a variable"
            );
            assert!(
                docker.contains(&format!("--mount=type=secret,id={name}")),
                "Dockerfile mounts {name}"
            );
            for not in [format!("ARG {name}"), format!("ENV {name}")] {
                assert!(!docker.contains(&not), "the image keeps {name} as {not}");
            }
            assert!(
                armv7.contains(&format!("-e {name}")),
                "build-armv7.sh forwards {name} by name"
            );
        }
        // The final image stage never sees them
        let last = docker.rsplit("\nFROM ").next().unwrap();
        assert!(
            !last.contains("TIDAL_PKCE") && !last.contains("TIDAL_DEVICE"),
            "{last}"
        );
        // What a release build carries is in no source file in plain
        let sources = [
            include_str!("project_apps.rs"),
            include_str!("client_mask.rs"),
            include_str!("tidal_unofficial.rs"),
            include_str!("tidal.rs"),
            build,
        ];
        for (id, secret) in [tidal_device_client(), tidal_pkce_client()].into_iter().flatten() {
            for text in sources {
                assert!(
                    !text.contains(&id) && !text.contains(&secret),
                    "a built-in value in plain"
                );
            }
        }
        // Without a client of the owner's: the build's, or none at all
        if std::env::var("PM_TIDAL_UNOFFICIAL_CLIENT_ID").is_err() {
            let lib = pm_library::Library::open_in_memory().unwrap();
            let c = crate::tidal_unofficial::client_of(&lib);
            match tidal_device_client() {
                None => assert!(c.is_none()),
                Some((id, _)) => assert!(c.is_some_and(|c| c.from == "builtin" && c.id == id)),
            }
        }
    }
}
