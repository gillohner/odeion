//! The daemon is `odeiond`; `pm-node`, its pre-Odeion name, is the same program and keeps working
//! (docs/branding-compatibility.md). Each test runs the real executables in their own process with
//! a cleared environment and a throwaway HOME, so nothing here reads or changes this process's
//! environment, and no node announces itself (loopback only, LAN features off).

use std::io::{Read, Write};
use std::path::Path;
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

const ODEIOND: &str = env!("CARGO_BIN_EXE_odeiond");
const PM_NODE: &str = env!("CARGO_BIN_EXE_pm-node");

fn run(bin: &str, home: &Path, args: &[&str], env: &[(&str, &str)]) -> Output {
    Command::new(bin)
        .args(args)
        .env_clear()
        .env("HOME", home)
        .env("RUST_LOG", "info")
        .envs(env.iter().copied())
        .output()
        .expect("runs")
}

fn text(b: &[u8]) -> String {
    String::from_utf8_lossy(b).into_owned()
}

/// The text of a config for a node on loopback that finds and announces nothing. The data folder
/// is written as a TOML string (`toml` escapes it), so a Windows path (`C:\\Users\\…`) stays valid.
fn quiet_config_text(data_dir: &str, port: u16) -> String {
    format!(
        "data_dir = {}\n\n[api]\nlisten = \"127.0.0.1:{port}\"\ntoken = \"fixture-token-odeion\"\n\
         announce = false\n\n[outputs]\ndiscover = false\nupnp = false\ncast = false\nairplay = false\n\
         bluos = false\nslimproto_port = 0\n\n[renderer]\nenabled = false\n",
        toml::Value::String(data_dir.to_string())
    )
}

/// A config for a node on loopback that finds and announces nothing.
fn quiet_config(dir: &Path, port: u16) -> std::path::PathBuf {
    let conf = dir.join("node.toml");
    std::fs::write(
        &conf,
        quiet_config_text(&dir.join("data").to_string_lossy(), port),
    )
    .unwrap();
    conf
}

#[test]
fn a_windows_data_folder_is_written_as_valid_toml() {
    for data in [
        r"C:\Users\fixture\AppData\Local\Temp\.tmpAb12\data",
        r"\\server\share\odeion data",
        "/tmp/plain/data",
        "/tmp/with \"quote\" and ♫",
    ] {
        let v: toml::Value = toml::from_str(&quiet_config_text(data, 9)).expect("valid TOML");
        assert_eq!(v["data_dir"].as_str(), Some(data));
        assert_eq!(v["api"]["listen"].as_str(), Some("127.0.0.1:9"));
    }
}

#[test]
fn odeiond_says_odeion_in_its_help_and_version() {
    let home = tempfile::tempdir().unwrap();
    let out = run(ODEIOND, home.path(), &["--help"], &[]);
    assert!(out.status.success(), "{out:?}");
    let help = text(&out.stdout);
    assert!(help.contains("Odeion node"), "{help}");
    assert!(help.contains("Usage: odeiond"), "{help}");
    assert!(!help.contains("Pubky Music"), "{help}"); // compat: pre-Odeion name must be gone
    let v = text(&run(ODEIOND, home.path(), &["--version"], &[]).stdout);
    assert!(v.starts_with("odeiond "), "{v}");
}

#[test]
fn pm_node_is_the_same_program_under_its_old_name() {
    let home = tempfile::tempdir().unwrap();
    let out = run(PM_NODE, home.path(), &["--help"], &[]);
    assert!(out.status.success(), "{out:?}");
    let help = text(&out.stdout);
    assert!(
        help.contains("Odeion node") && help.contains("Usage: pm-node"),
        "{help}"
    );
    // Every subcommand and option is the same
    let new = text(&run(ODEIOND, home.path(), &["--help"], &[]).stdout);
    assert_eq!(help.replace("Usage: pm-node", "Usage: odeiond"), new);
    let (a, b) = (
        text(&run(ODEIOND, home.path(), &["--version"], &[]).stdout),
        text(&run(PM_NODE, home.path(), &["--version"], &[]).stdout),
    );
    assert_eq!(a, b, "one version for both names");
    // Scripts that print the config keep working, and the old name says it is the old name
    let conf = quiet_config(home.path(), 9);
    let out = run(
        PM_NODE,
        home.path(),
        &["-c", conf.to_str().unwrap(), "config"],
        &[],
    );
    assert!(out.status.success(), "{out:?}");
    assert!(text(&out.stdout).contains("fixture-token-odeion"));
    assert!(
        text(&out.stderr).contains("pm-node is the pre-Odeion name of odeiond"),
        "{}",
        text(&out.stderr)
    );
    let out = run(
        ODEIOND,
        home.path(),
        &["-c", conf.to_str().unwrap(), "config"],
        &[],
    );
    assert!(
        !text(&out.stderr).contains("pre-Odeion name of odeiond"),
        "{}",
        text(&out.stderr)
    );
}

