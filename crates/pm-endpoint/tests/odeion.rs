//! The endpoint says Odeion, and takes ODEION_ settings at start like the node
//! (docs/branding-compatibility.md). Runs the real executable with a cleared environment.

use std::process::Command;

fn run(env: &[(&str, &str)], args: &[&str]) -> (String, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_pm-endpoint"))
        .args(args)
        .env_clear()
        .env("RUST_LOG", "info")
        .envs(env.iter().copied())
        .output()
        .expect("runs");
    assert!(out.status.success(), "{out:?}");
    (
        String::from_utf8_lossy(&out.stdout).into(),
        String::from_utf8_lossy(&out.stderr).into(),
    )
}

#[test]
fn help_says_odeion() {
    let (help, _) = run(&[], &["--help"]);
    assert!(
        help.contains("Odeion endpoint") && !help.contains("Pubky Music"),
        "{help}"
    ); // compat: pre-Odeion name must be gone
}

#[test]
fn old_setting_names_are_reported_by_name_at_start() {
    let (_, err) = run(&[("PM_NULL_OUTPUT", "realtime")], &["--outputs"]);
    assert!(
        err.contains("PM_NULL_OUTPUT") && err.contains("ODEION_NULL_OUTPUT"),
        "{err}"
    );
    assert!(!err.contains("realtime"), "the value reached the log: {err}");
    let (_, err) = run(&[("ODEION_NULL_OUTPUT", "realtime")], &["--outputs"]);
    assert!(!err.contains("pre-Odeion"), "{err}");
}
