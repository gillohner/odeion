//! DSF and DFF (DSDIFF) files in the library: their header (format, length and the embedded
//! ID3v2 tag), read by pm-dsd as the player reads it. A small writer in [`synth`] produces
//! valid minimal files for tests.

use crate::error::{LibraryError, Result};
use std::fs::File;
use std::path::Path;

/// The header types are pm-dsd's: the player reads the same header the same way.
pub use pm_dsd::{Container as DsdContainer, Header as DsdHeader};

/// Read the header of a `.dsf` or `.dff` file, sniffing the magic bytes, with its ID3v2 tag.
pub fn read_header(path: &Path) -> Result<DsdHeader> {
    let io_err = |source| LibraryError::Io {
        path: path.to_path_buf(),
        source,
    };
    // (a plain file: the header is read in small pieces at chunk offsets, each after a seek)
    let mut f = File::open(path).map_err(io_err)?;
    pm_dsd::read(&mut f, true).map_err(|e| match e {
        pm_dsd::Error::Io(e) => io_err(e),
        other => LibraryError::Probe {
            path: path.to_path_buf(),
            reason: other.to_string(),
        },
    })
}

/// Writers for minimal valid DSF/DFF files, for tests and tooling.
pub mod synth {
    /// A DSF file with `data` as the raw per-channel-blocked sample bytes
    /// (already laid out in `block_size` blocks per channel) and an optional
    /// ID3v2 tag appended after the data chunk.
    pub fn dsf(rate: u32, channels: u32, block_size: u32, data: &[u8], id3: Option<&[u8]>) -> Vec<u8> {
        let frames_per_block = block_size as u64 * 8;
        let blocks = data.len() as u64 / (block_size as u64 * channels as u64);
        let sample_count = blocks * frames_per_block;
        let data_chunk_size = 12 + data.len() as u64;
        let metadata_ptr = if id3.is_some() {
            28 + 52 + data_chunk_size
        } else {
            0
        };
        let total = 28 + 52 + data_chunk_size + id3.map(|t| t.len() as u64).unwrap_or(0);
        let mut v = Vec::with_capacity(total as usize);
        v.extend_from_slice(b"DSD ");
        v.extend_from_slice(&28u64.to_le_bytes());
        v.extend_from_slice(&total.to_le_bytes());
        v.extend_from_slice(&metadata_ptr.to_le_bytes());
        v.extend_from_slice(b"fmt ");
        v.extend_from_slice(&52u64.to_le_bytes());
        v.extend_from_slice(&1u32.to_le_bytes()); // version
        v.extend_from_slice(&0u32.to_le_bytes()); // format id: DSD raw
        v.extend_from_slice(&(if channels == 1 { 1u32 } else { 2u32 }).to_le_bytes()); // channel type
        v.extend_from_slice(&channels.to_le_bytes());
        v.extend_from_slice(&rate.to_le_bytes());
        v.extend_from_slice(&1u32.to_le_bytes()); // bits per sample
        v.extend_from_slice(&sample_count.to_le_bytes());
        v.extend_from_slice(&block_size.to_le_bytes());
        v.extend_from_slice(&0u32.to_le_bytes()); // reserved
        v.extend_from_slice(b"data");
        v.extend_from_slice(&data_chunk_size.to_le_bytes());
        v.extend_from_slice(data);
        if let Some(t) = id3 {
            v.extend_from_slice(t);
        }
        v
    }

    fn chunk(id: &[u8; 4], body: &[u8]) -> Vec<u8> {
        let mut v = Vec::with_capacity(12 + body.len() + 1);
        v.extend_from_slice(id);
        v.extend_from_slice(&(body.len() as u64).to_be_bytes());
        v.extend_from_slice(body);
        if body.len() % 2 == 1 {
            v.push(0);
        }
        v
    }

