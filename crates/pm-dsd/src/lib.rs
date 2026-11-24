//! DSF and DSDIFF (DFF) container headers: the format, where the sample data is and how it is
//! laid out, and the embedded ID3v2 tag. Headers only: reading the samples is pm-audio's job,
//! reading the tag's fields pm-library's. Both read the header here, so a file is understood
//! the same way by the scan that lists it and the player that plays it.
//!
//! Every size in a header is the file's word: sums are checked, a chunk whose size cannot be
//! ends the walk, the sample data is never longer than the file, and no more than [`MAX_TAG`]
//! bytes are set aside for a tag.

use pm_model::AudioFormat;
use std::io::{Read, Seek, SeekFrom};

/// The longest ID3v2 tag that is read (a tag's length field allows 256 MiB).
pub const MAX_TAG: u64 = 16 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Container {
    Dsf,
    Dff,
}

impl Container {
    pub fn name(&self) -> &'static str {
        match self {
            Container::Dsf => "dsf",
            Container::Dff => "dff",
        }
    }
}

/// What a DSF or DFF header says.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Header {
    pub container: Container,
    /// A DSD format that passed [`AudioFormat::validate`].
    pub format: AudioFormat,
    /// One-bit samples per channel.
    pub sample_count: u64,
    /// Byte offset of the first sample byte.
    pub data_offset: u64,
    /// Length in bytes of the sample data (DFF: at most what the file holds).
    pub data_len: u64,
    /// DSF: bytes per channel block (usually 4096). DFF: 1 (byte-interleaved).
    pub block_size: u32,
    /// DSF with one bit per sample: each byte holds its oldest sample in the least
    /// significant bit. DFF and 8-bit DSF: the most significant bit.
    pub lsb_first: bool,
    /// DFF only: the audio is compressed (DST), not plain DSD.
    pub compressed: bool,
    /// The raw ID3v2 tag, when asked for and present: DSF at its metadata pointer; DFF the
    /// first top-level `ID3 ` chunk, else the first one inside the first `DIIN` that has one
    /// (only those are tried). A tag that is missing, cut short, longer than its chunk or
    /// longer than [`MAX_TAG`] is none.
    pub id3: Option<Vec<u8>>,
}

