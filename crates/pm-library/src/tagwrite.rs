//! "Write IDs into my files" (identify brief; off by default): after a file
//! is identified, its recording MBID (and release MBID, AcoustID id when
//! known) can be written into its own tags, so other players and taggers see
//! them. Before anything changes, the tags the write touches are saved to a
//! JSON backup next to the node's data, and only the ID fields change.
//!
//! A write that stops half-way (an error on a share, the node stopped) does not cost the audio.
//! FLAC tags that fit the file's metadata area replace those bytes in place and the audio after
//! them is not written (a write cut there can leave the tags unreadable, never the audio).
//! Otherwise the file is written anew next to itself and takes the original's place only once
//! all of the audio is in it (`replace_file`). An MP3's tag sits in front of its audio, so it is
//! always written that way. WAV and AIFF keep their tag in a chunk of its own, written in place
//! by the `id3` crate as before.
//!
//! Formats: FLAC (Vorbis comments, the names Picard uses), MP3, WAV and AIFF (ID3
//! `TXXX` frames, as the scanner reads them). ALAC/MP4 and DSD are reported as
//! not supported rather than risked.

use id3::TagLike;
use pm_model::Mbid;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// What to write.
#[derive(Debug, Clone, PartialEq)]
pub struct Ids {
    pub recording: Mbid,
    pub release: Option<Mbid>,
    pub acoustid: Option<String>,
}

/// Fixes for a file's tags (owner, 2026-10-22: "a writer that fixes this"): the IDs a match
/// found, and the artists as the node knows them, one value each (`ARTISTS`, with their
/// MusicBrainz ids when known), so other players see "Sefa" and "Dr. Peacock", not one artist
/// called "Sefa, Dr. Peacock". The display credit (`ARTIST`/`TPE1`) is left as it is.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Fixes {
    pub ids: Option<Ids>,
    pub artists: Vec<String>,
    /// One per artist, in the same order, or empty.
    pub artist_mbids: Vec<Mbid>,
}

/// The saved state of a file's ID tags before a write.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Backup {
    pub path: PathBuf,
    /// `flac`, `wav` or `aiff`.
    pub format: String,
    pub saved_at: i64,
    /// FLAC: every Vorbis comment as it was (key, value), in order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub vorbis: Vec<(String, String)>,
    /// ID3: the `TXXX` frames the write touches, `None` when absent.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub txxx: Vec<(String, Option<String>)>,
    /// Whether the file had an ID3 tag at all.
    #[serde(default)]
    pub had_id3: bool,
}

#[derive(Debug, thiserror::Error)]
pub enum WriteError {
    #[error("{0}: writing IDs into this format is not supported")]
    Unsupported(String),
    #[error("{path}: {reason}")]
    Failed { path: PathBuf, reason: String },
}

fn failed(path: &Path, e: impl std::fmt::Display) -> WriteError {
    WriteError::Failed {
        path: path.to_path_buf(),
        reason: e.to_string(),
    }
}

const FLAC_KEYS: [&str; 5] = [
    "MUSICBRAINZ_TRACKID",
    "MUSICBRAINZ_ALBUMID",
    "ACOUSTID_ID",
    "ARTISTS",
    "MUSICBRAINZ_ARTISTID",
];
const TXXX_KEYS: [&str; 5] = [
    "MusicBrainz Track Id",
    "MusicBrainz Album Id",
    "Acoustid Id",
    "ARTISTS",
    "MusicBrainz Artist Id",
];

fn format_of(path: &Path) -> Option<&'static str> {
    match path.extension()?.to_str()?.to_ascii_lowercase().as_str() {
        "flac" => Some("flac"),
        "wav" | "wave" => Some("wav"),
        "aif" | "aiff" | "aifc" => Some("aiff"),
        "mp3" => Some("mp3"),
        _ => None,
    }
}

/// The values per key (several for the artist keys); empty = leave that key alone.
fn values(f: &Fixes) -> [Vec<String>; 5] {
    let ids = f.ids.as_ref();
    let artist_ids_fit = !f.artists.is_empty() && f.artist_mbids.len() == f.artists.len();
    [
        ids.map(|i| i.recording.to_string()).into_iter().collect(),
        ids.and_then(|i| i.release)
            .map(|r| r.to_string())
            .into_iter()
            .collect(),
        ids.and_then(|i| i.acoustid.clone()).into_iter().collect(),
        // One artist needs no ARTISTS tag (the display credit already says it)
        if f.artists.len() > 1 {
            f.artists.clone()
        } else {
            Vec::new()
        },
        if artist_ids_fit {
            f.artist_mbids.iter().map(|m| m.to_string()).collect()
        } else {
            Vec::new()
        },
    ]
}

/// Write `ids` into the file at `path`, after saving a backup under
/// `backup_dir`. Returns the backup's path.
pub fn write_ids(path: &Path, ids: &Ids, backup_dir: &Path) -> Result<PathBuf, WriteError> {
    write_fixes(
        path,
        &Fixes {
            ids: Some(ids.clone()),
            ..Default::default()
        },
        backup_dir,
    )
}

/// One tag a fix changes: `(key, values now, values after)`.
pub type Change = (String, Vec<String>, Vec<String>);

/// What [`write_fixes`] would change in the file: `(key, now, then)` for each key that differs.
pub fn preview(path: &Path, f: &Fixes) -> Result<Vec<Change>, WriteError> {
    let format = format_of(path).ok_or_else(|| WriteError::Unsupported(path.display().to_string()))?;
    let vals = values(f);
    let mut out = Vec::new();
    if format == "flac" {
        let tag = metaflac::Tag::read_from_path(path).map_err(|e| failed(path, e))?;
        for (k, v) in FLAC_KEYS.iter().zip(vals.iter()) {
            if v.is_empty() {
                continue;
            }
            let now: Vec<String> = tag
                .get_vorbis(k)
                .map(|i| i.map(str::to_string).collect())
                .unwrap_or_default();
            if &now != v {
                out.push(((*k).to_string(), now, v.clone()));
            }
        }
    } else {
        let tag = id3::Tag::read_from_path(path).unwrap_or_default();
        for (d, v) in TXXX_KEYS.iter().zip(vals.iter()) {
            if v.is_empty() {
                continue;
            }
            let now: Vec<String> = tag
                .extended_texts()
                .find(|t| t.description == *d)
                .map(|t| t.value.split('\0').map(str::to_string).collect())
                .unwrap_or_default();
            if &now != v {
                out.push(((*d).to_string(), now, v.clone()));
            }
        }
    }
    Ok(out)
}

