//! Small text helpers that several layers need.

/// `%XX` in a URL path or a `file://` link, decoded. `+` stays `+` (a path is not a form), and
/// a `%` that is not followed by two hex digits is kept as it is. The result is read as UTF-8,
/// with U+FFFD for bytes that are not.
pub fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        // (bytes, not a slice of the text: the two after a `%` may be half of a character)
        if b[i] == b'%' && i + 2 < b.len() {
            if let (Some(hi), Some(lo)) = ((b[i + 1] as char).to_digit(16), (b[i + 2] as char).to_digit(16)) {
                out.push((hi * 16 + lo) as u8);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::percent_decode;

    /// `%` followed by anything that is not two hex digits is kept as it is, also when the
    /// bytes after it are part of a longer character (the text was cut inside it).
    #[test]
    fn percent_decoding_takes_any_text() {
        assert_eq!(percent_decode("a%20b%2Fc"), "a b/c");
        assert_eq!(percent_decode("%C3%A9t%c3%a9"), "été");
        assert_eq!(percent_decode("A%2BB+C%"), "A+B+C%");
        for kept in ["%aé", "%é1", "100%", "%2", "%zz", "%+f", "%-1", "50%é", "%"] {
            assert_eq!(percent_decode(kept), kept);
        }
        assert_eq!(percent_decode("%FFx"), "\u{fffd}x");
    }
}
