// The project's built-in app values come from the build environment (src/project_apps.rs).
//
// Public client ids and keys: read as ODEION_PROJECT_*, else under their pre-Odeion names
// PM_PROJECT_* (the release workflow's repository variables keep their PM_ names), by
// `option_env!` in project_apps.rs; here only their names, so a change rebuilds.
//
// TIDAL's two built-in clients for full tracks (owner, 2026-11-04): the browser sign-in's
// (PM_TIDAL_PKCE_CLIENT_ID / _SECRET) and the device-code sign-in's (PM_TIDAL_DEVICE_CLIENT_ID /
// _SECRET), release secrets passed only to the steps that build. They are written masked into
// $OUT_DIR/tidal_clients.rs (src/client_mask.rs); a build without both values of a pair has no
// such client. Nothing here prints a value.

#[path = "src/client_mask.rs"]
mod client_mask;

const CLIENTS: &[(&str, &str, &str)] = &[
    (
        "DEVICE",
        "PM_TIDAL_DEVICE_CLIENT_ID",
        "PM_TIDAL_DEVICE_CLIENT_SECRET",
    ),
    ("PKCE", "PM_TIDAL_PKCE_CLIENT_ID", "PM_TIDAL_PKCE_CLIENT_SECRET"),
];

fn main() {
    for v in [
        "ODEION_PROJECT_TIDAL_CLIENT_ID",
        "ODEION_PROJECT_JAMENDO_CLIENT_ID",
        "ODEION_PROJECT_ACOUSTID_KEY",
        "ODEION_PROJECT_FANART_KEY",
        "ODEION_PROJECT_AUDIODB_KEY",
        "ODEION_PROJECT_ONEDRIVE_CLIENT_ID",
        "ODEION_PROJECT_GOOGLE_CLIENT_ID",
        "PM_PROJECT_TIDAL_CLIENT_ID",
        "PM_PROJECT_JAMENDO_CLIENT_ID",
        "PM_PROJECT_ACOUSTID_KEY",
        "PM_PROJECT_FANART_KEY",
        "PM_PROJECT_AUDIODB_KEY",
        "PM_PROJECT_ONEDRIVE_CLIENT_ID",
        "PM_PROJECT_GOOGLE_CLIENT_ID",
    ] {
        println!("cargo:rerun-if-env-changed={v}");
    }
    let mut clients = Vec::new();
    for (name, id, secret) in CLIENTS {
        println!("cargo:rerun-if-env-changed={id}");
        println!("cargo:rerun-if-env-changed={secret}");
        clients.push((
            *name,
            client_mask::pair(std::env::var(id).ok(), std::env::var(secret).ok()),
        ));
    }
    println!("cargo:rerun-if-changed=src/client_mask.rs");
    let out = std::path::PathBuf::from(std::env::var_os("OUT_DIR").expect("OUT_DIR"));
    std::fs::write(out.join("tidal_clients.rs"), client_mask::generate(&clients))
        .expect("write tidal_clients.rs");
}
