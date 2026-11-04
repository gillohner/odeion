//! Several MusicBrainz recordings of one song (owner, 2026-11-04: "most songs could be
//! auto-matched"): one fingerprint often names the same audio under several recordings, and a
//! search by the file's exact title, artist and length often finds the same song on several
//! releases. That is not doubt about the song: the most logical recording is picked from the
//! release context. What is really different (a live take, a remix, another artist, a length far
//! off, another title) is never picked: it stays for the owner.
//!

use crate::matching::{plain_name, plain_title, LENGTH_SLACK_MS};
use pm_model::Mbid;

/// What the file says about itself.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct FileTags {
    pub title: String,
    pub artist: String,
    pub album: Option<String>,
    pub duration_ms: Option<u64>,
}

/// The recording picked, and its release when one is clear.
#[derive(Debug, Clone, PartialEq)]
pub struct SongPick {
    pub recording: Mbid,
    pub release: Option<Mbid>,
    pub release_title: Option<String>,
}

/// Whether `title` is the file's song in the file's version: the same title once plain, and no
/// version words (live, remix, edit…) on one side the other doesn't have ([`crate::album_fit`]).
pub fn same_song(file: &FileTags, title: &str) -> bool {
    !file.title.trim().is_empty()
        && plain_title(&file.title) == plain_title(title)
        && crate::album_fit::versions_agree(&file.title, file.album.as_deref(), title, None)
}

/// Whether a credit is the file's artist: the same name once plain, or that name featuring
/// someone ("Band feat. Singer"). Another artist, or a duo the file's artist is half of, is not.
pub fn same_artist(file: &str, credit: &str) -> bool {
    let (f, c) = (plain_name(file), plain_name(credit));
    if f.is_empty() || c.is_empty() {
        return false;
    }
    f == c
        || ["feat", "ft", "featuring"]
            .iter()
            .any(|j| c.starts_with(&format!("{f} {j} ")))
}

/// Whether the file says it is live (its title or its album).
pub fn file_is_live(file: &FileTags) -> bool {
    crate::album_fit::version_marks(&file.title).contains(&"live")
        || crate::album_fit::marks_in(file.album.as_deref().unwrap_or_default()).contains(&"live")
}

/// How plain a release kind is for a song: 2 a studio album, 1 a single or EP, 0 anything with
/// a secondary type (compilation, live, remix…) or no type.
pub fn plain_kind(primary: Option<&str>, secondary: &[String]) -> u8 {
    if !secondary.is_empty() {
        return 0;
    }
    match primary.map(str::to_lowercase).as_deref() {
        Some("album") => 2,
        Some("single" | "ep") => 1,
        _ => 0,
    }
}