/// Write `f` into the file at `path`, after saving a backup under `backup_dir` (only the keys
/// it touches; the audio is never rewritten). Returns the backup's path.
pub fn write_fixes(path: &Path, f: &Fixes, backup_dir: &Path) -> Result<PathBuf, WriteError> {
    write_with(path, f, None, backup_dir)
}

/// The owner corrected a match ("Wrong match?"): the file's recording, release and AcoustID IDs
/// are taken out, then `ids` written (none: "Not matched", the IDs are only removed), after the
/// same backup as [`write_ids`]. Returns the backup's path.
///
/// Every recording id the scanner reads goes (Picard's `MUSICBRAINZ_TRACKID` and
/// `MUSICBRAINZ_RECORDINGID`), with the AcoustID id; the album's release id is kept unless `ids`
/// names a release (then it is written); `drop_isrc` takes the ISRC too
/// (a song left without a match that its ISRC would name again), which only FLAC allows here.
pub fn replace_ids(
    path: &Path,
    ids: Option<&Ids>,
    drop_isrc: bool,
    backup_dir: &Path,
) -> Result<PathBuf, WriteError> {
    can_replace(path, drop_isrc)?;
    let f = Fixes {
        ids: ids.cloned(),
        ..Fixes::default()
    };
    write_with(path, &f, Some(drop_isrc), backup_dir)
}

/// Whether [`replace_ids`] can do its work on this file: a format it writes, there and writable,
/// and (`drop_isrc`) an ISRC it can take out. Checked before anything is changed.
pub fn can_replace(path: &Path, drop_isrc: bool) -> Result<(), WriteError> {
    let format = format_of(path).ok_or_else(|| WriteError::Unsupported(path.display().to_string()))?;
    let meta = std::fs::metadata(path).map_err(|e| failed(path, e))?;
    if meta.permissions().readonly() {
        return Err(failed(path, "the file is read-only"));
    }
    if drop_isrc && format != "flac" {
        return Err(failed(path, "its ISRC can't be taken out of this format here"));
    }
    Ok(())
}

/// What a correction clears in FLAC: every recording id the scanner reads, and AcoustID's. The
/// album's release id stays (the album was right; it is replaced only when the owner names a
/// release): taking it out would move the song out of its album at the next scan.
const CLEAR_FLAC: [&str; 3] = ["MUSICBRAINZ_TRACKID", "MUSICBRAINZ_RECORDINGID", "ACOUSTID_ID"];
/// The same as ID3 `TXXX` frames.
const CLEAR_TXXX: [&str; 3] = ["MusicBrainz Track Id", "MusicBrainz Recording Id", "Acoustid Id"];

/// [`write_fixes`]; `clear` (a correction) takes the ids [`CLEAR_FLAC`] / [`CLEAR_TXXX`] out first,
/// and the ISRC too when it holds `true`.
fn write_with(path: &Path, f: &Fixes, clear: Option<bool>, backup_dir: &Path) -> Result<PathBuf, WriteError> {
    let format = format_of(path).ok_or_else(|| WriteError::Unsupported(path.display().to_string()))?;
    std::fs::create_dir_all(backup_dir).map_err(|e| failed(backup_dir, e))?;
    let vals = values(f);
    let mut backup = Backup {
        path: path.to_path_buf(),
        format: format.into(),
        saved_at: crate::db::now_unix(),
        vorbis: Vec::new(),
        txxx: Vec::new(),
        had_id3: false,
    };
    match format {
        "flac" => {
            let mut tag = metaflac::Tag::read_from_path(path).map_err(|e| failed(path, e))?;
            if let Some(vc) = tag.vorbis_comments() {
                for (k, vs) in &vc.comments {
                    for v in vs {
                        backup.vorbis.push((k.clone(), v.clone()));
                    }
                }
            }
            let bpath = save_backup(&backup, backup_dir)?;
            if let Some(drop_isrc) = clear {
                for k in CLEAR_FLAC {
                    tag.remove_vorbis(k);
                }
                if drop_isrc {
                    tag.remove_vorbis("ISRC");
                }
            }
            for (k, v) in FLAC_KEYS.iter().zip(vals.iter()) {
                if !v.is_empty() {
                    tag.set_vorbis(*k, v.clone());
                }
            }
            save_flac(path, &mut tag)?;
            Ok(bpath)
        }
        _ => {
            let mut tag = match id3::Tag::read_from_path(path) {
                Ok(t) => {
                    backup.had_id3 = true;
                    t
                }
                Err(e) if matches!(e.kind, id3::ErrorKind::NoTag) => id3::Tag::new(),
                Err(e) => return Err(failed(path, e)),
            };
            // (a correction clears one more: it is kept in the backup as well)
            let extra: &[&str] = if clear.is_some() {
                &["MusicBrainz Recording Id"]
            } else {
                &[]
            };
            for d in TXXX_KEYS.iter().chain(extra) {
                let cur = tag
                    .extended_texts()
                    .find(|t| t.description == *d)
                    .map(|t| t.value.clone());
                backup.txxx.push((d.to_string(), cur));
            }
            let bpath = save_backup(&backup, backup_dir)?;
            if clear.is_some() {
                for d in CLEAR_TXXX {
                    tag.remove_extended_text(Some(d), None);
                }
            }
            for (d, v) in TXXX_KEYS.iter().zip(vals.iter()) {
                if !v.is_empty() {
                    tag.remove_extended_text(Some(d), None);
                    // ID3v2.4: several values in one frame, NUL-separated (as Picard writes them)
                    tag.add_frame(id3::frame::ExtendedText {
                        description: (*d).into(),
                        value: v.join("\0"),
                    });
                }
            }
            save_id3(path, &tag)?;
            Ok(bpath)
        }
    }
}

fn save_backup(b: &Backup, dir: &Path) -> Result<PathBuf, WriteError> {
    let name = format!(
        "{}-{}.json",
        &blake3::hash(b.path.to_string_lossy().as_bytes()).to_hex()[..16],
        b.saved_at
    );
    let p = dir.join(name);
    let json = serde_json::to_vec_pretty(b).map_err(|e| failed(&p, e))?;
    std::fs::write(&p, json).map_err(|e| failed(&p, e))?;
    Ok(p)
}

/// Padding a rewritten FLAC file gets after its tags, so the next write fits in place.
const REWRITE_PADDING: u32 = 4096;
/// The longest a FLAC metadata block can say it is (24 bits).
const MAX_BLOCK: u64 = 0x00FF_FFFF;

