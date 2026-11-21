// TIDAL's two built-in clients for full tracks (owner, 2026-11-04): the browser sign-in's
// (PM_TIDAL_PKCE_CLIENT_ID / _SECRET) and the device-code sign-in's (PM_TIDAL_DEVICE_CLIENT_ID /
// _SECRET), release secrets passed only to the steps that build (the node's and the app's). They
// are written masked into $OUT_DIR/tidal_clients.rs (src/client_mask.rs); a build without both
// values of a pair has no such client. Nothing here prints a value.

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