#[test]
fn a_new_node_is_called_odeion_node() {
    let home = tempfile::tempdir().unwrap();
    let out = run(ODEIOND, home.path(), &["config"], &[]);
    assert!(out.status.success(), "{out:?}");
    assert!(
        text(&out.stdout).contains("name = \"Odeion node\""),
        "{}",
        text(&out.stdout)
    );
}

#[test]
fn old_setting_names_are_reported_by_name_without_their_values() {
    let home = tempfile::tempdir().unwrap();
    let conf = quiet_config(home.path(), 9);
    let c = conf.to_str().unwrap();
    let out = run(
        ODEIOND,
        home.path(),
        &["-c", c, "config"],
        &[
            ("PM_HOMESERVER_ADMIN_PASSWORD", "fixture-secret-old"),
            ("PM_TUNNEL_DIR", "/tmp/x"),
        ],
    );
    assert!(out.status.success(), "{out:?}");
    let err = text(&out.stderr);
    assert!(
        err.contains("PM_HOMESERVER_ADMIN_PASSWORD") && err.contains("ODEION_HOMESERVER_ADMIN_PASSWORD"),
        "{err}"
    );
    assert!(err.contains("PM_TUNNEL_DIR"), "{err}");
    let out = run(
        ODEIOND,
        home.path(),
        &["-c", c, "config"],
        &[
            ("PM_HOMESERVER_ADMIN_PASSWORD", "fixture-secret-old"),
            ("ODEION_HOMESERVER_ADMIN_PASSWORD", "fixture-secret-new"),
        ],
    );
    let err = text(&out.stderr);
    assert!(
        err.contains("ODEION_HOMESERVER_ADMIN_PASSWORD set as well as PM_HOMESERVER_ADMIN_PASSWORD"),
        "{err}"
    );
    for o in [&out.stdout, &out.stderr] {
        let t = text(o);
        assert!(!t.contains("fixture-secret"), "a value reached the output: {t}");
    }
    // Nothing old, nothing said
    let err = text(
        &run(
            ODEIOND,
            home.path(),
            &["-c", c, "config"],
            &[("ODEION_TUNNEL_DIR", "/tmp/x")],
        )
        .stderr,
    );
    assert!(!err.contains("pre-Odeion"), "{err}");
}

fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

fn get(port: u16, path: &str) -> Option<String> {
    let mut s = std::net::TcpStream::connect(("127.0.0.1", port)).ok()?;
    s.set_read_timeout(Some(Duration::from_secs(5))).ok()?;
    write!(
        s,
        "GET {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nAuthorization: Bearer fixture-token-odeion\r\nConnection: close\r\n\r\n"
    )
    .ok()?;
    let mut body = String::new();
    s.read_to_string(&mut body).ok()?;
    Some(body)
}

/// Serves until `/v1/node/settings` answers, and returns that answer.
fn serve_and_read_settings(bin: &str, env: &[(&str, &str)]) -> String {
    let home = tempfile::tempdir().unwrap();
    let port = free_port();
    let conf = quiet_config(home.path(), port);
    let mut child = Command::new(bin)
        .args(["-c", conf.to_str().unwrap(), "serve"])
        .env_clear()
        .env("HOME", home.path())
        .envs(env.iter().copied())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let end = Instant::now() + Duration::from_secs(60);
    let answer = loop {
        if let Some(r) = get(port, "/v1/node/settings").filter(|r| r.starts_with("HTTP/1.1 200")) {
            break r;
        }
        assert!(Instant::now() < end, "the node did not answer");
        assert!(child.try_wait().unwrap().is_none(), "the node stopped");
        std::thread::sleep(Duration::from_millis(200));
    };
    let _ = child.kill();
    let _ = child.wait();
    answer
}

#[test]
fn the_running_node_reads_new_names_old_names_and_prefers_the_new_one() {
    let on = "\"dev_outputs\":true";
    assert!(serve_and_read_settings(ODEIOND, &[]).contains("\"dev_outputs\":false"));
    assert!(
        serve_and_read_settings(ODEIOND, &[("ODEION_DEV_OUTPUTS", "1")]).contains(on),
        "new name alone"
    );
    assert!(
        serve_and_read_settings(PM_NODE, &[("PM_DEV_OUTPUTS", "1")]).contains(on),
        "old name alone"
    );
    let r = serve_and_read_settings(ODEIOND, &[("PM_DEV_OUTPUTS", "1"), ("ODEION_DEV_OUTPUTS", "")]);
    assert!(
        r.contains("\"dev_outputs\":false"),
        "the new name wins even empty: {r}"
    );
}
