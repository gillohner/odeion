//! ODEION_ settings and their pre-Odeion PM_ names (docs/branding-compatibility.md). These test
//! the plan on given variables, never the process environment, so they run in parallel safely;
//! crates/pm-node/tests/odeiond.rs runs the real executable for the process side.

use pm_model::brand::{self, plan_env};
use std::ffi::OsString;

fn vars(list: &[(&str, &str)]) -> Vec<(OsString, OsString)> {
    list.iter()
        .map(|(k, v)| (OsString::from(k), OsString::from(v)))
        .collect()
}

fn set_of(p: &brand::EnvPlan) -> Vec<(String, String)> {
    let mut s: Vec<_> = p
        .set
        .iter()
        .map(|(k, v)| (k.to_string_lossy().into_owned(), v.to_string_lossy().into_owned()))
        .collect();
    s.sort();
    s
}

#[test]
fn the_new_name_alone_is_read_under_the_name_the_code_uses() {
    let p = plan_env(vars(&[("ODEION_TUNNEL_DIR", "/tunnel"), ("HOME", "/h")]));
    assert_eq!(set_of(&p), [("PM_TUNNEL_DIR".to_string(), "/tunnel".to_string())]);
    assert!(p.legacy_only.is_empty() && p.overridden.is_empty(), "{p:?}");
}

#[test]
fn the_old_name_alone_keeps_working_and_is_reported_by_name() {
    let p = plan_env(vars(&[("PM_TUNNEL_DIR", "/old")]));
    assert!(
        p.set.is_empty(),
        "nothing to copy: the code reads PM_TUNNEL_DIR as before"
    );
    assert_eq!(p.legacy_only, ["PM_TUNNEL_DIR"]);
    assert!(p.overridden.is_empty());
}

#[test]
fn the_new_name_wins_over_the_old_one() {
    let p = plan_env(vars(&[
        ("PM_YTDLP", "/old/yt-dlp"),
        ("ODEION_YTDLP", "/new/yt-dlp"),
    ]));
    assert_eq!(set_of(&p), [("PM_YTDLP".to_string(), "/new/yt-dlp".to_string())]);
    assert_eq!(p.overridden, ["PM_YTDLP"]);
    assert!(p.legacy_only.is_empty());
}

#[test]
fn both_names_with_the_same_value_say_nothing() {
    // The image's entrypoint copies ODEION_ settings onto PM_ ones before the node starts
    let p = plan_env(vars(&[("PM_YTDLP", "/same"), ("ODEION_YTDLP", "/same")]));
    assert!(p.overridden.is_empty() && p.legacy_only.is_empty(), "{p:?}");
    assert!(p.log_lines().is_empty());
}

#[test]
fn the_new_name_wins_even_when_empty() {
    let p = plan_env(vars(&[
        ("PM_ACOUSTID_KEY", "old-key"),
        ("ODEION_ACOUSTID_KEY", ""),
    ]));
    assert_eq!(set_of(&p), [("PM_ACOUSTID_KEY".to_string(), String::new())]);
    assert_eq!(p.overridden, ["PM_ACOUSTID_KEY"]);
}

#[test]
fn values_pass_through_unchanged_unicode_and_all() {
    let p = plan_env(vars(&[
        ("ODEION_NAME", "Wohnzimmer – Ödeion ♫ "),
        ("PM_NAME", "x"),
    ]));
    assert_eq!(
        set_of(&p),
        [("PM_NAME".to_string(), "Wohnzimmer – Ödeion ♫ ".to_string())]
    );
}

#[cfg(unix)]
#[test]
fn values_that_are_not_unicode_pass_through_as_bytes() {
    use std::os::unix::ffi::{OsStrExt, OsStringExt};
    let raw = OsString::from_vec(vec![b'/', 0xff, b'x']);
    let p = plan_env(vec![(OsString::from("ODEION_LIBRESPOT"), raw.clone())]);
    assert_eq!(p.set.len(), 1);
    assert_eq!(p.set[0].0, OsString::from("PM_LIBRESPOT"));
    assert_eq!(p.set[0].1.as_bytes(), raw.as_bytes());
}

#[test]
fn other_variables_are_left_alone() {
    let p = plan_env(vars(&[
        ("ODEION_", "no name"),
        ("ODEION_lower", "not ours"),
        ("XODEION_A", "x"),
        ("PMX", "x"),
        ("RUST_LOG", "debug"),
        ("TAURI_SIGNING_PRIVATE_KEY", "secret"),
        ("PATH", "/bin"),
    ]));
    assert_eq!(p, brand::EnvPlan::default());
}

#[test]
fn the_report_names_variables_and_never_their_values() {
    let p = plan_env(vars(&[
        ("PM_HOMESERVER_ADMIN_PASSWORD", "hunter2-secret"),
        ("PM_TOKEN", "tok-old"),
        ("ODEION_TOKEN", "tok-new-secret"),
    ]));
    let lines = p.log_lines();
    let all = lines.join("\n");
    assert!(
        all.contains("PM_HOMESERVER_ADMIN_PASSWORD") && all.contains("ODEION_HOMESERVER_ADMIN_PASSWORD"),
        "{all}"
    );
    assert!(all.contains("PM_TOKEN") && all.contains("ODEION_TOKEN"), "{all}");
    for secret in ["hunter2-secret", "tok-old", "tok-new-secret"] {
        assert!(!all.contains(secret), "a value reached the log: {all}");
    }
    assert!(plan_env(vars(&[("HOME", "/h")])).log_lines().is_empty());
}

#[test]
fn build_values_take_the_new_name_first_even_empty() {
    assert_eq!(brand::build_env(Some("new"), Some("old")), Some("new"));
    assert_eq!(brand::build_env(Some(""), Some("old")), Some(""));
    assert_eq!(brand::build_env(None, Some("old")), Some("old"));
    assert_eq!(brand::build_env(None, None), None);
}

#[test]
fn the_names() {
    assert_eq!(brand::PRODUCT, "Odeion");
    assert_eq!(brand::NODE, "Odeion node");
    assert_eq!(brand::DAEMON, "odeiond");
    assert_eq!(brand::LEGACY_DAEMON, "pm-node");
    assert_eq!(brand::ENV_PREFIX, "ODEION_");
    assert_eq!(brand::LEGACY_ENV_PREFIX, "PM_");
}