/// Copy exactly `want` bytes; fewer (the source ended, or stopped answering) is an error.
fn copy_exact(from: &mut dyn std::io::Read, to: &mut dyn std::io::Write, want: u64) -> std::io::Result<()> {
    use std::io::Read as _;
    let got = std::io::copy(&mut from.take(want), to)?;
    if got != want {
        return Err(std::io::Error::new(
            std::io::ErrorKind::UnexpectedEof,
            format!("{got} of {want} bytes could be read"),
        ));
    }
    Ok(())
}

/// The file `write` fills, next to `path` (the same folder, so the same disk). Its name is
/// short whatever the song's is (a long file name plus a suffix can be longer than a name may
/// be) and the same for the same file, so one left by a write that never finished is found and
/// removed by the next.
fn rewrite_path(path: &Path) -> PathBuf {
    let name = path.file_name().unwrap_or_default().to_string_lossy();
    // FNV-1a: stable from one build of the node to the next
    let hash = name.bytes().fold(0xcbf2_9ce4_8422_2325u64, |h, b| {
        (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3)
    });
    path.with_file_name(format!(".odeion-write-{hash:016x}"))
}

/// Replace the file at `path` by what `write` puts into a new file next to it. The original is
/// touched only by the last step, a rename, after `write` succeeded and the new file reached the
/// disk; when anything fails before that, the new file is removed and the original is as it was.
/// A link is followed: the file it names is the one replaced, and the link stays a link.
fn replace_file(
    path: &Path,
    write: impl FnOnce(&mut std::fs::File) -> Result<(), WriteError>,
) -> Result<(), WriteError> {
    let real = std::fs::canonicalize(path).map_err(|e| failed(path, e))?;
    let path = real.as_path();
    let meta = std::fs::metadata(path).map_err(|e| failed(path, e))?;
    let new = rewrite_path(path);
    // (one left by a write the node did not live to finish)
    let _ = std::fs::remove_file(&new);
    let made = (|| {
        let mut file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(&new)
            .map_err(|e| failed(&new, e))?;
        write(&mut file)?;
        file.sync_all().map_err(|e| failed(&new, e))?;
        // As the original was: readable by the same people
        std::fs::set_permissions(&new, meta.permissions()).map_err(|e| failed(&new, e))?;
        std::fs::rename(&new, path).map_err(|e| failed(path, e))?;
        // The rename itself on the disk (not every filesystem lets a folder be synced)
        if let Some(Ok(dir)) = path.parent().map(std::fs::File::open) {
            let _ = dir.sync_all();
        }
        Ok(())
    })();
    if made.is_err() {
        let _ = std::fs::remove_file(&new);
    }
    made
}

/// Where a FLAC file's metadata is: from the `fLaC` mark (after an ID3 tag some taggers put in
/// front) to the first audio frame.
fn flac_metadata_span(file: &mut std::fs::File, path: &Path) -> Result<(u64, u64), WriteError> {
    use std::io::{Read, Seek, SeekFrom};
    let io = |e: std::io::Error| failed(path, e);
    let len = file.metadata().map_err(io)?.len();
    file.seek(SeekFrom::Start(0)).map_err(io)?;
    let mut mark = [0u8; 4];
    file.read_exact(&mut mark).map_err(io)?;
    let mut start = 0;
    if &mark[..3] == b"ID3" {
        start = id3_front_len(file).map_err(io)?;
        file.seek(SeekFrom::Start(start)).map_err(io)?;
        file.read_exact(&mut mark).map_err(io)?;
    }
    if &mark != b"fLaC" {
        return Err(failed(path, "no FLAC stream where it should start"));
    }
    let mut end = start + 4;
    loop {
        let mut header = [0u8; 4];
        file.read_exact(&mut header).map_err(io)?;
        let block = (u64::from(header[1]) << 16) | (u64::from(header[2]) << 8) | u64::from(header[3]);
        end += 4 + block;
        if end > len {
            return Err(failed(path, "a metadata block runs past the end of the file"));
        }
        if header[0] & 0x80 != 0 {
            return Ok((start, end));
        }
        file.seek(SeekFrom::Start(end)).map_err(io)?;
    }
}

/// The length of the ID3v2 tag at the start of `file` (0 when there is none).
fn id3_front_len(file: &mut std::fs::File) -> std::io::Result<u64> {
    use std::io::{Read, Seek, SeekFrom};
    file.seek(SeekFrom::Start(0))?;
    let mut header = [0u8; 10];
    let mut read = 0;
    while read < header.len() {
        match file.read(&mut header[read..])? {
            0 => break,
            n => read += n,
        }
    }
    if read < header.len() || &header[..3] != b"ID3" {
        return Ok(0);
    }
    let size = header[6..]
        .iter()
        .fold(0u64, |n, b| (n << 7) | u64::from(b & 0x7f));
    let footer = if header[5] & 0x10 != 0 { 10 } else { 0 };
    Ok(10 + size + footer)
}

/// Where the audio of a file with an ID3 tag in front starts: after the tag and the zero bytes
/// that follow it. The `id3` crate takes those zeros as the tag's padding and does not keep them
/// when it writes a tag of another length, so they are no part of what must stay the same.
fn id3_audio_start(file: &mut std::fs::File) -> std::io::Result<u64> {
    use std::io::{BufRead, Seek, SeekFrom};
    let mut at = id3_front_len(file)?;
    file.seek(SeekFrom::Start(at))?;
    let mut reader = std::io::BufReader::new(file);
    loop {
        let buf = reader.fill_buf()?;
        if buf.is_empty() {
            return Ok(at);
        }
        let zeros = buf.iter().take_while(|b| **b == 0).count();
        at += zeros as u64;
        if zeros < buf.len() {
            return Ok(at);
        }
        reader.consume(zeros);
    }
}

