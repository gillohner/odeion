//! DSF and DFF (DSDIFF) container headers: format, length and the embedded
//! ID3v2 tag. This module reads headers only; sample data is pm-audio's job.
//! A small writer in [`synth`] produces valid minimal files for tests.

use crate::error::{LibraryError, Result};
use pm_model::AudioFormat;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DsdContainer {
    Dsf,
    Dff,
}

impl DsdContainer {
    pub fn name(&self) -> &'static str {
        match self {
            DsdContainer::Dsf => "dsf",
            DsdContainer::Dff => "dff",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DsdHeader {
    pub container: DsdContainer,
    pub format: AudioFormat,
    /// One-bit samples per channel.
    pub sample_count: u64,
    /// Byte offset of the first sample byte.
    pub data_offset: u64,
    /// Length in bytes of the sample data.
    pub data_len: u64,
    /// DSF: bytes per channel block (usually 4096). DFF: 1 (byte-interleaved).
    pub block_size: u32,
    /// Raw ID3v2 tag bytes, if present.
    pub id3: Option<Vec<u8>>,
}

impl DsdHeader {
    pub fn duration_ms(&self) -> Option<u64> {
        if self.format.rate_hz == 0 {
            None
        } else {
            Some(self.sample_count.checked_mul(1000)? / self.format.rate_hz as u64)
        }
    }
}

fn io_err(path: &Path, e: std::io::Error) -> LibraryError {
    LibraryError::Io {
        path: path.to_path_buf(),
        source: e,
    }
}

fn bad(path: &Path, reason: impl Into<String>) -> LibraryError {
    LibraryError::Probe {
        path: path.to_path_buf(),
        reason: reason.into(),
    }
}

/// Read the header of a `.dsf` or `.dff` file, sniffing the magic bytes.
pub fn read_header(path: &Path) -> Result<DsdHeader> {
    let mut f = File::open(path).map_err(|e| io_err(path, e))?;
    let mut magic = [0u8; 4];
    f.read_exact(&mut magic).map_err(|e| io_err(path, e))?;
    f.seek(SeekFrom::Start(0)).map_err(|e| io_err(path, e))?;
    match &magic {
        b"DSD " => read_dsf(path, &mut f),
        b"FRM8" => read_dff(path, &mut f),
        _ => Err(bad(path, "not a DSF or DFF file")),
    }
}

fn read_exact_at<R: Read + Seek>(path: &Path, r: &mut R, off: u64, buf: &mut [u8]) -> Result<()> {
    r.seek(SeekFrom::Start(off)).map_err(|e| io_err(path, e))?;
    r.read_exact(buf).map_err(|e| io_err(path, e))
}

fn read_dsf<R: Read + Seek>(path: &Path, r: &mut R) -> Result<DsdHeader> {
    let mut hdr = [0u8; 28];
    read_exact_at(path, r, 0, &mut hdr)?;
    let chunk_size = u64::from_le_bytes(hdr[4..12].try_into().unwrap());
    if chunk_size != 28 {
        return Err(bad(path, "dsf: DSD chunk size is not 28"));
    }
    let metadata_ptr = u64::from_le_bytes(hdr[20..28].try_into().unwrap());

    let mut fmt = [0u8; 52];
    read_exact_at(path, r, 28, &mut fmt)?;
    if &fmt[0..4] != b"fmt " {
        return Err(bad(path, "dsf: missing fmt chunk"));
    }
    let fmt_size = u64::from_le_bytes(fmt[4..12].try_into().unwrap());
    let format_id = u32::from_le_bytes(fmt[16..20].try_into().unwrap());
    let channels = u32::from_le_bytes(fmt[24..28].try_into().unwrap());
    let rate = u32::from_le_bytes(fmt[28..32].try_into().unwrap());
    let bits = u32::from_le_bytes(fmt[32..36].try_into().unwrap());
    let sample_count = u64::from_le_bytes(fmt[36..44].try_into().unwrap());
    let block_size = u32::from_le_bytes(fmt[44..48].try_into().unwrap());
    if format_id != 0 {
        return Err(bad(path, format!("dsf: unsupported format id {format_id}")));
    }
    if !(1..=8).contains(&channels) {
        return Err(bad(path, format!("dsf: bad channel count {channels}")));
    }
    if bits != 1 && bits != 8 {
        return Err(bad(path, format!("dsf: bits per sample {bits} not 1 or 8")));
    }

    let data_hdr_off = 28 + fmt_size;
    let mut data = [0u8; 12];
    read_exact_at(path, r, data_hdr_off, &mut data)?;
    if &data[0..4] != b"data" {
        return Err(bad(path, "dsf: missing data chunk"));
    }
    let data_chunk_size = u64::from_le_bytes(data[4..12].try_into().unwrap());
    let data_len = data_chunk_size.saturating_sub(12);

    let id3 = if metadata_ptr != 0 {
        read_id3_at(path, r, metadata_ptr)?
    } else {
        None
    };

    let format = AudioFormat::dsd(rate, channels as u8);
    format.validate().map_err(|e| bad(path, format!("dsf: {e}")))?;
    Ok(DsdHeader {
        container: DsdContainer::Dsf,
        format,
        sample_count,
        data_offset: data_hdr_off + 12,
        data_len,
        block_size,
        id3,
    })
}

/// The longest ID3v2 tag that is read from a DSF file (as the reader that plays it allows).
const MAX_ID3: u64 = 16 * 1024 * 1024;

/// Read an ID3v2 tag starting at `off`: header + the synchsafe size.
fn read_id3_at<R: Read + Seek>(path: &Path, r: &mut R, off: u64) -> Result<Option<Vec<u8>>> {
    let mut h = [0u8; 10];
    if read_exact_at(path, r, off, &mut h).is_err() {
        return Ok(None);
    }
    if &h[0..3] != b"ID3" {
        return Ok(None);
    }
    let size = h[6..10]
        .iter()
        .fold(0u64, |acc, b| (acc << 7) | (*b as u64 & 0x7f));
    let footer = if h[5] & 0x10 != 0 { 10 } else { 0 };
    let total = 10 + size + footer;
    // The length is the file's word (up to 256 MiB fits its four bytes): no memory is set aside
    // for a tag longer than tags are; the file is read without it
    if total > MAX_ID3 {
        return Ok(None);
    }
    let mut buf = vec![0u8; total as usize];
    read_exact_at(path, r, off, &mut buf)?;
    Ok(Some(buf))
}

/// Where the chunk after the one at `at` starts (a 12-byte header, `size` bytes, padded to an
/// even length), or `None` when the size is no size a file can have (the sum does not fit).
fn next_chunk(at: u64, size: u64) -> Option<u64> {
    at.checked_add(12)?.checked_add(size)?.checked_add(size & 1)
}

fn read_dff<R: Read + Seek>(path: &Path, r: &mut R) -> Result<DsdHeader> {
    let mut hdr = [0u8; 16];
    read_exact_at(path, r, 0, &mut hdr)?;
    if &hdr[0..4] != b"FRM8" || &hdr[12..16] != b"DSD " {
        return Err(bad(path, "dff: not a DSDIFF form"));
    }
    let form_size = u64::from_be_bytes(hdr[4..12].try_into().unwrap());
    // Sizes are the file's word: sums are checked, and one that cannot be ends the walk
    let end = form_size.saturating_add(12);

    let mut rate = None;
    let mut channels = None;
    let mut data: Option<(u64, u64)> = None;
    let mut id3 = None;

    let mut pos = 16u64;
    while pos.saturating_add(12) <= end {
        let mut ch = [0u8; 12];
        if read_exact_at(path, r, pos, &mut ch).is_err() {
            break;
        }
        let id = [ch[0], ch[1], ch[2], ch[3]];
        let size = u64::from_be_bytes(ch[4..12].try_into().unwrap());
        let body = pos + 12;
        match &id {
            b"PROP" => {
                // "SND " then sub-chunks.
                let mut kind = [0u8; 4];
                read_exact_at(path, r, body, &mut kind)?;
                if &kind == b"SND " {
                    let mut p = body + 4;
                    let pend = body.saturating_add(size);
                    while p.saturating_add(12) <= pend {
                        let mut sc = [0u8; 12];
                        read_exact_at(path, r, p, &mut sc)?;
                        let sid = [sc[0], sc[1], sc[2], sc[3]];
                        let ssize = u64::from_be_bytes(sc[4..12].try_into().unwrap());
                        match &sid {
                            b"FS  " => {
                                let mut v = [0u8; 4];
                                read_exact_at(path, r, p + 12, &mut v)?;
                                rate = Some(u32::from_be_bytes(v));
                            }
                            b"CHNL" => {
                                let mut v = [0u8; 2];
                                read_exact_at(path, r, p + 12, &mut v)?;
                                channels = Some(u16::from_be_bytes(v) as u32);
                            }
                            _ => {}
                        }
                        let Some(next) = next_chunk(p, ssize) else { break };
                        p = next;
                    }
                }
            }
            b"DSD " => data = Some((body, size)),
            b"ID3 " => id3 = read_id3_at(path, r, body)?,
            b"DIIN" => {
                let mut p = body;
                let pend = body.saturating_add(size);
                while p.saturating_add(12) <= pend {
                    let mut sc = [0u8; 12];
                    read_exact_at(path, r, p, &mut sc)?;
                    let sid = [sc[0], sc[1], sc[2], sc[3]];
                    let ssize = u64::from_be_bytes(sc[4..12].try_into().unwrap());
                    if &sid == b"ID3 " {
                        id3 = read_id3_at(path, r, p + 12)?;
                    }
                    let Some(next) = next_chunk(p, ssize) else { break };
                    p = next;
                }
            }
            _ => {}
        }
        let Some(next) = next_chunk(pos, size) else { break };
        pos = next;
    }

    let rate = rate.ok_or_else(|| bad(path, "dff: missing FS chunk"))?;
    let channels = channels.ok_or_else(|| bad(path, "dff: missing CHNL chunk"))?;
    let (data_offset, data_len) = data.ok_or_else(|| bad(path, "dff: missing DSD data chunk"))?;
    // No more audio than the file holds, whatever the chunk says its size is
    let in_file = r
        .seek(SeekFrom::End(0))
        .map_err(|e| io_err(path, e))?
        .saturating_sub(data_offset);
    let data_len = data_len.min(in_file);
    if !(1..=8).contains(&channels) {
        return Err(bad(path, format!("dff: bad channel count {channels}")));
    }
    let format = AudioFormat::dsd(rate, channels as u8);
    format.validate().map_err(|e| bad(path, format!("dff: {e}")))?;
    Ok(DsdHeader {
        container: DsdContainer::Dff,
        format,
        sample_count: data_len.saturating_mul(8) / channels as u64,
        data_offset,
        data_len,
        block_size: 1,
        id3,
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