impl Header {
    /// How long the audio plays, from the header's sample count.
    pub fn duration_ms(&self) -> Option<u64> {
        if self.format.rate_hz == 0 {
            None
        } else {
            Some(self.sample_count.checked_mul(1000)? / self.format.rate_hz as u64)
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Io(#[from] std::io::Error),
    /// Not a file this reads (another container, a compressed DSF).
    #[error("{0}")]
    Unsupported(String),
    /// A DSF or DFF file whose header is wrong.
    #[error("{0}")]
    Malformed(String),
}

type Result<T> = std::result::Result<T, Error>;

fn malformed(s: impl Into<String>) -> Error {
    Error::Malformed(s.into())
}

/// Read the header of a DSF or DFF file from its start, sniffing the magic bytes; with `tag`,
/// the ID3v2 tag too. The reader's position afterwards is unspecified.
pub fn read<R: Read + Seek>(r: &mut R, tag: bool) -> Result<Header> {
    let mut magic = [0u8; 4];
    read_at(r, 0, &mut magic)?;
    match &magic {
        b"DSD " => read_dsf(r, tag),
        b"FRM8" => read_dff(r, tag),
        _ => Err(Error::Unsupported("not a DSF or DFF file".into())),
    }
}

fn read_at<R: Read + Seek>(r: &mut R, off: u64, buf: &mut [u8]) -> std::io::Result<()> {
    r.seek(SeekFrom::Start(off))?;
    r.read_exact(buf)
}

fn le64(b: &[u8]) -> u64 {
    u64::from_le_bytes(b[..8].try_into().unwrap())
}

fn le32(b: &[u8]) -> u32 {
    u32::from_le_bytes(b[..4].try_into().unwrap())
}

fn be64(b: &[u8]) -> u64 {
    u64::from_be_bytes(b[..8].try_into().unwrap())
}

fn dsd_format(rate: u32, channels: u32, what: &str) -> Result<AudioFormat> {
    if !(1..=8).contains(&channels) {
        return Err(malformed(format!("{what}: bad channel count {channels}")));
    }
    let format = AudioFormat::dsd(rate, channels as u8);
    format.validate().map_err(|e| malformed(format!("{what}: {e}")))?;
    Ok(format)
}

fn read_dsf<R: Read + Seek>(r: &mut R, tag: bool) -> Result<Header> {
    let mut hdr = [0u8; 28];
    read_at(r, 0, &mut hdr)?;
    if le64(&hdr[4..]) != 28 {
        return Err(malformed("dsf: DSD chunk size is not 28"));
    }
    let metadata_ptr = le64(&hdr[20..]);

    let mut fmt = [0u8; 52];
    read_at(r, 28, &mut fmt)?;
    if &fmt[0..4] != b"fmt " {
        return Err(malformed("dsf: missing fmt chunk"));
    }
    let fmt_size = le64(&fmt[4..]);
    let format_id = le32(&fmt[16..]);
    let channels = le32(&fmt[24..]);
    let rate = le32(&fmt[28..]);
    let bits = le32(&fmt[32..]);
    let sample_count = le64(&fmt[36..]);
    let block_size = le32(&fmt[44..]);
    if format_id != 0 {
        return Err(Error::Unsupported(format!(
            "dsf: unsupported format id {format_id}"
        )));
    }
    let lsb_first = match bits {
        1 => true,
        8 => false,
        other => return Err(malformed(format!("dsf: bits per sample {other} not 1 or 8"))),
    };
    let format = dsd_format(rate, channels, "dsf")?;

    let data_hdr = 28u64
        .checked_add(fmt_size)
        .ok_or_else(|| malformed("dsf: fmt chunk size"))?;
    let mut data = [0u8; 12];
    read_at(r, data_hdr, &mut data)?;
    if &data[0..4] != b"data" {
        return Err(malformed("dsf: missing data chunk"));
    }
    let id3 = if tag && metadata_ptr != 0 {
        read_tag(r, metadata_ptr, MAX_TAG)
    } else {
        None
    };
    Ok(Header {
        container: Container::Dsf,
        format,
        sample_count,
        data_offset: data_hdr + 12,
        data_len: le64(&data[4..]).saturating_sub(12),
        block_size,
        lsb_first,
        compressed: false,
        id3,
    })
}

/// The ID3v2 tag at `off` (header and the synchsafe size, plus a footer when flagged), or
/// none when there is no tag there, it is cut short, or it is longer than `room` (a DFF chunk's
/// size) or [`MAX_TAG`].
fn read_tag<R: Read + Seek>(r: &mut R, off: u64, room: u64) -> Option<Vec<u8>> {
    let mut h = [0u8; 10];
    read_at(r, off, &mut h).ok()?;
    if &h[0..3] != b"ID3" {
        return None;
    }
    let size = h[6..10]
        .iter()
        .fold(0u64, |acc, b| (acc << 7) | (*b as u64 & 0x7f));
    let footer = if h[5] & 0x10 != 0 { 10 } else { 0 };
    let total = 10 + size + footer;
    if total > MAX_TAG.min(room) {
        return None;
    }
    let mut buf = vec![0u8; total as usize];
    read_at(r, off, &mut buf).ok()?;
    Some(buf)
}

/// Where the DSDIFF chunk after the one at `at` starts (a 12-byte header, `size` bytes, padded
/// to an even length), or `None` when the size is no size a file can have (the sum does not fit).
fn next_chunk(at: u64, size: u64) -> Option<u64> {
    at.checked_add(12)?.checked_add(size)?.checked_add(size & 1)
}

/// Walk the sub-chunks of a chunk body from `from` to `end`, calling `each` with the reader,
/// the id, where its body starts and its size; `each` answers whether to go on.
fn sub_chunks<R: Read + Seek>(
    r: &mut R,
    from: u64,
    end: u64,
    mut each: impl FnMut(&mut R, [u8; 4], u64, u64) -> Result<bool>,
) -> Result<()> {
    let mut p = from;
    while p.saturating_add(12) <= end {
        let mut sc = [0u8; 12];
        read_at(r, p, &mut sc)?;
        let size = be64(&sc[4..]);
        if !each(r, [sc[0], sc[1], sc[2], sc[3]], p + 12, size)? {
            break;
        }
        let Some(next) = next_chunk(p, size) else { break };
        p = next;
    }
    Ok(())
}

fn read_dff<R: Read + Seek>(r: &mut R, tag: bool) -> Result<Header> {
    let mut hdr = [0u8; 16];
    read_at(r, 0, &mut hdr)?;
    if &hdr[12..16] != b"DSD " {
        return Err(Error::Unsupported("dff: form type is not DSD".into()));
    }
    let end = be64(&hdr[4..]).saturating_add(12);

    let mut rate = None;
    let mut channels = None;
    let mut compressed = false;
    let mut data: Option<(u64, u64)> = None;
    // (one attempt each: a tag that does not read is not looked for again in a later chunk)
    let mut top_tag: Option<Option<Vec<u8>>> = None;
    let mut diin_tag: Option<Option<Vec<u8>>> = None;

    let mut pos = 16u64;
    while pos.saturating_add(12) <= end {
        let mut ch = [0u8; 12];
        if read_at(r, pos, &mut ch).is_err() {
            break;
        }
        let size = be64(&ch[4..]);
        let body = pos + 12;
        match &ch[0..4] {
            b"PROP" => {
                let mut kind = [0u8; 4];
                read_at(r, body, &mut kind)?;
                if &kind == b"SND " {
                    sub_chunks(r, body + 4, body.saturating_add(size), |r, id, at, _| {
                        match &id {
                            b"FS  " => {
                                let mut v = [0u8; 4];
                                read_at(r, at, &mut v)?;
                                rate = Some(u32::from_be_bytes(v));
                            }
                            b"CHNL" => {
                                let mut v = [0u8; 2];
                                read_at(r, at, &mut v)?;
                                channels = Some(u16::from_be_bytes(v) as u32);
                            }
                            b"CMPR" => {
                                // (a type cut short is not plain DSD either)
                                let mut v = [0u8; 4];
                                compressed |= read_at(r, at, &mut v).is_err() || &v != b"DSD ";
                            }
                            _ => {}
                        }
                        Ok(true)
                    })?;
                }
            }
            b"DSD " => data = Some((body, size)),
            b"DST " => compressed = true,
            b"ID3 " if tag && top_tag.is_none() => top_tag = Some(read_tag(r, body, size)),
            b"DIIN" if tag && diin_tag.is_none() => {
                sub_chunks(r, body, body.saturating_add(size), |r, id, at, size| {
                    if &id == b"ID3 " {
                        diin_tag = Some(read_tag(r, at, size));
                    }
                    Ok(diin_tag.is_none())
                })?;
            }
            _ => {}
        }
        let Some(next) = next_chunk(pos, size) else { break };
        pos = next;
    }

    let rate = rate.ok_or_else(|| malformed("dff: missing FS chunk"))?;
    let channels = channels.ok_or_else(|| malformed("dff: missing CHNL chunk"))?;
    let format = dsd_format(rate, channels, "dff")?;
    let (data_offset, data_len) = match data {
        Some(d) => d,
        // (a DST file has its audio in a DST chunk and no DSD chunk)
        None if compressed => return Err(Error::Unsupported("dff: DST compressed audio".into())),
        None => return Err(malformed("dff: missing DSD data chunk")),
    };
    // No more audio than the file holds, whatever the chunk says its size is
    let in_file = r.seek(SeekFrom::End(0))?.saturating_sub(data_offset);
    let data_len = data_len.min(in_file);
    Ok(Header {
        container: Container::Dff,
        format,
        sample_count: data_len.saturating_mul(8) / channels as u64,
        data_offset,
        data_len,
        block_size: 1,
        lsb_first: false,
        compressed,
        id3: top_tag.flatten().or(diin_tag.flatten()),
    })
}

#[cfg(test)]
mod tests;
