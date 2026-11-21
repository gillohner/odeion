// The project's built-in app values come from the build environment (src/project_apps.rs).
//
// Public client ids and keys: read as ODEION_PROJECT_*, else under their pre-Odeion names
// PM_PROJECT_* (the release workflow's repository variables keep their PM_ names), by
// `option_env!` in project_apps.rs; here only their names, so a change rebuilds.
//
// TIDAL's two built-in clients for full tracks are compiled in by pm-sources' own build script
// (crates/pm-sources/build.rs), so the node and the app carry the same ones.

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
}
