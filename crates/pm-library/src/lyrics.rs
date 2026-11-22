//! Lyrics: embedded tag text or a sidecar `.lrc`, parsed into lines with
//! optional timestamps so a player can follow along.

use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LyricLine {
    /// Position of the line when the text is synchronised (LRC timestamps).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ms: Option<u64>,
    pub text: String,
}

/// Parse LRC or plain text. Lines with `[mm:ss.xx]` tags (several per line
/// allowed) become timed lines sorted by time; metadata tags such as
/// `[ti:…]` are dropped. Returns the lines and whether any was timed.
pub fn parse(text: &str) -> (Vec<LyricLine>, bool) {
    let mut lines = Vec::new();
    let mut synced = false;
    for raw in text.lines() {
        let raw = raw.trim_end();
        let mut rest = raw.trim_start();
        let mut times = Vec::new();
        let mut metadata = false;
        while let Some(stripped) = rest.strip_prefix('[') {
            let Some(end) = stripped.find(']') else { break };
            let tag = &stripped[..end];
            match parse_timestamp(tag) {
                Some(ms) => times.push(ms),
                None if tag.contains(':')
                    && tag
                        .split(':')
                        .next()
                        .is_some_and(|k| k.chars().all(|c| c.is_ascii_alphabetic())) =>
                {
                    metadata = true;
                }
                None => break,
            }
            rest = stripped[end + 1..].trim_start();
        }
        if metadata && times.is_empty() {
            continue;
        }
        let text = rest.trim().to_string();
        if times.is_empty() {
            if !text.is_empty() {
                lines.push(LyricLine { ms: None, text });
            }
        } else {
            synced = true;
            for ms in times {
                lines.push(LyricLine {
                    ms: Some(ms),
                    text: text.clone(),
                });
            }
        }
    }
    if synced {
        lines.retain(|l| l.ms.is_some());
        lines.sort_by_key(|l| l.ms);
    }
    (lines, synced)
}

/// `mm:ss`, `mm:ss.xx`, `mm:ss.xxx` or `hh:mm:ss.xx` → milliseconds.
fn parse_timestamp(tag: &str) -> Option<u64> {
    let mut parts: Vec<&str> = tag.split(':').collect();
    if parts.len() < 2 || parts.len() > 3 {
        return None;
    }
    let last = parts.pop()?;
    let (secs, frac) = match last.split_once('.') {
        Some((s, f)) => (s, f),
        None => (last, ""),
    };
    let secs: u64 = secs.parse().ok()?;
    // Digits only, so the first three are three bytes (a file's text is anything: `[ti:9.99€ x]`
    // reads as minutes "ti", seconds 9 and a fraction that is no number)
    if !frac.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let frac_ms = match frac.len() {
        0 => 0,
        1 => frac.parse::<u64>().ok()? * 100,
        2 => frac.parse::<u64>().ok()? * 10,
        _ => frac[..3].parse::<u64>().ok()?,
    };
    // (numbers too long for a time are no time, not an overflow)
    let mut ms = secs.checked_mul(1000)?.checked_add(frac_ms)?;
    let mins: u64 = parts.pop()?.parse().ok()?;
    ms = ms.checked_add(mins.checked_mul(60_000)?)?;
    if let Some(h) = parts.pop() {
        ms = ms.checked_add(h.parse::<u64>().ok()?.checked_mul(3_600_000)?)?;
    }
    Some(ms)
}

/// Modification time (unix seconds) of the sidecar `.lrc`, if there is one.
pub fn sidecar_mtime(audio: &Path) -> Option<i64> {
    std::fs::metadata(audio.with_extension("lrc"))
        .ok()?
        .modified()
        .ok()?
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .map(|d| d.as_secs() as i64)
}

/// The longest sidecar that is read (a long song's synchronised lyrics are a few tens of KB).
pub const SIDECAR_MAX_BYTES: u64 = 1024 * 1024;