/// Whether the length the ID3v2 header in front of `file` claims covers bytes that are neither
/// frames nor zero padding: the audio, when the length field is too large. The writer replaces
/// everything up to that length, so such a tag must not be rewritten. `None` when this does not
/// walk the tag's layout (no tag, a version other than 2.3 or 2.4, an extended header,
/// unsynchronisation, frames that run past the claimed end): no evidence either way.
fn id3_length_covers_other_bytes(file: &mut std::fs::File, len: u64) -> std::io::Result<Option<bool>> {
    use std::io::{BufRead, Read, Seek, SeekFrom};
    let claimed = id3_front_len(file)?;
    if claimed < 10 {
        return Ok(None);
    }
    let mut header = [0u8; 10];
    file.seek(SeekFrom::Start(0))?;
    file.read_exact(&mut header)?;
    let (version, flags) = (header[3], header[5]);
    if !(version == 3 || version == 4) || flags & 0xC0 != 0 {
        return Ok(None);
    }
    // (a footer, 2.4 only, is the last 10 bytes of what the header claims)
    let footer = if flags & 0x10 != 0 { 10 } else { 0 };
    let end = claimed.min(len).saturating_sub(footer);
    let mut pos = 10u64;
    while pos + 10 <= end {
        let mut frame = [0u8; 10];
        file.seek(SeekFrom::Start(pos))?;
        file.read_exact(&mut frame)?;
        if frame[0] == 0 {
            break;
        }
        let size = if version == 4 {
            frame[4..8]
                .iter()
                .fold(0u64, |n, b| (n << 7) | u64::from(b & 0x7f))
        } else {
            u64::from(u32::from_be_bytes([frame[4], frame[5], frame[6], frame[7]]))
        };
        pos += 10 + size;
        if pos > end {
            return Ok(None);
        }
    }
    // What follows the frames, up to the claimed end, is padding: zeros and nothing else
    file.seek(SeekFrom::Start(pos))?;
    let mut rest = std::io::BufReader::new(file.take(end.saturating_sub(pos)));
    loop {
        let buf = rest.fill_buf()?;
        if buf.is_empty() {
            return Ok(Some(false));
        }
        if buf.iter().any(|b| *b != 0) {
            return Ok(Some(true));
        }
        let n = buf.len();
        rest.consume(n);
    }
}

/// Save `tag` into the FLAC file at `path`. Tags that fit the file's metadata area replace
/// exactly its bytes, in place; the audio after it is not written at all. Tags that need more
/// room go into a new file with the same audio (`replace_file`).
fn save_flac(path: &Path, tag: &mut metaflac::Tag) -> Result<(), WriteError> {
    use std::io::{Seek, SeekFrom, Write};
    let io = |e: std::io::Error| failed(path, e);
    let mut file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(path)
        .map_err(io)?;
    let len = file.metadata().map_err(io)?.len();
    let (start, end) = flac_metadata_span(&mut file, path)?;
    let room = end - start;
    tag.remove_blocks(metaflac::BlockType::Padding);
    let mut head = Vec::new();
    tag.write_to(&mut head).map_err(|e| failed(path, e))?;
    let bare = head.len() as u64;
    if bare != room {
        // A padding block (4 bytes and its length) takes up what is left of the room; tags too
        // long for the room get a new file, with room for the next write
        let padding = match room.checked_sub(bare + 4) {
            Some(left) if left <= MAX_BLOCK => left as u32,
            _ => REWRITE_PADDING,
        };
        tag.push_block(metaflac::Block::Padding(padding));
        head.clear();
        tag.write_to(&mut head).map_err(|e| failed(path, e))?;
    }
    if head.len() as u64 == room {
        file.seek(SeekFrom::Start(start)).map_err(io)?;
        file.write_all(&head).map_err(io)?;
        return file.sync_data().map_err(io);
    }
    // (`move`: the original is closed when the copy is made, before the rename over it, which
    // some filesystems refuse while it is open)
    replace_file(path, move |new| {
        // What is in front of the stream, the new tags, then the audio: all of it, or no new file
        file.seek(SeekFrom::Start(0)).map_err(io)?;
        copy_exact(&mut file, new, start).map_err(io)?;
        new.write_all(&head).map_err(io)?;
        file.seek(SeekFrom::Start(end)).map_err(io)?;
        copy_exact(&mut file, new, len - end).map_err(io)
    })
}

/// Save `tag` into the file at `path`. A WAV or AIFF file holds the tag in a chunk of its own
/// and is written in place. Any other file (an MP3) has the tag in front of its audio, and a tag
/// of another length moves all of the audio: that is done in a copy, which takes the original's
/// place only when its audio (what follows the tag and its padding, [`id3_audio_start`]) is as
/// long as the original's.
fn save_id3(path: &Path, tag: &id3::Tag) -> Result<(), WriteError> {
    use std::io::{Read, Seek, SeekFrom};
    let io = |e: std::io::Error| failed(path, e);
    // Opened for writing although only read here: a file the node may not write is not replaced
    // either (a new file in a folder it may write would get around that)
    let mut file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(path)
        .map_err(io)?;
    let mut mark = [0u8; 4];
    let chunked = file.read_exact(&mut mark).is_ok() && (&mark == b"RIFF" || &mark == b"FORM");
    if chunked {
        drop(file);
        return tag
            .write_to_path(path, id3::Version::Id3v24)
            .map_err(|e| failed(path, e));
    }
    let len = file.metadata().map_err(io)?.len();
    let audio = len.saturating_sub(id3_audio_start(&mut file).map_err(io)?);
    // A tag whose length covers bytes that are no part of it (the audio) must not be rewritten:
    // the writer replaces everything up to that length, and the comparison below would not see
    // it. Where the tag's layout is not walked, one that claims the whole of a file leaves
    // nothing to compare either, and is left alone too
    let covers = id3_length_covers_other_bytes(&mut file, len).map_err(io)?;
    let tagged = id3_front_len(&mut file).map_err(io)? >= 10;
    if covers == Some(true) || (covers.is_none() && tagged && audio == 0) {
        return Err(failed(
            path,
            "the tag's length covers more than the tag; writing it would cut into the audio",
        ));
    }
    replace_file(path, move |new| {
        file.seek(SeekFrom::Start(0)).map_err(io)?;
        copy_exact(&mut file, new, len).map_err(io)?;
        new.seek(SeekFrom::Start(0)).map_err(io)?;
        tag.write_to_file(&mut *new, id3::Version::Id3v24)
            .map_err(|e| failed(path, e))?;
        let now = new.metadata().map_err(io)?.len();
        let kept = now.saturating_sub(id3_audio_start(new).map_err(io)?);
        if kept != audio {
            return Err(failed(
                path,
                format!("the rewritten file holds {kept} bytes of audio, the original {audio}"),
            ));
        }
        Ok(())
    })
}

