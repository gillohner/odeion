//! The product's names, and the pre-Odeion names kept so existing installs keep working
//! (docs/branding-compatibility.md).
//!
//! Settings in the environment are spelled `ODEION_<NAME>`; the old `PM_<NAME>` spelling is still
//! read for at least the next release. The code reads the `PM_` names, so the compatibility is
//! one step at the start of each program, before any thread starts and before the environment is
//! read: [`apply_env_compat`] copies every `ODEION_<NAME>` onto `PM_<NAME>` (the new name wins,
//! even when it is empty), and reports, by name only, the old names still in use.
//!
//! Only names made of `A`–`Z`, `0`–`9` and `_` after the prefix are ours; anything else in the
//! environment is left alone.

use std::ffi::{OsStr, OsString};

/// The product: the web page, the app and the integrations' display name.
pub const PRODUCT: &str = "Odeion";
/// What a node is called where it needs saying (and a new node's default name).
pub const NODE: &str = "Odeion node";
/// The daemon's executable.
pub const DAEMON: &str = "odeiond";
/// compat: pre-Odeion name of the daemon's executable, still shipped next to it.
pub const LEGACY_DAEMON: &str = "pm-node";
/// The prefix of settings in the environment.
pub const ENV_PREFIX: &str = "ODEION_";
/// compat: pre-Odeion prefix, still read for at least the next release.
pub const LEGACY_ENV_PREFIX: &str = "PM_";

/// What [`apply_env_compat`] does with an environment.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct EnvPlan {
    /// `PM_<NAME>` set to the value of `ODEION_<NAME>`, which wins even when empty.
    pub set: Vec<(OsString, OsString)>,
    /// `PM_<NAME>` read because no `ODEION_<NAME>` is set.
    pub legacy_only: Vec<String>,
    /// `PM_<NAME>` replaced by the different value of its `ODEION_<NAME>`.
    pub overridden: Vec<String>,
}

impl EnvPlan {
    /// One line for each kind of old name in use, naming the variables and never their values;
    /// empty when there is nothing to say.
    pub fn log_lines(&self) -> Vec<String> {
        let mut lines = Vec::new();
        if !self.legacy_only.is_empty() {
            lines.push(format!(
                "settings read under their pre-Odeion names: {}; both spellings work this release, \
                 the ODEION_ ones are {}",
                self.legacy_only.join(", "),
                self.legacy_only
                    .iter()
                    .map(|n| new_name(n))
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
        if !self.overridden.is_empty() {
            lines.push(format!(
                "{} set as well as {}: the ODEION_ value is used",
                self.overridden
                    .iter()
                    .map(|n| new_name(n))
                    .collect::<Vec<_>>()
                    .join(", "),
                self.overridden.join(", ")
            ));
        }
        lines
    }
}

fn new_name(legacy: &str) -> String {
    format!("{ENV_PREFIX}{}", &legacy[LEGACY_ENV_PREFIX.len()..])
}

/// The part after `prefix` when `name` is one of ours.
fn ours<'a>(name: &'a OsStr, prefix: &str) -> Option<&'a str> {
    let rest = name.to_str()?.strip_prefix(prefix)?;
    (!rest.is_empty()
        && rest
            .bytes()
            .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'_'))
    .then_some(rest)
}

/// The plan for these variables, without touching the process environment.
pub fn plan_env<I: IntoIterator<Item = (OsString, OsString)>>(vars: I) -> EnvPlan {
    let vars: Vec<(OsString, OsString)> = vars.into_iter().collect();
    let new: std::collections::BTreeMap<&str, &OsString> = vars
        .iter()
        .filter_map(|(k, v)| ours(k, ENV_PREFIX).map(|n| (n, v)))
        .collect();
    let mut legacy: Vec<(&str, &OsString)> = vars
        .iter()
        .filter_map(|(k, v)| ours(k, LEGACY_ENV_PREFIX).map(|n| (n, v)))
        .collect();
    legacy.sort_unstable();
    let mut plan = EnvPlan::default();
    for (name, value) in &new {
        plan.set
            .push((format!("{LEGACY_ENV_PREFIX}{name}").into(), (*value).clone()));
    }
    for (name, value) in legacy {
        let full = format!("{LEGACY_ENV_PREFIX}{name}");
        match new.get(name) {
            // Both the same (the image's entrypoint copies ODEION_ onto PM_): nothing to say
            Some(n) if *n == value => {}
            Some(_) => plan.overridden.push(full),
            None => plan.legacy_only.push(full),
        }
    }
    plan
}

/// Applies [`plan_env`] to this process's environment and returns the plan, for the caller to
/// log once its logging is up ([`EnvPlan::log_lines`]).
///
/// Call it first thing in `main`, before any other thread exists: changing the environment while
/// another thread reads it is undefined behaviour on some platforms.
pub fn apply_env_compat() -> EnvPlan {
    let plan = plan_env(std::env::vars_os());
    for (k, v) in &plan.set {
        std::env::set_var(k, v);
    }
    plan
}

/// A value fixed at build time: the `ODEION_` one when it was set (even empty), else the `PM_` one.
/// `build_env(option_env!("ODEION_X"), option_env!("PM_X"))`.
pub const fn build_env(new: Option<&'static str>, legacy: Option<&'static str>) -> Option<&'static str> {
    match new {
        Some(v) => Some(v),
        None => legacy,
    }
}
