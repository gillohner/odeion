// The project's built-in app values come from the build environment (src/project_apps.rs): public
// client ids and keys only. No TIDAL device client is built in (owner, 2026-10-31): full tracks
// need one the owner enters.
//
// Each value is read as ODEION_PROJECT_*, else under its pre-Odeion name PM_PROJECT_* (the release
// workflow's repository variables keep their PM_ names). Only names are printed here, never a value.

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
