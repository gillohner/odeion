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
    same_song_as(file, title, None, None)
}

/// [`same_song`] with what MusicBrainz says tells the candidate apart (its disambiguation):
/// the same title once plain, and the same version ([`crate::album_fit::same_version`]:
/// qualifiers and disambiguation that aren't neutral must be on both sides).
pub fn same_song_as(
    file: &FileTags,
    title: &str,
    disambiguation: Option<&str>,
    credit: Option<&str>,
) -> bool {
    !file.title.trim().is_empty()
        && plain_title(&file.title) == plain_title(title)
        && crate::album_fit::same_version(
            &file.title,
            file.album.as_deref(),
            title,
            disambiguation,
            &[],
            &[file.artist.as_str(), credit.unwrap_or_default()],
        )
}

/// Whether a release is named like the file's album (qualifiers included).
pub fn named_like_album(file: &FileTags, release_title: &str) -> bool {
    file.album
        .as_deref()
        .is_some_and(|a| crate::album_fit::same_name(release_title, a))
}

/// Whether the file's album says it is another version than the plain one ("(Re-Recorded)",
/// "(Taylor's Version)", "(Live)"): then only a release named so is evidence.
pub fn album_says_version(file: &FileTags) -> bool {
    let album = file.album.as_deref().unwrap_or_default();
    // In a qualifier, or a version word anywhere in the name ("Live at Roskilde 1995"): such a
    // name only stops a pick (a release named so is still taken), it never allows one
    !crate::album_fit::qualifiers(album).is_empty() || !crate::album_fit::marks_in(album).is_empty()
}

/// Whether a credit is the file's artist: the same name once plain, or that name featuring
/// someone ("Band feat. Singer"). Another artist, or a duo the file's artist is half of, is not.
pub fn same_artist(file: &str, credit: &str) -> bool {
    let (f, c) = (plain_name(file), plain_name(credit));
    if f.is_empty() || c.is_empty() {
        return false;
    }
    f == c
        || ["feat", "ft", "featuring", "with"]
            .iter()
            .any(|j| c.starts_with(&format!("{f} {j} ")))
}

