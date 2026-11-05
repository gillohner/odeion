//! What kind of picture a cover or an avatar is, from its first bytes: only JPEG, PNG, WebP and
//! GIF are pictures here. A picture's own label (a tag's MIME string, a file name, a server's
//! `Content-Type`) is never taken as its type: a label says nothing about what a browser does
//! with the bytes.

/// The picture's MIME type by its first bytes, or `None` when it is no picture the node serves.
pub fn image_type(bytes: &[u8]) -> Option<&'static str> {
    match bytes {
        [0xFF, 0xD8, 0xFF, ..] => Some("image/jpeg"),
        [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, ..] => Some("image/png"),
        [b'R', b'I', b'F', b'F', _, _, _, _, b'W', b'E', b'B', b'P', ..] => Some("image/webp"),
        [b'G', b'I', b'F', b'8', b'7' | b'9', b'a', ..] => Some("image/gif"),
        _ => None,
    }
}

/// The file extension for a type [`image_type`] gives.
pub fn image_extension(mime: &str) -> &'static str {
    match mime {
        "image/png" => "png",
        "image/webp" => "webp",
        "image/gif" => "gif",
        _ => "jpg",
    }
}

/// [`image_type`] of a file, from its first bytes.
pub fn file_image_type(path: &std::path::Path) -> Option<&'static str> {
    use std::io::Read;
    let mut head = [0u8; 16];
    let mut f = std::fs::File::open(path).ok()?;
    let n = f.read(&mut head).ok()?;
    image_type(&head[..n])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_pictures_are_pictures() {
        assert_eq!(image_type(&[0xFF, 0xD8, 0xFF, 0xE0, 0, 0]), Some("image/jpeg"));
        assert_eq!(image_type(b"\xFF\xD8garbage"), None, "a near-match is no picture");
        assert_eq!(image_type(b"\x89PNGgarbage"), None, "a near-match is no picture");
        assert_eq!(image_type(b"\x89PNG\r\n\x1a\n...."), Some("image/png"));
        assert_eq!(image_type(b"RIFF\0\0\0\0WEBPVP8 "), Some("image/webp"));
        assert_eq!(image_type(b"GIF89a.."), Some("image/gif"));
        for not in [
            &b"<html><script>"[..],
            b"<?xml version=\"1.0\"?><svg",
            b"<svg xmlns",
            b"RIFF\0\0\0\0WAVEfmt ",
            b"",
            b"\xD8\xFF",
            b"PNG",
        ] {
            assert_eq!(image_type(not), None, "{not:?}");
        }
    }
}
