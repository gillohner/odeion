// app.js is kept as ordered parts (ui/app/NN-name.js) and served as one file: the parts joined
// end to end, in name order, into $OUT_DIR/app.js (src/lib.rs includes it). tools/ui-src.js joins
// them the same way for the tools, and tools/refactor/web-ui-split.js cuts a whole app.js back
// into the parts.

use std::path::PathBuf;

fn main() {
    println!("cargo:rerun-if-changed=ui/app");
    // (the folder, not the file: a path that does not exist would run this at every build)
    println!("cargo:rerun-if-changed=static");
    let manifest = PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
    // A single app.js beside the parts (a merge kept a branch's copy) would be ignored without a
    // word, and that branch's changes with it
    assert!(
        !manifest.join("static/app.js").exists(),
        "static/app.js exists: the script is kept in ui/app. Cut the file into the parts and \
         remove it: crates/pm-web/ui/README.md has the steps"
    );
    let dir = manifest.join("ui/app");
    let mut names: Vec<String> = std::fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("{}: {e}", dir.display()))
        .map(|f| {
            f.expect("a file of ui/app")
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    // (an editor's own files beside a part it has open are not parts and not mistakes)
    names.retain(|n| !n.starts_with('.') && !n.starts_with('#') && !n.ends_with('~'));
    names.sort();
    let mut joined = Vec::new();
    for name in &names {
        // A file that is not a part would be left out of the script without a word
        assert!(
            is_part(name),
            "ui/app/{name}: not a part of app.js (parts are NN-name.js)"
        );
        joined.extend(std::fs::read(dir.join(name)).unwrap_or_else(|e| panic!("ui/app/{name}: {e}")));
    }
    assert!(!joined.is_empty(), "ui/app has no parts");
    let out = PathBuf::from(std::env::var_os("OUT_DIR").expect("OUT_DIR"));
    std::fs::write(out.join("app.js"), joined).expect("write app.js");
}

/// `NN-name.js`: two digits (the part's place), a dash, lowercase words.
fn is_part(name: &str) -> bool {
    let Some(stem) = name.strip_suffix(".js") else {
        return false;
    };
    let b = stem.as_bytes();
    b.len() > 3
        && b[0].is_ascii_digit()
        && b[1].is_ascii_digit()
        && b[2] == b'-'
        && b[3..]
            .iter()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == b'-')
}