/// Put the ID tags back as the backup has them.
pub fn restore(backup: &Backup) -> Result<(), WriteError> {
    let path = &backup.path;
    match backup.format.as_str() {
        "flac" => {
            let mut tag = metaflac::Tag::read_from_path(path).map_err(|e| failed(path, e))?;
            // (the backup holds every comment: a correction's keys come back too)
            let keys: Vec<&str> = FLAC_KEYS
                .iter()
                .copied()
                .chain(["MUSICBRAINZ_RECORDINGID", "ISRC"])
                .collect();
            for k in &keys {
                tag.remove_vorbis(k);
            }
            for k in keys {
                let vs: Vec<String> = backup
                    .vorbis
                    .iter()
                    .filter(|(bk, _)| bk.eq_ignore_ascii_case(k))
                    .map(|(_, v)| v.clone())
                    .collect();
                if !vs.is_empty() {
                    tag.set_vorbis(k, vs);
                }
            }
            save_flac(path, &mut tag)
        }
        "wav" | "aiff" | "mp3" => {
            let mut tag = id3::Tag::read_from_path(path).map_err(|e| failed(path, e))?;
            for (d, v) in &backup.txxx {
                tag.remove_extended_text(Some(d), None);
                if let Some(v) = v {
                    tag.add_frame(id3::frame::ExtendedText {
                        description: d.clone(),
                        value: v.clone(),
                    });
                }
            }
            save_id3(path, &tag)
        }
        other => Err(WriteError::Unsupported(other.into())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MB: &str = "b1a9c0e9-d987-4042-ae91-78d6a3267d69";
    const REL: &str = "1a5b6a49-4c4f-3d18-9e3d-1c4d1b0a7a3e";

    fn ids() -> Ids {
        Ids {
            recording: Mbid::parse(MB).unwrap(),
            release: Some(Mbid::parse(REL).unwrap()),
            acoustid: Some("84eed5cd-5b1f-4f5c-8c92-2686d2877862".into()),
        }
    }

    /// Decoded samples, to prove the audio is untouched.
    fn samples(path: &Path) -> Vec<i32> {
        use symphonia::core::codecs::audio::AudioDecoderOptions;
        use symphonia::core::formats::probe::Hint;
        use symphonia::core::formats::{FormatOptions, TrackType};
        use symphonia::core::io::MediaSourceStream;
        use symphonia::core::meta::MetadataOptions;
        let mss = MediaSourceStream::new(Box::new(std::fs::File::open(path).unwrap()), Default::default());
        let mut hint = Hint::new();
        hint.with_extension(path.extension().unwrap().to_str().unwrap());
        let mut fmt = symphonia::default::get_probe()
            .probe(&hint, mss, FormatOptions::default(), MetadataOptions::default())
            .unwrap();
        let track = fmt.default_track(TrackType::Audio).unwrap();
        let id = track.id;
        let params = track.codec_params.as_ref().unwrap().audio().unwrap().clone();
        let mut dec = symphonia::default::get_codecs()
            .make_audio_decoder(&params, &AudioDecoderOptions::default())
            .unwrap();
        let mut out = Vec::new();
        while let Ok(Some(p)) = fmt.next_packet() {
            if p.track_id != id {
                continue;
            }
            let buf = dec.decode(&p).unwrap();
            let mut v = Vec::new();
            buf.copy_to_vec_interleaved::<i32>(&mut v);
            out.extend(v);
        }
        out
    }

    /// A correction ("Wrong match?") takes out every recording id the scanner reads (Picard's
    /// `MUSICBRAINZ_RECORDINGID` too), the release and AcoustID ids, and the ISRC when asked;
    /// the backup brings them all back. A file it can't change is refused before anything is.
    #[test]
    fn a_correction_clears_every_id_the_scan_reads() {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("a.flac");
        std::fs::copy(
            concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/tests/fixtures/ietf-60-mono-audio.flac"
            ),
            &f,
        )
        .unwrap();
        {
            let mut tag = metaflac::Tag::read_from_path(&f).unwrap();
            tag.set_vorbis("MUSICBRAINZ_RECORDINGID", vec![MB]);
            tag.set_vorbis("MUSICBRAINZ_TRACKID", vec![MB]);
            tag.set_vorbis("ISRC", vec!["USQX91300108"]);
            tag.set_vorbis("MUSICBRAINZ_ALBUMID", vec![REL]);
            tag.save().unwrap();
        }
        let (_, before) = crate::probe_and_read(&f).unwrap();
        assert_eq!(before.recording_mbid.map(|m| m.to_string()).as_deref(), Some(MB));
        assert!(before.isrc.is_some());
        let backups = dir.path().join("backups");
        let other = Mbid::parse("0383dadf-2a4e-4d10-a46a-e9e041da8eb3").unwrap();
        replace_ids(
            &f,
            Some(&Ids {
                recording: other,
                release: None,
                acoustid: None,
            }),
            false,
            &backups,
        )
        .unwrap();
        let (_, now) = crate::probe_and_read(&f).unwrap();
        assert_eq!(
            now.recording_mbid,
            Some(other),
            "no older recording id left to win"
        );
        assert!(now.isrc.is_some(), "kept unless asked");
        let backup = replace_ids(&f, None, true, &backups).unwrap();
        let (_, none) = crate::probe_and_read(&f).unwrap();
        assert_eq!((none.recording_mbid, none.isrc), (None, None));
        // Review r2: the album was right, its release id stays (the song stays in its album)
        assert_eq!(none.release_mbid.map(|m| m.to_string()).as_deref(), Some(REL));
        let b: Backup = serde_json::from_slice(&std::fs::read(backup).unwrap()).unwrap();
        restore(&b).unwrap();
        let (_, back) = crate::probe_and_read(&f).unwrap();
        assert_eq!(back.recording_mbid, Some(other));
        assert!(back.isrc.is_some());
        // Refused before anything changes: a missing file, an ISRC this format can't drop here
        assert!(can_replace(&dir.path().join("gone.flac"), false).is_err());
        let w = dir.path().join("a.wav");
        std::fs::write(&w, b"RIFF").unwrap();
        assert!(can_replace(&w, false).is_ok());
        assert!(can_replace(&w, true).is_err());
    }

    #[test]
    fn flac_ids_are_written_with_a_backup_and_restore() {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("a.flac");
        std::fs::copy(
            concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/tests/fixtures/ietf-60-mono-audio.flac"
            ),
            &f,
        )
        .unwrap();
        let before = samples(&f);
        let (_, tags_before) = crate::probe_and_read(&f).unwrap();
        assert!(tags_before.recording_mbid.is_none());
        let bpath = write_ids(&f, &ids(), &dir.path().join("backups")).unwrap();
        assert!(bpath.exists());
        let (_, tags) = crate::probe_and_read(&f).unwrap();
        assert_eq!(
            tags.recording_mbid.unwrap().to_string(),
            MB,
            "the scanner reads it back"
        );
        assert_eq!(samples(&f), before, "audio untouched");
        let b: Backup = serde_json::from_slice(&std::fs::read(&bpath).unwrap()).unwrap();
        assert!(b
            .vorbis
            .iter()
            .all(|(k, _)| !k.eq_ignore_ascii_case("MUSICBRAINZ_TRACKID")));
        restore(&b).unwrap();
        let (_, back) = crate::probe_and_read(&f).unwrap();
        assert!(back.recording_mbid.is_none(), "restored");
        assert_eq!(samples(&f), before);
    }

    /// The artists written one value each: the scanner reads them back as separate artists, the
    /// preview says what changes, and the backup puts the file back.
    #[test]
    fn flac_artists_are_written_one_per_value() {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("a.flac");
        std::fs::copy(
            concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/tests/fixtures/ietf-60-mono-audio.flac"
            ),
            &f,
        )
        .unwrap();
        let before = samples(&f);
        let fixes = Fixes {
            ids: None,
            artists: vec!["Sefa".into(), "Dr. Peacock".into()],
            artist_mbids: vec![],
        };
        let p = preview(&f, &fixes).unwrap();
        assert_eq!(p.len(), 1, "{p:?}");
        assert_eq!(p[0].0, "ARTISTS");
        assert_eq!(p[0].2, vec!["Sefa", "Dr. Peacock"]);
        let bpath = write_fixes(&f, &fixes, &dir.path().join("backups")).unwrap();
        let (_, tags) = crate::probe_and_read(&f).unwrap();
        assert_eq!(tags.artist_list, vec!["Sefa", "Dr. Peacock"]);
        assert!(preview(&f, &fixes).unwrap().is_empty(), "nothing left to change");
        assert_eq!(samples(&f), before, "audio untouched");
        let b: Backup = serde_json::from_slice(&std::fs::read(&bpath).unwrap()).unwrap();
        restore(&b).unwrap();
        let (_, back) = crate::probe_and_read(&f).unwrap();
        assert!(back.artist_list.is_empty(), "restored");
    }

    #[test]
    fn mp3_artists_go_into_one_id3_frame() {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("a.mp3");
        std::fs::write(&f, b"").unwrap();
        let fixes = Fixes {
            ids: None,
            artists: vec!["Sefa".into(), "Cyclon".into()],
            artist_mbids: vec![],
        };
        write_fixes(&f, &fixes, &dir.path().join("backups")).unwrap();
        let t = id3::Tag::read_from_path(&f).unwrap();
        let v = t
            .extended_texts()
            .find(|x| x.description == "ARTISTS")
            .unwrap()
            .value
            .clone();
        assert_eq!(v, "Sefa\0Cyclon");
        let mut back = crate::tags::TagSet::default();
        for et in t.extended_texts() {
            back.merge_from(crate::tags::from_id3(&t));
            let _ = et;
        }
        assert_eq!(back.artist_list, vec!["Sefa", "Cyclon"]);
    }

    #[test]
    fn wav_ids_go_into_an_id3_chunk() {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("a.wav");
        let spec = hound::WavSpec {
            channels: 2,
            sample_rate: 44_100,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut w = hound::WavWriter::create(&f, spec).unwrap();
        for i in 0..44_100 {
            let v = ((i as f64 * 0.05).sin() * 9000.0) as i16;
            w.write_sample(v).unwrap();
            w.write_sample(v).unwrap();
        }
        w.finalize().unwrap();
        let before = samples(&f);
        write_ids(&f, &ids(), &dir.path().join("backups")).unwrap();
        let tag = id3::Tag::read_from_path(&f).unwrap();
        let mb = tag
            .extended_texts()
            .find(|t| t.description == "MusicBrainz Track Id")
            .map(|t| t.value.clone());
        assert_eq!(mb.as_deref(), Some(MB));
        assert_eq!(samples(&f), before, "audio untouched");
    }

    #[test]
    fn other_formats_are_refused() {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("a.m4a");
        std::fs::write(&f, b"x").unwrap();
        assert!(matches!(
            write_ids(&f, &ids(), dir.path()),
            Err(WriteError::Unsupported(_))
        ));
        assert!(
            std::fs::read_dir(dir.path()).unwrap().count() == 1,
            "no backup written"
        );
    }

    /// What comes after the metadata: the audio frames, byte for byte.
    fn flac_audio(path: &Path) -> Vec<u8> {
        let mut f = std::fs::File::open(path).unwrap();
        let (_, end) = flac_metadata_span(&mut f, path).unwrap();
        std::fs::read(path).unwrap()[end as usize..].to_vec()
    }

    /// The fixture with no padding at all: its tags, then the audio. (metaflac's own writer
    /// fills the room it had with padding again, or adds 1024 bytes of it.)
    fn flac_without_padding(dir: &Path) -> PathBuf {
        let f = flac_fixture(dir);
        let audio = flac_audio(&f);
        let mut tag = metaflac::Tag::read_from_path(&f).unwrap();
        tag.remove_blocks(metaflac::BlockType::Padding);
        let mut bytes = Vec::new();
        tag.write_to(&mut bytes).unwrap();
        bytes.extend(&audio);
        std::fs::write(&f, bytes).unwrap();
        let mut file = std::fs::File::open(&f).unwrap();
        let (start, end) = flac_metadata_span(&mut file, &f).unwrap();
        assert_eq!(
            (start, end + audio.len() as u64),
            (0, std::fs::metadata(&f).unwrap().len())
        );
        assert!(metaflac::Tag::read_from_path(&f)
            .unwrap()
            .get_blocks(metaflac::BlockType::Padding)
            .next()
            .is_none());
        f
    }

    fn flac_fixture(dir: &Path) -> PathBuf {
        let f = dir.join("a.flac");
        std::fs::copy(
            concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/tests/fixtures/ietf-60-mono-audio.flac"
            ),
            &f,
        )
        .unwrap();
        f
    }

    /// Tags that fit a FLAC file's padding replace the same bytes of the same file: it keeps its
    /// length, and the audio after the tags is not written.
    #[cfg(unix)]
    #[test]
    fn flac_tags_that_fit_are_written_in_place() {
        use std::os::unix::fs::MetadataExt;
        let dir = tempfile::tempdir().unwrap();
        let f = flac_fixture(dir.path());
        // Room for the IDs, whatever the fixture came with
        let mut tag = metaflac::Tag::read_from_path(&f).unwrap();
        tag.remove_blocks(metaflac::BlockType::Padding);
        tag.push_block(metaflac::Block::Padding(8192));
        tag.write_to_path(&f).unwrap();
        let (audio, before) = (flac_audio(&f), std::fs::metadata(&f).unwrap());
        write_ids(&f, &ids(), &dir.path().join("backups")).unwrap();
        let after = std::fs::metadata(&f).unwrap();
        assert_eq!(after.ino(), before.ino(), "the same file, not a new one");
        assert_eq!(after.len(), before.len());
        assert_eq!(flac_audio(&f), audio);
        let (_, tags) = crate::probe_and_read(&f).unwrap();
        assert_eq!(tags.recording_mbid.unwrap().to_string(), MB);
        assert!(!rewrite_path(&f).exists());
    }

    /// Tags that need more room than the file has go into a new file that takes its place: the
    /// audio is the same byte for byte, the tags read back, the backup restores them, and the
    /// file is as readable as it was.
    #[cfg(unix)]
    #[test]
    fn flac_tags_that_do_not_fit_take_a_new_file_with_the_same_audio() {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        let dir = tempfile::tempdir().unwrap();
        let f = flac_without_padding(dir.path());
        std::fs::set_permissions(&f, std::fs::Permissions::from_mode(0o640)).unwrap();
        let (audio, heard, before) = (flac_audio(&f), samples(&f), std::fs::metadata(&f).unwrap());
        let bpath = write_ids(&f, &ids(), &dir.path().join("backups")).unwrap();
        let after = std::fs::metadata(&f).unwrap();
        assert_ne!(after.ino(), before.ino(), "a new file took its place");
        assert!(after.len() > before.len());
        assert_eq!(after.permissions().mode() & 0o777, 0o640);
        assert_eq!(flac_audio(&f), audio);
        assert_eq!(samples(&f), heard);
        let (_, tags) = crate::probe_and_read(&f).unwrap();
        assert_eq!(tags.recording_mbid.unwrap().to_string(), MB);
        assert!(!rewrite_path(&f).exists(), "nothing is left next to it");
        // The next write fits the room the new file got
        let b: Backup = serde_json::from_slice(&std::fs::read(&bpath).unwrap()).unwrap();
        let rewritten = std::fs::metadata(&f).unwrap().ino();
        restore(&b).unwrap();
        assert_eq!(std::fs::metadata(&f).unwrap().ino(), rewritten);
        assert_eq!(flac_audio(&f), audio);
        let (_, back) = crate::probe_and_read(&f).unwrap();
        assert!(back.recording_mbid.is_none());
    }

    /// A FLAC file with an ID3 tag in front (some taggers write one) keeps it, and its audio.
    #[test]
    fn a_flac_file_with_an_id3_tag_in_front_keeps_it() {
        let dir = tempfile::tempdir().unwrap();
        let plain = flac_without_padding(dir.path());
        let mut front = Vec::new();
        let mut id3 = id3::Tag::new();
        id3.set_title("In front");
        id3.write_to(&mut front, id3::Version::Id3v24).unwrap();
        let f = dir.path().join("b.flac");
        let mut bytes = front.clone();
        bytes.extend(std::fs::read(&plain).unwrap());
        std::fs::write(&f, bytes).unwrap();
        let audio = flac_audio(&f);
        assert_eq!(audio, flac_audio(&plain));
        let before = std::fs::metadata(&f).unwrap().len();
        write_ids(&f, &ids(), &dir.path().join("backups")).unwrap();
        let now = std::fs::read(&f).unwrap();
        assert!(now.len() as u64 > before, "no room: the file was written anew");
        assert_eq!(&now[..front.len()], &front[..]);
        assert_eq!(flac_audio(&f), audio);
        let tag = metaflac::Tag::read_from_path(&f).unwrap();
        assert_eq!(
            tag.get_vorbis("MUSICBRAINZ_TRACKID").unwrap().collect::<Vec<_>>(),
            [MB]
        );
    }

    /// A file is replaced only by a write that finished: one that fails leaves the original as
    /// it was, byte for byte, and nothing next to it.
    #[test]
    fn a_failed_rewrite_leaves_the_file_as_it_was() {
        use std::io::Write;
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("a.flac");
        std::fs::write(&f, b"the original, all of it").unwrap();
        // One left by a write that never finished is no obstacle
        std::fs::write(rewrite_path(&f), b"left over").unwrap();
        let r = replace_file(&f, |new| {
            new.write_all(b"half of the new").unwrap();
            Err(failed(&f, "the share went away"))
        });
        assert!(matches!(r, Err(WriteError::Failed { .. })));
        assert_eq!(std::fs::read(&f).unwrap(), b"the original, all of it");
        assert!(!rewrite_path(&f).exists());
        replace_file(&f, |new| {
            new.write_all(b"the new one").unwrap();
            Ok(())
        })
        .unwrap();
        assert_eq!(std::fs::read(&f).unwrap(), b"the new one");
        assert!(!rewrite_path(&f).exists());
    }

    /// A song the library reaches through a link (a folder of links to another disk) is written
    /// where it really is: the link stays a link, and nothing is left next to either.
    #[cfg(unix)]
    #[test]
    fn a_linked_file_is_replaced_where_it_is_and_the_link_stays() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("disk")).unwrap();
        std::fs::create_dir(dir.path().join("library")).unwrap();
        let real = dir.path().join("disk/a.mp3");
        let mut audio: Vec<u8> = (0..50_000u32).map(|i| (i % 251) as u8).collect();
        audio[..2].copy_from_slice(&[0xFF, 0xFB]);
        std::fs::write(&real, &audio).unwrap();
        let link = dir.path().join("library/linked.mp3");
        std::os::unix::fs::symlink(&real, &link).unwrap();
        write_ids(&link, &ids(), &dir.path().join("backups")).unwrap();
        assert!(std::fs::symlink_metadata(&link).unwrap().file_type().is_symlink());
        let tag = id3::Tag::read_from_path(&real).unwrap();
        assert!(tag
            .extended_texts()
            .any(|t| t.description == "MusicBrainz Track Id" && t.value == MB));
        assert!(std::fs::read(&real).unwrap().ends_with(&audio));
        assert!(!rewrite_path(&link).exists() && !rewrite_path(&real).exists());
    }

    /// A tag whose length field reaches the end of the file (or lies past it) would make the
    /// writer take the audio for part of the tag. Such a file is refused and stays as it is.
    #[test]
    fn an_mp3_whose_tag_claims_the_whole_file_is_left_alone() {
        let dir = tempfile::tempdir().unwrap();
        let mut front = Vec::new();
        let mut tag = id3::Tag::new();
        tag.set_title("T");
        tag.write_to(&mut front, id3::Version::Id3v24).unwrap();
        let mut audio: Vec<u8> = (0..20_000u32).map(|i| (i % 251) as u8).collect();
        audio[..2].copy_from_slice(&[0xFF, 0xFB]);
        let whole = audio.len() as u32 + front.len() as u32 + 1 - 10;
        // (to the end of the file, past it, short of it by one byte, and half-way into the audio)
        for claimed in [whole, 0x0FFF_FFFF, whole - 1, whole - 10_000] {
            // The frames, a zero byte (where a reader stops), then the audio; the header says
            // the tag is `claimed` bytes long
            let mut bytes = [&front[..], &[0u8][..], &audio[..]].concat();
            for (i, b) in bytes[6..10].iter_mut().enumerate() {
                *b = ((claimed >> (7 * (3 - i))) & 0x7f) as u8;
            }
            let f = dir.path().join(format!("claims-{claimed}.mp3"));
            std::fs::write(&f, &bytes).unwrap();
            let r = write_ids(&f, &ids(), &dir.path().join("backups"));
            assert!(matches!(r, Err(WriteError::Failed { .. })), "{claimed}: {r:?}");
            assert_eq!(
                std::fs::read(&f).unwrap(),
                bytes,
                "{claimed}: the file is as it was"
            );
            assert!(!rewrite_path(&f).exists());
        }
    }

    /// A file the node may not write is not replaced by a new one either, also in a folder it
    /// may write.
    #[cfg(unix)]
    #[test]
    fn a_read_only_file_is_not_replaced() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("a.mp3");
        let mut audio: Vec<u8> = (0..20_000u32).map(|i| (i % 251) as u8).collect();
        audio[..2].copy_from_slice(&[0xFF, 0xFB]);
        std::fs::write(&f, &audio).unwrap();
        std::fs::set_permissions(&f, std::fs::Permissions::from_mode(0o444)).unwrap();
        // (root may write anything: nothing to show then)
        if std::fs::OpenOptions::new().write(true).open(&f).is_ok() {
            return;
        }
        assert!(write_ids(&f, &ids(), &dir.path().join("backups")).is_err());
        assert_eq!(std::fs::read(&f).unwrap(), audio);
        assert!(!rewrite_path(&f).exists());
    }

    /// The file a rewrite fills has a short name whatever the song's is.
    #[test]
    fn the_rewrite_file_has_a_short_name() {
        let long = format!("{}.flac", "x".repeat(250));
        let p = rewrite_path(&Path::new("/music/a").join(&long));
        assert_eq!(p.parent(), Some(Path::new("/music/a")));
        let name = p.file_name().unwrap().to_string_lossy().into_owned();
        assert!(name.starts_with(".odeion-write-") && name.len() == 30, "{name}");
        assert_eq!(p, rewrite_path(&Path::new("/music/a").join(&long)));
        assert_ne!(p, rewrite_path(Path::new("/music/a/other.flac")));
    }

    /// Audio that could not be read to its end is an error, never a shorter copy.
    #[test]
    fn a_copy_that_comes_up_short_is_an_error() {
        struct Fails(usize);
        impl std::io::Read for Fails {
            fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
                if self.0 == 0 {
                    return Err(std::io::Error::other("the share went away"));
                }
                let n = self.0.min(buf.len());
                self.0 -= n;
                buf[..n].fill(7);
                Ok(n)
            }
        }
        let mut out = Vec::new();
        assert!(
            copy_exact(&mut Fails(100), &mut out, 300).is_err(),
            "a read error"
        );
        let mut out = Vec::new();
        let mut short: &[u8] = &[7; 100];
        let e = copy_exact(&mut short, &mut out, 300).unwrap_err();
        assert_eq!(
            e.kind(),
            std::io::ErrorKind::UnexpectedEof,
            "the source ended early"
        );
        let mut out = Vec::new();
        let mut whole: &[u8] = &[7; 300];
        copy_exact(&mut whole, &mut out, 300).unwrap();
        assert_eq!(out, [7; 300]);
    }

    /// An MP3's tag is written into a copy that takes the file's place: the audio after the tag
    /// is the same byte for byte, with a tag there before or not, and a second write (another
    /// length of tag) keeps it too.
    #[cfg(unix)]
    #[test]
    fn mp3_tags_are_written_into_a_copy_with_the_same_audio() {
        use std::os::unix::fs::MetadataExt;
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("a.mp3");
        // (not decodable audio: what matters is that these bytes stay; it starts as a frame does)
        let mut audio: Vec<u8> = (0..200_000u32).map(|i| (i % 251) as u8).collect();
        audio[..2].copy_from_slice(&[0xFF, 0xFB]);
        std::fs::write(&f, &audio).unwrap();
        let audio_of = |f: &Path| {
            let mut file = std::fs::File::open(f).unwrap();
            let start = id3_audio_start(&mut file).unwrap() as usize;
            std::fs::read(f).unwrap()[start..].to_vec()
        };
        let before = std::fs::metadata(&f).unwrap().ino();
        let bpath = write_ids(&f, &ids(), &dir.path().join("backups")).unwrap();
        assert_ne!(std::fs::metadata(&f).unwrap().ino(), before);
        assert_eq!(audio_of(&f), audio);
        let tag = id3::Tag::read_from_path(&f).unwrap();
        assert!(tag
            .extended_texts()
            .any(|t| t.description == "MusicBrainz Track Id" && t.value == MB));
        let fixes = Fixes {
            ids: None,
            artists: vec!["Sefa".into(), "Cyclon".repeat(2000)],
            artist_mbids: vec![],
        };
        write_fixes(&f, &fixes, &dir.path().join("backups")).unwrap();
        assert_eq!(audio_of(&f), audio);
        let b: Backup = serde_json::from_slice(&std::fs::read(&bpath).unwrap()).unwrap();
        restore(&b).unwrap();
        assert_eq!(audio_of(&f), audio);
        assert!(!rewrite_path(&f).exists());

        // Zero bytes between a tag and the audio are the tag's padding: a file with them is
        // written like any other (the padding may go, the audio stays)
        let mut front = Vec::new();
        id3::Tag::new()
            .write_to(&mut front, id3::Version::Id3v24)
            .unwrap();
        let padded = dir.path().join("padded.mp3");
        std::fs::write(&padded, [&front[..], &[0u8; 300], &audio[..]].concat()).unwrap();
        write_ids(&padded, &ids(), &dir.path().join("backups")).unwrap();
        assert_eq!(audio_of(&padded), audio);
        // So is a file with no tag that starts with zeros
        let zeros = dir.path().join("zeros.mp3");
        std::fs::write(&zeros, [&[0u8; 70_000][..], &audio[..]].concat()).unwrap();
        write_ids(&zeros, &ids(), &dir.path().join("backups")).unwrap();
        assert_eq!(audio_of(&zeros), audio);
        assert!(id3::Tag::read_from_path(&zeros).is_ok());
    }
}