    /// A DFF file with byte-interleaved `data` and an optional top-level ID3 chunk.
    pub fn dff(rate: u32, channels: u16, data: &[u8], id3: Option<&[u8]>) -> Vec<u8> {
        let mut snd = Vec::new();
        snd.extend_from_slice(b"SND ");
        snd.extend_from_slice(&chunk(b"FS  ", &rate.to_be_bytes()));
        let mut chnl = Vec::new();
        chnl.extend_from_slice(&channels.to_be_bytes());
        for i in 0..channels {
            let id: &[u8; 4] = match (channels, i) {
                (1, _) => b"C   ",
                (_, 0) => b"SLFT",
                (_, 1) => b"SRGT",
                (_, 2) => b"C   ",
                (_, 3) => b"LFE ",
                (_, 4) => b"LS  ",
                (_, 5) => b"RS  ",
                _ => b"C   ",
            };
            chnl.extend_from_slice(id);
        }
        snd.extend_from_slice(&chunk(b"CHNL", &chnl));
        let mut cmpr = Vec::new();
        cmpr.extend_from_slice(b"DSD ");
        cmpr.push(14);
        cmpr.extend_from_slice(b"not compressed");
        snd.extend_from_slice(&chunk(b"CMPR", &cmpr));

        let mut body = Vec::new();
        body.extend_from_slice(b"DSD ");
        body.extend_from_slice(&chunk(b"FVER", &0x0105_0000u32.to_be_bytes()));
        body.extend_from_slice(&chunk(b"PROP", &snd));
        body.extend_from_slice(&chunk(b"DSD ", data));
        if let Some(t) = id3 {
            body.extend_from_slice(&chunk(b"ID3 ", t));
        }
        let mut v = Vec::new();
        v.extend_from_slice(b"FRM8");
        v.extend_from_slice(&(body.len() as u64).to_be_bytes());
        v.extend_from_slice(&body);
        v
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pm_model::AudioFormat;

    fn id3_bytes() -> Vec<u8> {
        use id3::TagLike;
        let mut t = id3::Tag::new();
        t.set_title("DSD Test");
        t.add_frame(id3::frame::ExtendedText {
            description: "MusicBrainz Track Id".into(),
            value: "b1a9c0e9-d987-4042-ae91-78d6a3267d69".into(),
        });
        let mut out = Vec::new();
        t.write_to(&mut out, id3::Version::Id3v24).unwrap();
        out
    }

    #[test]
    fn dsf_header_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("t.dsf");
        let data = vec![0x69u8; 4096 * 2 * 3]; // 3 blocks × 2 channels
        std::fs::write(&p, synth::dsf(2_822_400, 2, 4096, &data, Some(&id3_bytes()))).unwrap();
        let h = read_header(&p).unwrap();
        assert_eq!(h.container, DsdContainer::Dsf);
        assert_eq!(h.format, AudioFormat::dsd(2_822_400, 2));
        assert_eq!(h.sample_count, 3 * 4096 * 8);
        assert_eq!(h.data_len, data.len() as u64);
        assert_eq!(h.data_offset, 28 + 52 + 12);
        assert_eq!(h.block_size, 4096);
        assert!(h.id3.is_some());
        assert_eq!(h.duration_ms(), Some(3 * 4096 * 8 * 1000 / 2_822_400));
    }

