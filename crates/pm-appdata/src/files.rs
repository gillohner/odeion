//! Song files in a person's private storage (`/priv/pubky.music/files/`): each is named
//! `<first 16 hex of its BLAKE3>-<name>`, so the same file uploaded twice (by a node or a phone)
//! lands on one path, and a node finds a phone's file by its hash.

/// Where song files live.
pub const FILES_DIR: &str = "/priv/pubky.music/files/";

/// A file's name with anything but letters, digits, `.`, `-` and `_` turned into `_`.
pub fn safe_file_name(local: &std::path::Path) -> String {
    let name = local
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "file".into());
    let safe: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_') {
                c
            } else {
                '_'
            }
        })
        .collect();
    safe
}

/// `<first 16 hex of the hash>-<name>`, adding the prefix once: a file the node brought from the
/// homeserver already carries it (that is how `x-x-01.flac` names appeared, owner 2026-10-22).
pub fn remote_name_for(hash: &str, safe_name: &str) -> String {
    let prefix = &hash[..16];
    match split_remote_name(safe_name) {
        Some((h, _)) if h == prefix => safe_name.to_string(),
        _ => format!("{prefix}-{safe_name}"),
    }
}

/// A remote file name's hash prefix and original name, when it has one.
pub fn split_remote_name(name: &str) -> Option<(&str, &str)> {
    let (h, rest) = name.split_at_checked(17)?;
    let h = h.strip_suffix('-')?;
    (h.len() == 16 && h.bytes().all(|b| b.is_ascii_hexdigit()) && !rest.is_empty()).then_some((h, rest))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_carry_the_hash_once_and_only_safe_letters() {
        let h = "0123456789abcdef0123456789abcdef";
        let safe = safe_file_name(std::path::Path::new("/x/01 Héllo (live).flac"));
        assert_eq!(safe, "01_H_llo__live_.flac");
        let n = remote_name_for(h, &safe);
        assert_eq!(n, "0123456789abcdef-01_H_llo__live_.flac");
        assert_eq!(remote_name_for(h, &n), n, "the prefix is added once");
        assert_eq!(
            split_remote_name(&n),
            Some(("0123456789abcdef", "01_H_llo__live_.flac"))
        );
        assert_eq!(split_remote_name("song.flac"), None);
    }
}