/// The most logical recording among a search's hits, when one is exactly the file's song: its
/// title and version, its artist, its length (within 3 s), a strong hit, and not only on live
/// releases (unless the file is live). Among several such recordings, in order: the one on a
/// release named like the file's album, the one on the plainest release (studio album, then
/// single or EP), the one on most releases, the lowest id. The release is said only when the
/// picked recording has exactly one release named like the album. `None`: nothing is exactly the
/// file's song, the owner decides.
#[cfg(feature = "musicbrainz")]
pub fn pick_exact_hit(file: &FileTags, hits: &[crate::musicbrainz::RecordingHit]) -> Option<SongPick> {
    let album = file.album.as_deref().map(plain_title).filter(|a| !a.is_empty());
    let live_ok = file_is_live(file);
    let has_kind =
        |r: &crate::musicbrainz::HitRelease, k: &str| r.kinds.iter().any(|x| x.eq_ignore_ascii_case(k));
    let exact: Vec<&crate::musicbrainz::RecordingHit> = hits
        .iter()
        .filter(|h| h.score >= 90)
        .filter(|h| same_song(file, &h.title) && same_artist(&file.artist, &h.artist))
        .filter(|h| matches!((h.length_ms, file.duration_ms), (Some(a), Some(b)) if a.abs_diff(b) <= LENGTH_SLACK_MS))
        .filter(|h| live_ok || h.releases.is_empty() || !h.releases.iter().all(|r| has_kind(r, "live")))
        .collect();
    fn named<'h>(
        h: &'h crate::musicbrainz::RecordingHit,
        album: Option<&str>,
    ) -> Vec<&'h crate::musicbrainz::HitRelease> {
        h.releases
            .iter()
            .filter(|r| album.is_some_and(|a| plain_title(&r.title) == a))
            .collect()
    }
    let album_ref = album.as_deref();
    let plain = |h: &crate::musicbrainz::RecordingHit| -> u8 {
        h.releases
            .iter()
            .filter(|r| !r.status.eq_ignore_ascii_case("bootleg"))
            .map(|r| {
                plain_kind(
                    r.kinds.first().map(String::as_str),
                    r.kinds.get(1..).unwrap_or_default(),
                )
            })
            .max()
            .unwrap_or(0)
    };
    let best = exact.into_iter().min_by(|a, b| {
        let key = |h: &crate::musicbrainz::RecordingHit| {
            (
                std::cmp::Reverse(!named(h, album_ref).is_empty()),
                std::cmp::Reverse(plain(h)),
                std::cmp::Reverse(h.releases.len()),
            )
        };
        key(a)
            .cmp(&key(b))
            .then_with(|| a.mbid.to_string().cmp(&b.mbid.to_string()))
    })?;
    let on_album = named(best, album_ref);
    let release = (on_album.len() == 1).then(|| on_album[0]);
    Some(SongPick {
        recording: best.mbid,
        release: release.and_then(|r| r.mbid),
        release_title: release.map(|r| r.title.clone()),
    })
}

#[cfg(all(test, feature = "musicbrainz"))]
mod tests {
    use super::*;
    use crate::musicbrainz::{HitRelease, RecordingHit};

    fn id(i: u32) -> Mbid {
        Mbid::parse(&format!("00000000-0000-4000-8000-{i:012}")).unwrap()
    }

    fn rel(i: u32, title: &str, kinds: &[&str]) -> HitRelease {
        HitRelease {
            title: title.into(),
            mbid: Some(id(500 + i)),
            status: "Official".into(),
            kinds: kinds.iter().map(|k| k.to_string()).collect(),
        }
    }

    fn hit(
        i: u32,
        title: &str,
        artist: &str,
        length: Option<u64>,
        releases: Vec<HitRelease>,
    ) -> RecordingHit {
        RecordingHit {
            mbid: id(i),
            score: 100,
            title: title.into(),
            artist: artist.into(),
            release: releases.first().map(|r| r.title.clone()),
            release_mbid: releases.first().and_then(|r| r.mbid),
            length_ms: length,
            releases,
        }
    }

    fn file() -> FileTags {
        FileTags {
            title: "Over My Shoulder".into(),
            artist: "Mike + The Mechanics".into(),
            album: Some("Beggar on a Beach of Gold".into()),
            duration_ms: Some(232_000),
        }
    }

