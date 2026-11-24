use super::*;
use std::io::Cursor;

fn chunk(id: &[u8; 4], body: &[u8]) -> Vec<u8> {
    let mut v = id.to_vec();
    v.extend_from_slice(&(body.len() as u64).to_be_bytes());
    v.extend_from_slice(body);
    if body.len() % 2 == 1 {
        v.push(0);
    }
    v
}

/// An ID3v2.4 tag with `n` bytes of body.
fn tag(n: u32) -> Vec<u8> {
    let mut v = b"ID3\x04\x00\x00".to_vec();
    v.extend((0..4).rev().map(|i| ((n >> (7 * i)) & 0x7f) as u8));
    v.extend(vec![b'x'; n as usize]);
    v
}

/// A DFF form from top-level chunks after FVER; PROP holds FS, CHNL (2) and CMPR `cmpr`.
fn dff(cmpr: &[u8; 4], rest: &[Vec<u8>]) -> Vec<u8> {
    let mut snd = b"SND ".to_vec();
    snd.extend(chunk(b"FS  ", &2_822_400u32.to_be_bytes()));
    snd.extend(chunk(b"CHNL", b"\x00\x02SLFTSRGT"));
    snd.extend(chunk(b"CMPR", &[&cmpr[..], b"\x00"].concat()));
    let mut body = b"DSD ".to_vec();
    body.extend(chunk(b"FVER", &0x0105_0000u32.to_be_bytes()));
    body.extend(chunk(b"PROP", &snd));
    for c in rest {
        body.extend_from_slice(c);
    }
    let mut v = b"FRM8".to_vec();
    v.extend_from_slice(&(body.len() as u64).to_be_bytes());
    v.extend(body);
    v
}

/// A DSF file with a fmt chunk of `fmt_size` bytes, 2 channels, 1-bit samples, `data` and a tag.
fn dsf(fmt_size: u64, data: &[u8], tag: Option<&[u8]>) -> Vec<u8> {
    let data_at = 28 + fmt_size;
    let tag_at = if tag.is_some() {
        data_at + 12 + data.len() as u64
    } else {
        0
    };
    let mut v = b"DSD ".to_vec();
    v.extend_from_slice(&28u64.to_le_bytes());
    v.extend_from_slice(&0u64.to_le_bytes());
    v.extend_from_slice(&tag_at.to_le_bytes());
    v.extend_from_slice(b"fmt ");
    v.extend_from_slice(&fmt_size.to_le_bytes());
    for x in [1u32, 0, 2, 2, 2_822_400, 1] {
        v.extend_from_slice(&x.to_le_bytes());
    }
    v.extend_from_slice(&(data.len() as u64 * 4).to_le_bytes());
    v.extend_from_slice(&(data.len() as u32 / 2).to_le_bytes());
    v.extend_from_slice(&0u32.to_le_bytes());
    v.resize(data_at as usize, 0);
    v.extend_from_slice(b"data");
    v.extend_from_slice(&(12 + data.len() as u64).to_le_bytes());
    v.extend_from_slice(data);
    if let Some(t) = tag {
        v.extend_from_slice(t);
    }
    v
}

#[test]
fn a_dsf_header_says_where_its_samples_are() {
    let data = [0x55u8; 64];
    for fmt_size in [52u64, 60] {
        let h = read(&mut Cursor::new(dsf(fmt_size, &data, Some(&tag(5)))), true).unwrap();
        assert_eq!(h.container, Container::Dsf);
        assert_eq!(h.format, AudioFormat::dsd(2_822_400, 2));
        // (after a fmt chunk of its own size, not one of 52 bytes taken for granted)
        assert_eq!(h.data_offset, 28 + fmt_size + 12, "{fmt_size}");
        assert_eq!(h.data_len, 64);
        assert_eq!(h.sample_count, 64 * 4);
        assert_eq!(h.block_size, 32);
        assert!(h.lsb_first);
        assert!(!h.compressed);
        assert_eq!(h.id3.as_deref(), Some(&tag(5)[..]));
    }
}

#[test]
fn the_tag_is_read_only_when_asked_for() {
    let file = dsf(52, &[0u8; 64], Some(&tag(5)));
    assert!(read(&mut Cursor::new(&file), false).unwrap().id3.is_none());
    let file = dff(b"DSD ", &[chunk(b"DSD ", &[0u8; 100]), chunk(b"ID3 ", &tag(5))]);
    assert!(read(&mut Cursor::new(&file), false).unwrap().id3.is_none());
    assert!(read(&mut Cursor::new(&file), true).unwrap().id3.is_some());
}

#[test]
fn a_tag_cut_short_or_too_long_is_none_and_the_file_still_reads() {
    let mut cut = dsf(52, &[0u8; 64], Some(&tag(50)));
    cut.truncate(cut.len() - 10);
    let h = read(&mut Cursor::new(cut), true).unwrap();
    assert!(h.id3.is_none());
    assert_eq!(h.data_len, 64);
    let mut long = tag(0);
    long[6..10].copy_from_slice(&[0x7f; 4]);
    assert!(read(&mut Cursor::new(dsf(52, &[0u8; 64], Some(&long))), true)
        .unwrap()
        .id3
        .is_none());
}