/// Whether the file says it is live (its title or its album).
pub fn file_is_live(file: &FileTags) -> bool {
    // Only what the file says in a qualifier: a word of a name ("Live Forever") is no evidence
    crate::album_fit::version_marks(&file.title).contains(&"live")
        || crate::album_fit::version_marks(file.album.as_deref().unwrap_or_default()).contains(&"live")
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
    use crate::musicbrainz::{HitRelease, RecordingHit};
    let live_file = file_is_live(file);
    let has_kind = |r: &HitRelease, k: &str| r.kinds.iter().any(|x| x.eq_ignore_ascii_case(k));
    // A recording only on live releases is a live take: the file's, only when the file says it is
    // live, or a live release is named like the file's album
    let live_ok = |h: &RecordingHit| {
        h.releases.is_empty()
            || !h.releases.iter().all(|r| has_kind(r, "live"))
            || live_file
            || h.releases
                .iter()
                .any(|r| has_kind(r, "live") && named_like_album(file, &r.title))
    };
    let exact: Vec<&RecordingHit> = hits
        .iter()
        .filter(|h| h.score >= 90)
        .filter(|h| {
            same_song_as(file, &h.title, h.disambiguation.as_deref(), Some(&h.artist))
                && same_artist(&file.artist, &h.artist)
        })
        .filter(|h| matches!((h.length_ms, file.duration_ms), (Some(a), Some(b)) if a.abs_diff(b) <= LENGTH_SLACK_MS))
        .filter(|h| live_ok(h))
        .collect();
    fn named<'h>(h: &'h RecordingHit, file: &FileTags) -> Vec<&'h HitRelease> {
        h.releases
            .iter()
            .filter(|r| named_like_album(file, &r.title))
            .collect()
    }
    let any_named = exact.iter().any(|h| !named(h, file).is_empty());
    // An album that says it is another version: only a recording on that very release
    if album_says_version(file) && !any_named {
        return None;
    }
    // Several exact hits, none on the file's album: only when they say the same of themselves
    // (title as written, disambiguation); else they may be different versions
    if !any_named && exact.len() > 1 {
        let what = |h: &RecordingHit| {
            (
                h.title.to_lowercase(),
                h.disambiguation.clone().unwrap_or_default().to_lowercase(),
            )
        };
        if exact.iter().any(|h| what(h) != what(exact[0])) {
            return None;
        }
    }
    let plain = |h: &RecordingHit| -> u8 {
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
        let key = |h: &RecordingHit| {
            (
                std::cmp::Reverse(!named(h, file).is_empty()),
                std::cmp::Reverse(plain(h)),
                std::cmp::Reverse(h.releases.len()),
            )
        };
        key(a)
            .cmp(&key(b))
            .then_with(|| a.mbid.to_string().cmp(&b.mbid.to_string()))
    })?;
    let on_album = named(best, file);
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
            disambiguation: None,
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

    fn hit_d(i: u32, title: &str, disamb: &str, releases: Vec<HitRelease>) -> RecordingHit {
        let mut h = hit(i, title, "Band", Some(232_000), releases);
        h.disambiguation = (!disamb.is_empty()).then(|| disamb.to_string());
        h
    }

    fn tags(title: &str, album: &str) -> FileTags {
        FileTags {
            title: title.into(),
            artist: "Band".into(),
            album: Some(album.into()),
            duration_ms: Some(232_000),
        }
    }

    /// PR #250 review (high): another version of the song is never picked on its own. A
    /// recording MusicBrainz marks by its disambiguation, or by a qualifier the file doesn't have
    /// (re-recorded, "Taylor's Version", an alternate take, a mono mix, anything unknown), is not
    /// the file's; only remaster, year, featured artist, album/single version and the like are
    /// neutral. A file whose album says it is another version needs a hit on that very release.
    #[test]
    fn other_versions_are_never_picked_on_their_own() {
        let orig = rel(1, "Greatest Hits", &["Album", "Compilation"]);
        let rerec = rel(2, "Greatest Hits (Re-Recorded)", &["Album", "Compilation"]);
        // The original file: the re-recording (marked only by its disambiguation) is not it
        let hits = vec![
            hit_d(1, "Song", "", vec![orig.clone()]),
            hit_d(2, "Song", "re-recording", vec![rerec.clone()]),
        ];
        assert_eq!(
            pick_exact_hit(&tags("Song", "Greatest Hits"), &hits)
                .unwrap()
                .recording,
            id(1)
        );
        assert!(pick_exact_hit(
            &tags("Song", "Greatest Hits"),
            &[hit_d(2, "Song", "re-recording", vec![])]
        )
        .is_none());
        // The re-recorded album's file: the original's release isn't named like its album
        assert!(pick_exact_hit(
            &tags("Song", "Greatest Hits (Re-Recorded)"),
            &[hit_d(1, "Song", "", vec![orig])]
        )
        .is_none());
        // Taylor's Version: only the hit that says so, on that release
        let tv = rel(3, "1989 (Taylor's Version)", &["Album"]);
        let og = rel(4, "1989", &["Album"]);
        let hits = vec![
            hit_d(3, "Style (Taylor's Version)", "", vec![tv.clone()]),
            hit_d(4, "Style", "", vec![og.clone()]),
        ];
        assert!(pick_exact_hit(
            &tags("Style", "1989 (Taylor's Version)"),
            &[hit_d(4, "Style", "", vec![og.clone()])]
        )
        .is_none());
        assert_eq!(
            pick_exact_hit(
                &tags("Style (Taylor's Version)", "1989 (Taylor's Version)"),
                &hits
            )
            .unwrap()
            .recording,
            id(3)
        );
        assert_eq!(
            pick_exact_hit(&tags("Style", "1989"), &hits).unwrap().recording,
            id(4)
        );
        // Unknown or one-sided qualifiers
        for other in [
            "Song (Alternate Take)",
            "Song (Rehearsal)",
            "Song (Mono)",
            "Song (Explicit)",
            "Song - Take 2",
        ] {
            assert!(
                pick_exact_hit(&tags("Song", "Album"), &[hit_d(5, other, "", vec![])]).is_none(),
                "{other}"
            );
        }
        assert!(
            pick_exact_hit(
                &tags("Song (Mono)", "Album"),
                &[hit_d(5, "Song (Mono)", "", vec![])]
            )
            .is_some(),
            "both mono"
        );
        assert!(pick_exact_hit(
            &tags("Song", "Album"),
            &[hit_d(5, "Song", "alternate take", vec![])]
        )
        .is_none());
        // Neutral qualifiers and disambiguations
        for same in [
            "Song (2011 Remaster)",
            "Song - Remastered 2009",
            "Song (feat. Someone)",
            "Song (Album Version)",
            "Song (Single Version)",
            "Song (Original Mix)",
        ] {
            assert!(
                pick_exact_hit(&tags("Song", "Album"), &[hit_d(5, same, "", vec![])]).is_some(),
                "{same}"
            );
        }
        assert!(pick_exact_hit(
            &tags("Song", "Album"),
            &[hit_d(5, "Song", "2011 remaster", vec![])]
        )
        .is_some());
        // Several exact hits, none on the file's album, differing in what they say: for the owner
        let hits = vec![
            hit_d(6, "Song", "", vec![rel(6, "Hits", &["Album"])]),
            hit_d(7, "Song (2011 Remaster)", "", vec![rel(7, "More", &["Album"])]),
        ];
        assert!(pick_exact_hit(&tags("Song", "Elsewhere"), &hits).is_none());
        // ... the same, with one on the file's album: that one
        let hits = vec![
            hit_d(6, "Song", "", vec![rel(6, "Elsewhere", &["Album"])]),
            hit_d(7, "Song (2011 Remaster)", "", vec![rel(7, "More", &["Album"])]),
        ];
        assert_eq!(
            pick_exact_hit(&tags("Song", "Elsewhere"), &hits)
                .unwrap()
                .recording,
            id(6)
        );
    }

    /// PR #250 review (high): a word in the album's name is no evidence of a live file ("Live
    /// Forever"); a live-only recording needs the file's own qualifier, or a live release named
    /// like the file's album.
    #[test]
    fn live_needs_real_evidence() {
        let live_elsewhere = hit_d(1, "Song", "", vec![rel(1, "Live at Wembley", &["Album", "Live"])]);
        assert!(pick_exact_hit(
            &tags("Song", "Live Forever"),
            std::slice::from_ref(&live_elsewhere)
        )
        .is_none());
        assert!(pick_exact_hit(
            &tags("Song (Live)", "Album"),
            &[hit_d(
                1,
                "Song (Live)",
                "",
                vec![rel(1, "Live at Wembley", &["Album", "Live"])]
            )]
        )
        .is_some());
        let named = hit_d(2, "Song", "", vec![rel(2, "Live in Paris", &["Album", "Live"])]);
        assert!(
            pick_exact_hit(&tags("Song", "Live in Paris"), &[named]).is_some(),
            "the live release named like the album"
        );
        assert!(!file_is_live(&tags("Song", "Live Forever")));
        assert!(file_is_live(&tags("Song", "Hits (Live)")));
    }

    /// MusicBrainz's disambiguation of a recording is read from a search answer.
    #[test]
    fn a_search_answer_carries_the_disambiguation() {
        let v = serde_json::json!({"recordings": [{"id": "00000000-0000-4000-8000-000000000001", "score": 100,
            "title": "Song", "disambiguation": "re-recording", "length": 200000, "artist-credit": [{"name": "Band"}]}]});
        let hits = crate::musicbrainz::parse_search_hits(&v);
        assert_eq!(hits[0].disambiguation.as_deref(), Some("re-recording"));
    }

    /// PR #250 delta review (Kimi, high): "with …" names a featured artist only when that artist
    /// is credited on the file or the hit; "(With Strings)", "(with Orchestra)" are versions.
    #[test]
    fn with_names_a_featured_artist_only_when_credited() {
        let f = FileTags {
            title: "Song".into(),
            artist: "Band".into(),
            album: Some("Album".into()),
            duration_ms: Some(232_000),
        };
        let h = |title: &str, artist: &str| hit(1, title, artist, Some(232_000), vec![]);
        assert!(pick_exact_hit(&f, &[h("Song (With Strings)", "Band")]).is_none());
        assert!(pick_exact_hit(&f, &[h("Song (with Orchestra)", "Band")]).is_none());
        assert!(
            pick_exact_hit(&f, &[h("Song (with Carol)", "Band")]).is_none(),
            "Carol is credited nowhere"
        );
        assert!(
            pick_exact_hit(&f, &[h("Song (with Carol)", "Band with Carol")]).is_some(),
            "Carol is credited on the hit"
        );
        let mut g = f.clone();
        g.title = "Song (with Carol)".into();
        g.artist = "Band with Carol".into();
        assert!(
            pick_exact_hit(&g, &[h("Song", "Band with Carol")]).is_some(),
            "credited on the file"
        );
        assert!(
            pick_exact_hit(&f, &[h("Song (feat. Carol)", "Band")]).is_some(),
            "feat. is always a featured artist"
        );
    }

    /// PR #250 delta review (medium): version words anywhere in the album's name ("Live at
    /// Roskilde 1995", "Unplugged in New York") stop an automatic pick unless a hit is on a release
    /// named like that album; they never allow one ("Live Forever" allows no live-only take).
    #[test]
    fn version_words_in_the_album_name_only_block() {
        let studio = hit_d(1, "Song", "", vec![rel(1, "Hits", &["Album"])]);
        assert!(pick_exact_hit(
            &tags("Song", "Live at Roskilde 1995"),
            std::slice::from_ref(&studio)
        )
        .is_none());
        assert!(pick_exact_hit(
            &tags("Song", "Unplugged in New York"),
            std::slice::from_ref(&studio)
        )
        .is_none());
        let there = hit_d(
            2,
            "Song",
            "",
            vec![rel(2, "Live at Roskilde 1995", &["Album", "Live"])],
        );
        assert_eq!(
            pick_exact_hit(&tags("Song", "Live at Roskilde 1995"), &[studio.clone(), there])
                .unwrap()
                .recording,
            id(2)
        );
        let elsewhere = hit_d(3, "Song", "", vec![rel(3, "Live at Wembley", &["Album", "Live"])]);
        assert!(pick_exact_hit(&tags("Song", "Live Forever"), &[elsewhere]).is_none());
        assert!(
            pick_exact_hit(&tags("Song", "Live Forever"), std::slice::from_ref(&studio)).is_none(),
            "the name blocks"
        );
        assert!(
            pick_exact_hit(&tags("Song", "Album"), &[studio]).is_some(),
            "a plain name blocks nothing"
        );
    }
}