    /// Exactly the file's title, artist and length on several recordings: the one on the
    /// release named like the file's album; else a plain studio album's; else the one on most
    /// releases; else the lowest id. Always the same answer for the same hits.
    #[test]
    fn the_most_logical_of_several_exact_recordings_is_picked() {
        let a = "Mike + The Mechanics";
        let hits = vec![
            hit(
                3,
                "Over My Shoulder",
                a,
                Some(232_500),
                vec![rel(1, "Hits", &["Album", "Compilation"])],
            ),
            hit(
                2,
                "Over My Shoulder",
                a,
                Some(231_200),
                vec![rel(2, "Beggar on a Beach of Gold", &["Album"])],
            ),
            hit(
                1,
                "Over My Shoulder",
                a,
                Some(233_000),
                vec![rel(3, "Over My Shoulder", &["Single"])],
            ),
        ];
        let p = pick_exact_hit(&file(), &hits).expect("picked");
        assert_eq!(p.recording, id(2), "on the file's album");
        assert_eq!(p.release, Some(id(502)));
        // In another order: the same
        let mut rev = hits.clone();
        rev.reverse();
        assert_eq!(pick_exact_hit(&file(), &rev).unwrap().recording, id(2));
        // No release named like the album: a plain studio album's, the release left open
        let mut f = file();
        f.album = Some("Something Else".into());
        let p = pick_exact_hit(&f, &hits).unwrap();
        assert_eq!((p.recording, p.release), (id(2), None));
        // Only compilations: the one on most releases, then the lowest id
        let hits = vec![
            hit(
                7,
                "Over My Shoulder",
                a,
                Some(232_000),
                vec![rel(1, "Hits", &["Album", "Compilation"])],
            ),
            hit(
                5,
                "Over My Shoulder",
                a,
                Some(232_000),
                vec![
                    rel(2, "More Hits", &["Album", "Compilation"]),
                    rel(4, "Best", &["Album", "Compilation"]),
                ],
            ),
            hit(
                6,
                "Over My Shoulder",
                a,
                Some(232_000),
                vec![rel(3, "Even More", &["Album", "Compilation"])],
            ),
        ];
        assert_eq!(pick_exact_hit(&f, &hits).unwrap().recording, id(5));
        let hits = vec![
            hit(9, "Over My Shoulder", a, Some(232_000), vec![]),
            hit(8, "Over My Shoulder", a, Some(232_000), vec![]),
        ];
        assert_eq!(pick_exact_hit(&f, &hits).unwrap().recording, id(8));
        // One exact recording: it
        let one = vec![hit(4, "Over My Shoulder", a, Some(232_000), vec![])];
        assert_eq!(pick_exact_hit(&f, &one).unwrap().recording, id(4));
        // A featured artist in the credit is still the artist
        let feat = vec![hit(
            4,
            "Over My Shoulder",
            "Mike + The Mechanics feat. Paul Carrack",
            Some(232_000),
            vec![],
        )];
        assert!(pick_exact_hit(&f, &feat).is_some());
    }

    /// What is really different is never picked: a live take (only on live releases), a remix,
    /// another artist, a length far off, no length, another title, a weak hit.
    #[test]
    fn real_differences_are_left_for_review() {
        let a = "Mike + The Mechanics";
        let f = file();
        let none = |h: RecordingHit| pick_exact_hit(&f, &[h]).is_none();
        assert!(
            none(hit(
                1,
                "Over My Shoulder",
                a,
                Some(232_000),
                vec![rel(1, "Live at Wembley", &["Album", "Live"])]
            )),
            "live"
        );
        assert!(
            none(hit(1, "Over My Shoulder (Alice Remix)", a, Some(232_000), vec![])),
            "remix"
        );
        assert!(
            none(hit(1, "Over My Shoulder", "Someone Else", Some(232_000), vec![])),
            "another artist"
        );
        assert!(
            none(hit(1, "Over My Shoulder", a, Some(252_000), vec![])),
            "20 s off"
        );
        assert!(
            none(hit(1, "Over My Shoulder", a, None, vec![])),
            "no length to compare"
        );
        assert!(
            none(hit(1, "Over My Shoulder Again", a, Some(232_000), vec![])),
            "another title"
        );
        let mut weak = hit(1, "Over My Shoulder", a, Some(232_000), vec![]);
        weak.score = 80;
        assert!(none(weak), "a weak hit");
        // The file's own album is live: a live recording is its version
        let mut live = file();
        live.album = Some("Live in Paris".into());
        assert!(pick_exact_hit(
            &live,
            &[hit(
                1,
                "Over My Shoulder",
                a,
                Some(232_000),
                vec![rel(1, "Live in Paris", &["Album", "Live"])]
            )]
        )
        .is_some());
        // An exact one among different ones: the exact one
        let mixed = vec![
            hit(1, "Over My Shoulder (Alice Remix)", a, Some(232_000), vec![]),
            hit(2, "Over My Shoulder", a, Some(232_000), vec![]),
        ];
        assert_eq!(pick_exact_hit(&f, &mixed).unwrap().recording, id(2));
    }
}