#[test]
fn a_dff_header_says_whether_its_audio_is_compressed() {
    let plain = dff(b"DSD ", &[chunk(b"DSD ", &[0u8; 100])]);
    let h = read(&mut Cursor::new(plain), true).unwrap();
    assert_eq!(
        (h.container, h.compressed, h.data_len, h.sample_count),
        (Container::Dff, false, 100, 400)
    );
    let cmpr = dff(b"DST ", &[chunk(b"DSD ", &[0u8; 100])]);
    assert!(read(&mut Cursor::new(cmpr), true).unwrap().compressed);
    let dst = dff(b"DSD ", &[chunk(b"DSD ", &[0u8; 100]), chunk(b"DST ", &[0u8; 8])]);
    assert!(read(&mut Cursor::new(dst), true).unwrap().compressed);
}

#[test]
fn a_top_level_dff_tag_comes_before_one_in_diin() {
    let diin = chunk(b"DIIN", &chunk(b"ID3 ", &tag(3)));
    let both = dff(
        b"DSD ",
        &[diin.clone(), chunk(b"DSD ", &[0u8; 10]), chunk(b"ID3 ", &tag(7))],
    );
    assert_eq!(read(&mut Cursor::new(both), true).unwrap().id3, Some(tag(7)));
    let only_diin = dff(b"DSD ", &[diin, chunk(b"DSD ", &[0u8; 10])]);
    assert_eq!(read(&mut Cursor::new(only_diin), true).unwrap().id3, Some(tag(3)));
}

#[test]
fn other_files_are_not_read() {
    assert!(matches!(
        read(&mut Cursor::new(b"RIFF....WAVE".to_vec()), true),
        Err(Error::Unsupported(_))
    ));
    let mut aiff = dff(b"DSD ", &[]);
    aiff[12..16].copy_from_slice(b"AIFF");
    assert!(matches!(
        read(&mut Cursor::new(aiff), true),
        Err(Error::Unsupported(_))
    ));
    // (a DSF format id other than 0: compressed)
    let mut compressed = dsf(52, &[0u8; 64], None);
    compressed[28 + 16..28 + 20].copy_from_slice(&1u32.to_le_bytes());
    assert!(matches!(
        read(&mut Cursor::new(compressed), true),
        Err(Error::Unsupported(_))
    ));
    // (no sample rate: nothing to play)
    let no_fs = b"FRM8\x00\x00\x00\x00\x00\x00\x00\x04DSD ".to_vec();
    assert!(matches!(
        read(&mut Cursor::new(no_fs), true),
        Err(Error::Malformed(_))
    ));
}

#[test]
fn a_dff_tag_is_read_by_its_own_length_within_its_chunk() {
    // A tag shorter than its chunk (padding after it): the tag alone
    let padded = [tag(5), vec![0u8; 9]].concat();
    let file = dff(b"DSD ", &[chunk(b"DSD ", &[0u8; 10]), chunk(b"ID3 ", &padded)]);
    assert_eq!(read(&mut Cursor::new(file), true).unwrap().id3, Some(tag(5)));
    // A tag that says it is longer than its chunk: no tag, even though the file goes on
    let mut long = tag(5);
    long[9] = 100;
    let file = dff(b"DSD ", &[chunk(b"ID3 ", &long), chunk(b"DSD ", &[0u8; 200])]);
    let h = read(&mut Cursor::new(file), true).unwrap();
    assert!(h.id3.is_none());
    assert_eq!(h.data_len, 200);
}

#[test]
fn a_tag_that_does_not_read_is_not_looked_for_again() {
    // (many such chunks would each be read in vain)
    let file = dff(
        b"DSD ",
        &[
            chunk(b"ID3 ", b"not a tag!"),
            chunk(b"ID3 ", &tag(5)),
            chunk(b"DSD ", &[0u8; 10]),
        ],
    );
    assert!(read(&mut Cursor::new(file), true).unwrap().id3.is_none());
}

#[test]
fn a_dst_file_is_unsupported_not_malformed() {
    // A real DST file: CMPR "DST " and the audio in a DST chunk, no DSD chunk
    let file = dff(b"DST ", &[chunk(b"DST ", &[0u8; 64])]);
    assert!(matches!(
        read(&mut Cursor::new(file), true),
        Err(Error::Unsupported(_))
    ));
    // A CMPR cut short at the end of the file: not plain DSD
    let mut cut = dff(b"DSD ", &[]);
    let at = cut.windows(4).position(|w| w == b"CMPR").unwrap() + 12;
    cut.truncate(at + 2);
    let n = cut.len() as u64 - 12;
    cut[4..12].copy_from_slice(&n.to_be_bytes());
    assert!(matches!(
        read(&mut Cursor::new(cut), true),
        Err(Error::Unsupported(_))
    ));
}

