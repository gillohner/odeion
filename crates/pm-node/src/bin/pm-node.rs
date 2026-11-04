// compat: pre-Odeion name of the `odeiond` executable, kept so scripts, systemd units and images
// that run `pm-node` keep working (docs/branding-compatibility.md). The same program, built from
// the same source; it only says, once, that it is the old name.
include!("../main.rs");