    #[test]
    fn dff_header_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("t.dff");
        let data = vec![0x69u8; 1000];
        std::fs::write(&p, synth::dff(5_644_800, 2, &data, Some(&id3_bytes()))).unwrap();
        let h = read_header(&p).unwrap();
        assert_eq!(h.container, DsdContainer::Dff);
        assert_eq!(h.format, AudioFormat::dsd(5_644_800, 2));
        assert_eq!(h.sample_count, 1000 * 8 / 2);
        assert_eq!(h.data_len, 1000);
        assert!(h.id3.is_some());
        let no_tag = dir.path().join("n.dff");
        std::fs::write(&no_tag, synth::dff(2_822_400, 1, &data, None)).unwrap();
        assert!(read_header(&no_tag).unwrap().id3.is_none());
    }

    #[test]
    fn rejects_garbage() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("x.dsf");
        std::fs::write(&p, b"RIFF....WAVE").unwrap();
        assert!(matches!(read_header(&p), Err(LibraryError::Probe { .. })));
    }

    /// Chunk sizes are the file's word. One that cannot be a size (the sum with its place does
    /// not fit) ends the read with an error; it does not wrap around to an earlier place.
    #[test]
    fn dff_chunk_sizes_that_cannot_be_are_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("t.dff");
        let form = |form_size: u64, chunks: &[u8]| {
            let mut b = b"FRM8".to_vec();
            b.extend(form_size.to_be_bytes());
            b.extend(b"DSD ");
            b.extend(chunks);
            b.extend([0u8; 64]);
            b
        };
        let chunk = |id: &[u8; 4], size: u64, body: &[u8]| {
            let mut b = id.to_vec();
            b.extend(size.to_be_bytes());
            b.extend(body);
            b
        };
        for size in [u64::MAX, u64::MAX - 11, u64::MAX - 27, u64::MAX - 28, 1 << 63] {
            // At the top level, with a form that claims every size there is
            let top = form(u64::MAX, &chunk(b"JUNK", size, &[]));
            // Inside the properties chunk and inside the edited-master chunk
            let prop_body = [&b"SND "[..], &chunk(b"JUNK", size, &[])].concat();
            let prop = form(200, &chunk(b"PROP", prop_body.len() as u64, &prop_body));
            let diin_body = chunk(b"JUNK", size, &[]);
            let diin = form(200, &chunk(b"DIIN", diin_body.len() as u64, &diin_body));
            for (what, bytes) in [("top", top), ("PROP", prop), ("DIIN", diin)] {
                std::fs::write(&p, bytes).unwrap();
                assert!(
                    matches!(read_header(&p), Err(LibraryError::Probe { .. })),
                    "{what} {size}"
                );
            }
        }
    }

    /// A data chunk that says it is longer than the file is as long as the file: the length
    /// and the playing time come from what is there.
    #[test]
    fn a_dff_data_chunk_is_no_longer_than_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("t.dff");
        let data = vec![0x69u8; 1000];
        let mut bytes = synth::dff(2_822_400, 2, &data, None);
        // The `DSD ` data chunk is the last one: its size field sits 8 bytes before the data
        let at = bytes.len() - data.len() - 8;
        assert_eq!(&bytes[at - 4..at], b"DSD ");
        for claimed in [u64::MAX, u64::MAX - 12, 1 << 62] {
            bytes[at..at + 8].copy_from_slice(&claimed.to_be_bytes());
            std::fs::write(&p, &bytes).unwrap();
            let h = read_header(&p).unwrap();
            assert_eq!(h.data_len, 1000, "{claimed}");
            assert_eq!(h.sample_count, 1000 * 8 / 2, "{claimed}");
            assert_eq!(h.duration_ms(), Some(4000 * 1000 / 2_822_400), "{claimed}");
        }
    }

    /// How long a DSF file's tag is is the file's word: for one that says it is longer than
    /// tags are, no memory is set aside, and the file is read without its tag.
    #[test]
    fn a_dsf_tag_of_an_impossible_length_is_not_read() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("t.dsf");
        let data = vec![0x69u8; 4096 * 2];
        // (a tag header that gives the longest length its four bytes hold, about 256 MiB)
        let header = [b'I', b'D', b'3', 4, 0, 0, 0x7F, 0x7F, 0x7F, 0x7F];
        std::fs::write(&p, synth::dsf(2_822_400, 2, 4096, &data, Some(&header))).unwrap();
        let h = read_header(&p).unwrap();
        assert!(h.id3.is_none());
        assert_eq!(h.data_len, data.len() as u64);
        // (a tag of a usual length is still read)
        let tag = id3_bytes();
        std::fs::write(&p, synth::dsf(2_822_400, 2, 4096, &data, Some(&tag))).unwrap();
        assert_eq!(read_header(&p).unwrap().id3.as_deref(), Some(&tag[..]));
    }
}