#[test]
fn a_prop_that_is_not_snd_is_skipped() {
    // Not SND, with a sample rate and channels of its own that must not be taken
    let mut odd = b"XYZ ".to_vec();
    odd.extend(chunk(b"FS  ", &5_644_800u32.to_be_bytes()));
    odd.extend(chunk(b"CHNL", b"\x00\x01C   "));
    // (after the real PROP, so a reader taking it would end up with its values)
    let file = dff(b"DSD ", &[chunk(b"PROP", &odd), chunk(b"DSD ", &[0u8; 10])]);
    let h = read(&mut Cursor::new(file), true).unwrap();
    assert_eq!(h.format, AudioFormat::dsd(2_822_400, 2));
    // Only one that is not SND: no sample rate, nothing to play
    let body = [b"DSD ".to_vec(), chunk(b"PROP", &odd), chunk(b"DSD ", &[0u8; 10])].concat();
    let mut file = b"FRM8".to_vec();
    file.extend_from_slice(&(body.len() as u64).to_be_bytes());
    file.extend(body);
    assert!(matches!(
        read(&mut Cursor::new(file), true),
        Err(Error::Malformed(_))
    ));
}

#[test]
fn in_diin_too_a_tag_fits_its_chunk_and_is_tried_once() {
    let mut long = tag(5);
    long[9] = 100;
    let diin = |subs: &[Vec<u8>]| chunk(b"DIIN", &subs.concat());
    let tag_of = |chunks: &[Vec<u8>]| {
        let file = dff(b"DSD ", &[chunks, &[chunk(b"DSD ", &[0u8; 10])][..]].concat());
        read(&mut Cursor::new(file), true).unwrap().id3
    };
    assert_eq!(tag_of(&[diin(&[chunk(b"ID3 ", &tag(5))])]), Some(tag(5)));
    assert_eq!(tag_of(&[diin(&[chunk(b"ID3 ", &long)])]), None);
    // The first ID3 in the first DIIN that has one is the only one tried
    let first_bad = diin(&[chunk(b"ID3 ", b"not a tag!"), chunk(b"ID3 ", &tag(5))]);
    assert_eq!(tag_of(&[first_bad, diin(&[chunk(b"ID3 ", &tag(3))])]), None);
    // A DIIN without one does not count
    assert_eq!(
        tag_of(&[diin(&[chunk(b"DIAR", b"x")]), diin(&[chunk(b"ID3 ", &tag(3))])]),
        Some(tag(3))
    );
}

/// A DSF whose sample count or data chunk claims more than it holds reads as what is there:
/// the data ends before the tag after it and at the file's end, and the samples in the last
/// whole set of channel blocks.
#[test]
fn a_dsf_that_claims_more_than_it_holds_reads_as_what_is_there() {
    let data = [0x55u8; 64];
    let whole = dsf(52, &data, Some(&tag(5)));
    // (the sample count at 28 + 36, the block size at 28 + 44, the data chunk's size at 28 + 52 + 4)
    let put = |f: &mut Vec<u8>, at: usize, n: u64| f[at..at + 8].copy_from_slice(&n.to_le_bytes());
    for (n, len) in [
        (u64::MAX, None),
        (64 * 4 + 1, None),
        (u64::MAX, Some(u64::MAX)),
        (64 * 4, Some(1 << 40)),
    ] {
        let mut f = whole.clone();
        put(&mut f, 64, n);
        if let Some(len) = len {
            put(&mut f, 84, len);
        }
        let h = read(&mut Cursor::new(f), true).unwrap();
        assert_eq!(h.data_len, 64, "{n} {len:?}");
        assert_eq!(h.sample_count, n.min(64 * 4), "{n} {len:?}");
        assert_eq!(h.id3.as_deref(), Some(&tag(5)[..]), "{n} {len:?}");
    }
    // No tag: the data ends at the file's end
    let mut f = dsf(52, &data, None);
    put(&mut f, 84, 1 << 40);
    let h = read(&mut Cursor::new(f), false).unwrap();
    assert_eq!((h.data_len, h.sample_count), (64, 64 * 4));
    // Data that is not whole sets of blocks (84 bytes, blocks of 32 for 2 channels, no tag):
    // the samples end with the last whole set
    let mut f = dsf(52, &[0x55u8; 84], None);
    f[72..76].copy_from_slice(&32u32.to_le_bytes());
    put(&mut f, 64, u64::MAX);
    let h = read(&mut Cursor::new(f), false).unwrap();
    assert_eq!((h.data_len, h.sample_count), (84, 32 * 8));
    // A file as written: as it says
    let h = read(&mut Cursor::new(whole), false).unwrap();
    assert_eq!((h.data_len, h.sample_count), (64, 64 * 4));
}
