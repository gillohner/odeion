// The app's commands, each allowed by name: the connect page (capabilities/default.json) gets
// the connect page's, the node's page (and the offline page) only the player's, through a
// capability the app adds for their origin (src/native.rs, `PAGE_COMMANDS`).
const COMMANDS: &[&str] = &[
    "saved",
    "open_saved",
    "connect_manual",
    "follow_moved",
    "discover_nodes",
    "ring_connect",
    "ring_cancel",
    "open_ring",
    "forget",
    "has_downloads",
    "open_offline",
    "crash_report",
    "crash_dismiss",
    "crash_share",
    "native_load",
    "native_ahead",
    "native_play",
    "native_pause",
    "native_seek",
    "native_stop",
    "native_status",
];

fn main() {
    tauri_build::try_build(
        tauri_build::Attributes::new().app_manifest(tauri_build::AppManifest::new().commands(COMMANDS)),
    )
    .expect("the app's build step");
}