/// The `.lrc` file next to `audio` (same stem), if there is one.
///
/// Only a lyrics file is read: a regular file of at most [`SIDECAR_MAX_BYTES`]. A link is
/// followed only to another `.lrc` file (a library made of links to another disk).
pub fn sidecar(audio: &Path) -> Option<String> {
    use std::io::Read;
    let mut lrc = audio.with_extension("lrc");
    if std::fs::symlink_metadata(&lrc).ok()?.file_type().is_symlink() {
        lrc = std::fs::canonicalize(&lrc).ok()?;
        let named_lrc = lrc.extension().is_some_and(|e| e.eq_ignore_ascii_case("lrc"));
        if !named_lrc {
            return None;
        }
    }
    let file = std::fs::File::open(&lrc).ok()?;
    let meta = file.metadata().ok()?;
    if !meta.is_file() || meta.len() > SIDECAR_MAX_BYTES {
        return None;
    }
    let mut text = Vec::new();
    // (the limit again: the length was the file's a moment ago)
    file.take(SIDECAR_MAX_BYTES).read_to_end(&mut text).ok()?;
    let text = String::from_utf8_lossy(&text).into_owned();
    (!text.trim().is_empty()).then_some(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_lrc_with_metadata_and_repeats() {
        let (lines, synced) = parse("[ti:Song]\n[ar:Band]\n[00:12.00]Hello\n[01:02.5][00:30.250]Twice\n\n");
        assert!(synced);
        assert_eq!(
            lines
                .iter()
                .map(|l| (l.ms.unwrap(), l.text.as_str()))
                .collect::<Vec<_>>(),
            [(12_000, "Hello"), (30_250, "Twice"), (62_500, "Twice")]
        );
    }

    #[test]
    fn plain_text_keeps_lines() {
        let (lines, synced) = parse("First line\n\n[chorus]\nSecond");
        assert!(!synced);
        assert_eq!(
            lines.iter().map(|l| l.text.as_str()).collect::<Vec<_>>(),
            ["First line", "[chorus]", "Second"]
        );
        assert!(lines.iter().all(|l| l.ms.is_none()));
    }

    #[test]
    fn timestamps() {
        assert_eq!(parse_timestamp("00:01"), Some(1000));
        assert_eq!(parse_timestamp("01:01.5"), Some(61_500));
        assert_eq!(parse_timestamp("1:00:00.123"), Some(3_600_123));
        assert_eq!(parse_timestamp("ti"), None);
        assert_eq!(parse_timestamp("ab:cd"), None);
    }

    /// A file's text is anything: a tag that only looks like a time is no time, and never a
    /// panic (the fraction was cut at its third byte, inside a character; long numbers overflowed).
    #[test]
    fn text_that_only_looks_like_a_time_is_not_one() {
        for tag in [
            "ti:9.99€ Song",
            "00:12.34é",
            "00:01.€",
            "00:01.1é",
            "00:01.+12",
            "00:99999999999999999999",
            "18446744073709551615:00",
            "18446744073709551:00:00",
            "307445734561825861:00",
            "00:18446744073709552",
        ] {
            assert_eq!(parse_timestamp(tag), None, "{tag}");
        }
        let (lines, synced) =
            parse("[ti:9.99€ Song]\n[00:12.34é]La\n[00:01.500]Da\n[00:99999999999999999999]x");
        assert!(synced);
        assert_eq!(
            lines.iter().map(|l| (l.ms, l.text.as_str())).collect::<Vec<_>>(),
            [(Some(1500), "Da")],
            "the other lines carry no time, so a synchronised text leaves them out"
        );
    }

    #[test]
    fn sidecar_next_to_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let audio = dir.path().join("a.flac");
        assert!(sidecar(&audio).is_none());
        std::fs::write(dir.path().join("a.lrc"), "[00:00.00]x").unwrap();
        assert_eq!(sidecar(&audio).as_deref(), Some("[00:00.00]x"));
    }

    /// Only a lyrics file is read as a sidecar: a regular file no longer than lyrics are, also
    /// behind a link, which is followed only to another `.lrc` file.
    #[cfg(unix)]
    #[test]
    fn a_sidecar_is_a_lyrics_file_and_nothing_else() {
        use std::os::unix::fs::symlink;
        let dir = tempfile::tempdir().unwrap();
        let music = dir.path().join("music");
        std::fs::create_dir(&music).unwrap();
        let other = dir.path().join("notes.txt");
        std::fs::write(&other, "not lyrics").unwrap();

        // A link to a file that is no .lrc
        symlink(&other, music.join("a.lrc")).unwrap();
        assert_eq!(sidecar(&music.join("a.flac")), None);
        // A link to something that is no file
        symlink("/dev/zero", music.join("b.lrc")).unwrap();
        assert_eq!(sidecar(&music.join("b.flac")), None);
        // A link that leads nowhere, a folder, a file longer than lyrics are
        symlink(dir.path().join("gone.lrc"), music.join("c.lrc")).unwrap();
        assert_eq!(sidecar(&music.join("c.flac")), None);
        std::fs::create_dir(music.join("d.lrc")).unwrap();
        assert_eq!(sidecar(&music.join("d.flac")), None);
        std::fs::write(music.join("e.lrc"), vec![b'x'; SIDECAR_MAX_BYTES as usize + 1]).unwrap();
        assert_eq!(sidecar(&music.join("e.flac")), None);
        std::fs::write(music.join("f.lrc"), vec![b'x'; SIDECAR_MAX_BYTES as usize]).unwrap();
        assert_eq!(
            sidecar(&music.join("f.flac")).map(|t| t.len()),
            Some(SIDECAR_MAX_BYTES as usize)
        );

        // A library of links to another disk: a link to a lyrics file is read, also through a
        // linked folder
        let disk = dir.path().join("disk");
        std::fs::create_dir(&disk).unwrap();
        std::fs::write(disk.join("song.LRC"), "[00:01.00]linked").unwrap();
        symlink(disk.join("song.LRC"), music.join("g.lrc")).unwrap();
        assert_eq!(
            sidecar(&music.join("g.flac")).as_deref(),
            Some("[00:01.00]linked")
        );
        symlink(&disk, music.join("album")).unwrap();
        std::fs::write(disk.join("two.lrc"), "two").unwrap();
        assert_eq!(sidecar(&music.join("album/two.flac")).as_deref(), Some("two"));
    }
}
