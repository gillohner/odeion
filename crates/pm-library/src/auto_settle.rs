//! Settling an unsure song without review (owner, 2026-11-11: "make sure our library is properly
//! indexed"). A run leaves a song waiting for review when its evidence names several recordings,
//! or when two ways of matching disagree. Much of that pile is not a real question: one audio
//! fingerprint names the same song under several MusicBrainz recordings, or the file's own tags
//! agree with one of them. This decides from what a run stored for the song (its open options,
//! the file's tags, the release its folder is on), never from a new request, so the same answer
//! comes from a run, the start's re-check and a replay of stored suggestions.
//!
//! A song is settled when its best option is clearly ahead:
//! - **R1**: every sure fingerprint option (AcoustID score at or above
//!   [`pm_model::Confidence::FINGERPRINT_ACCEPT`]) is the file's song: its title, its version and
//!   its artist. Several recordings are then the same audio under duplicate MusicBrainz entries.
//! - **R1b**: every sure fingerprint option is one song by the file's artist, in the file's
//!   version, but the file's title says otherwise (a typo, "Pt." for "Part", a translation,
//!   "Track 01"). The audio is unanimous and the artist agrees.
//! - **R2**: the sure fingerprint options that are the file's song are one song; the others name
//!   other songs (AcoustID links a fingerprint to other recordings by mistake). Audio and the
//!   file's own tags agree on one, the others only have the audio.
//! - **R3**: the folder's other songs are identified on one release, and exactly one option of
//!   the file's song is on it.
//!
//! What stays for the owner: any option the owner rejected for the song; a sure option of the
//! file's title in another version (live, remix, edit, demo, acoustic, instrumental, mono…: a
//! remaster is the same recording), or on a release whose name says another version; the file's
//! song by another artist; an album that names a version no option's release has; the folder
//! naming another song than the fingerprint; editions of the folder that fit as well naming
//! different recordings (a release's type never chooses the recording); sure options of other
//! songs only; weak evidence (no sure fingerprint option, the folder's release not singling one
//! out).
//!
//! Inside the settled group (the audio's own recordings) the recording is chosen the same way
//! every time, never by the order of an answer: the one on the folder's release, then one on a
//! release named like the file's album, then the lowest MusicBrainz id. The release is kept only when the folder names
//! it; otherwise which edition stays open.

use crate::album_fit::{marks_in, same_version, version_marks};
use crate::matching::{plain_name, plain_title};
use crate::same_song::{album_says_version, named_like_album, same_artist, same_song_as, FileTags};
use pm_model::Confidence;

/// A fingerprint option at or above this is sure (AcoustID's accept threshold, not lowered).
pub const STRONG: f32 = Confidence::FINGERPRINT_ACCEPT;
/// An option settled by its folder's release needs at least this (what may be published).
pub const R3_MIN: f32 = Confidence::PUBLISH_THRESHOLD;

/// One open option of a song, as stored (a suggestion row) or as a run found it.
#[derive(Debug, Clone, PartialEq)]
pub struct Opt {
    pub recording: String,
    pub release: Option<String>,
    pub title: Option<String>,
    pub artist: Option<String>,
    pub release_title: Option<String>,
    pub score: f32,
    /// `fingerprint`, `search` or `album`.
    pub source: String,
    /// The recording's length (seconds), when the run's kept AcoustID answer says.
    pub duration_secs: Option<f64>,
    /// Whether the recording is only on live releases, when the kept AcoustID answer says.
    pub live_only: Option<bool>,
}

impl From<&crate::SuggestionRow> for Opt {
    fn from(r: &crate::SuggestionRow) -> Self {
        Self {
            recording: r.recording_mbid.clone(),
            release: r.release_mbid.clone(),
            title: r.title.clone(),
            artist: r.artist.clone(),
            release_title: r.release_title.clone(),
            score: r.score,
            source: r.source.clone(),
            duration_secs: None,
            live_only: None,
        }
    }
}

impl Opt {
    /// What the run's kept AcoustID answer says of this option's recording: its length, and
    /// whether it is only on live releases.
    pub fn with_audio(mut self, results: &[crate::identify::AcoustIdResult]) -> Self {
        let rec = results
            .iter()
            .flat_map(|r| r.recordings.iter())
            .find(|x| x.id == self.recording);
        if let Some(rec) = rec {
            self.duration_secs = self.duration_secs.or(rec.duration);
            let groups = &rec.releasegroups;
            if !groups.is_empty() {
                self.live_only = Some(
                    groups
                        .iter()
                        .all(|g| g.secondarytypes.iter().any(|t| t.eq_ignore_ascii_case("live"))),
                );
            }
        }
        self
    }
}

/// A song's options with the run's kept AcoustID answer: every recording the answer names surely
/// (a run keeps five options at most, so one it left out may be the conflict) joins them as a
/// fingerprint option, and a fingerprint option takes the answer's own score, length and release
/// kinds. Recordings in `rejected` stay out.
pub fn with_answer(
    mut options: Vec<Opt>,
    results: &[crate::identify::AcoustIdResult],
    rejected: &dyn Fn(&str) -> bool,
) -> Vec<Opt> {
    for r in results.iter().filter(|r| r.score >= STRONG) {
        for rec in &r.recordings {
            if rejected(&rec.id) || pm_model::Mbid::parse(&rec.id).is_err() {
                continue;
            }
            match options.iter_mut().find(|o| o.recording == rec.id) {
                Some(o) if o.source == "fingerprint" => o.score = o.score.max(r.score),
                Some(_) => {}
                None => options.push(Opt {
                    recording: rec.id.clone(),
                    release: None,
                    title: rec.title.clone(),
                    artist: rec.artist(),
                    release_title: None,
                    score: r.score,
                    source: "fingerprint".into(),
                    duration_secs: None,
                    live_only: None,
                }),
            }
        }
    }
    options.into_iter().map(|o| o.with_audio(results)).collect()
}

/// Whether what is known of an option's audio says it is another take than the file: a length
/// more than 3 s off, or only on live releases for a file that doesn't say it is live (the same
/// checks a run makes for one audio under several recordings).
fn other_take(tags: &FileTags, o: &Opt) -> bool {
    let off = matches!((o.duration_secs, tags.duration_ms), (Some(d), Some(ms)) if (d - ms as f64 / 1000.0).abs() > crate::identify::DURATION_SLACK_SECS);
    off || (o.live_only == Some(true) && !crate::same_song::file_is_live(tags))
}

/// What is known of a song waiting for review.
#[derive(Debug, Clone, Copy)]
pub struct Song<'a> {
    pub tags: &'a FileTags,
    pub options: &'a [Opt],
    /// The release the song's folder is on, when one stands out ([`folder_release`]).
    pub folder_release: Option<&'a str>,
    /// The owner rejected some recording for this song: it waits for them.
    pub rejected: bool,
    /// The run's AcoustID answer for the file was read (its every sure recording is among
    /// `options`, with lengths and release kinds): without it, fingerprint options settle nothing.
    pub answer: bool,
}

/// Why a song was settled.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Rule {
    R1,
    R1b,
    R2,
    R3,
}

impl Rule {
    pub fn name(self) -> &'static str {
        match self {
            Rule::R1 => "R1",
            Rule::R1b => "R1b",
            Rule::R2 => "R2",
            Rule::R3 => "R3",
        }
    }
}

/// Why a song stays for the owner.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Stay {
    /// The owner rejected a recording for it.
    Rejected,
    /// A sure option (or the folder's) of the file's title in another version.
    Version,
    /// The file's song by another artist (or an artist missing to compare).
    Artist,
    /// The file's album names a version and no option is on a release named so.
    AlbumVersion,
    /// The folder names another song than the fingerprint.
    Folder,
    /// Sure fingerprint options of other songs only.
    DifferentSongs,
    /// No sure evidence (search or weak fingerprint options only, the folder not deciding).
    Weak,
    /// The file has no title or artist to compare.
    NoTags,
    /// Editions of the folder that fit as well name different recordings.
    Editions,
    /// Fingerprint options, but the run's AcoustID answer isn't kept to check them against.
    NoAnswer,
    /// No recording of the settled group has a known length within 3 s of the file's.
    LengthUnknown,
    /// Nothing open.
    Empty,
}

impl Stay {
    pub fn name(self) -> &'static str {
        match self {
            Stay::Rejected => "rejected",
            Stay::Version => "version",
            Stay::Artist => "artist",
            Stay::AlbumVersion => "album_says_version",
            Stay::Folder => "folder",
            Stay::DifferentSongs => "different_songs",
            Stay::Weak => "weak",
            Stay::NoTags => "no_tags",
            Stay::Editions => "editions",
            Stay::NoAnswer => "no_answer",
            Stay::LengthUnknown => "length_unknown",
            Stay::Empty => "empty",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Verdict {
    /// Settle on `recording` (`release` only when the folder names it), with `score`.
    Pick {
        recording: String,
        release: Option<String>,
        score: f32,
        rule: Rule,
    },
    Stay(Stay),
}

/// Whether an option is the file's song: its title and version, and its artist.
fn agrees(tags: &FileTags, o: &Opt) -> bool {
    match (o.title.as_deref(), o.artist.as_deref()) {
        (Some(t), Some(a)) => same_song_as(tags, t, None, Some(a)) && same_artist(&tags.artist, a),
        _ => false,
    }
}

/// Whether an option's release is named as another version than the file says it is ("Live at
/// Wembley" for a studio file). Rows carry no release-group types, so the name is all there is.
fn release_says_other_version(tags: &FileTags, o: &Opt) -> bool {
    let Some(rt) = o.release_title.as_deref() else {
        return false;
    };
    let mut file: Vec<&str> = version_marks(&tags.title);
    file.extend(marks_in(tags.album.as_deref().unwrap_or_default()));
    marks_in(rt).iter().any(|m| !file.contains(m))
}

/// The settled group's recording: on the folder's release, then on a release named like the
/// file's album, then the lowest id. Its release when the folder names it (the folder's release,
/// or the folder's own fit of the album lane).
fn pick<'a>(tags: &FileTags, pool: &[&'a Opt], folder: Option<&str>) -> Option<(&'a Opt, Option<String>)> {
    fn on_folder(o: &Opt, folder: Option<&str>) -> bool {
        folder.is_some() && o.release.as_deref() == folder
    }
    let key = |o: &'a Opt| (!on_folder(o, folder), !named(o, tags), o.recording.as_str());
    let best: &'a Opt = pool.iter().copied().min_by(|a, b| key(a).cmp(&key(b)))?;
    let release = if on_folder(best, folder) || best.source == "album" {
        best.release.clone()
    } else {
        None
    };
    Some((best, release))
}

/// Whether an option is on a release named like the file's album.
fn named(o: &Opt, tags: &FileTags) -> bool {
    o.release_title
        .as_deref()
        .is_some_and(|t| named_like_album(tags, t))
}

/// Whether an option's recording has a known length within 3 s of the file's.
fn length_fits(o: &Opt, tags: &FileTags) -> bool {
    matches!((o.duration_secs, tags.duration_ms), (Some(d), Some(ms)) if (d - ms as f64 / 1000.0).abs() <= crate::identify::DURATION_SLACK_SECS)
}

/// Settle on one of a group of the file's song's recordings (R1, R1b, R2): an album that names a
/// version needs one on a release named so; only a recording whose length is known to fit can
/// be picked (as a run's own rule for one audio under several recordings requires).
fn settle_group(tags: &FileTags, group: &[&Opt], folder: Option<&str>, rule: Rule) -> Verdict {
    if album_says_version(tags) && !group.iter().any(|o| named(o, tags)) {
        return Verdict::Stay(Stay::AlbumVersion);
    }
    let fitting: Vec<&Opt> = group.iter().copied().filter(|o| length_fits(o, tags)).collect();
    match pick(tags, &fitting, folder) {
        Some((o, release)) => Verdict::Pick {
            recording: o.recording.clone(),
            release,
            score: best_score(group).min(0.99),
            rule,
        },
        None => Verdict::Stay(Stay::LengthUnknown),
    }
}

/// The best score of a group of options.
fn best_score(g: &[&Opt]) -> f32 {
    g.iter().map(|o| o.score).fold(0.0f32, f32::max)
}

/// Decide one song ([module docs](self)).
pub fn decide(song: &Song) -> Verdict {
    let tags = song.tags;
    if song.rejected {
        return Verdict::Stay(Stay::Rejected);
    }
    if song.options.is_empty() {
        return Verdict::Stay(Stay::Empty);
    }
    let file_plain = plain_title(&tags.title);
    if file_plain.is_empty() || plain_name(&tags.artist).is_empty() {
        return Verdict::Stay(Stay::NoTags);
    }
    let strong: Vec<&Opt> = song
        .options
        .iter()
        .filter(|o| o.source == "fingerprint" && o.score >= STRONG)
        .collect();
    let album: Vec<&Opt> = song.options.iter().filter(|o| o.source == "album").collect();
    // The file's title in another version, from the audio or the folder: a real question
    for o in strong.iter().chain(album.iter()) {
        let Some(t) = o.title.as_deref() else { continue };
        if plain_title(t) == file_plain && !same_song_as(tags, t, None, o.artist.as_deref()) {
            return Verdict::Stay(Stay::Version);
        }
    }
    // The file's song by another artist (or by nobody to compare with)
    for o in &strong {
        let Some(t) = o.title.as_deref() else { continue };
        if same_song_as(tags, t, None, o.artist.as_deref())
            && !o.artist.as_deref().is_some_and(|a| same_artist(&tags.artist, a))
        {
            return Verdict::Stay(Stay::Artist);
        }
    }
    let agreeing: Vec<&Opt> = strong.iter().copied().filter(|o| agrees(tags, o)).collect();
    if agreeing
        .iter()
        .any(|o| release_says_other_version(tags, o) || other_take(tags, o))
    {
        return Verdict::Stay(Stay::Version);
    }
    // The folder naming another song than the file says
    if album.iter().any(|o| !agrees(tags, o)) {
        return Verdict::Stay(Stay::Folder);
    }
    // Sure fingerprint options are checked against the run's whole answer, or settle nothing
    if !strong.is_empty() && !song.answer {
        return Verdict::Stay(Stay::NoAnswer);
    }
    if !agreeing.is_empty() {
        // R1: every sure option is the file's song; R2: those that are, are one song (they agree
        // with the file, so they are), the rest name other songs
        let rule = if agreeing.len() == strong.len() {
            Rule::R1
        } else {
            Rule::R2
        };
        // The audio's recordings: the folder's own option may be another recording of the song
        return settle_group(tags, &agreeing, song.folder_release, rule);
    }
    // A search by the file's tags that found the file's song: the tags and the audio disagree
    let tags_searched = song
        .options
        .iter()
        .any(|o| o.source == "search" && agrees(tags, o));
    // R1b: the audio unanimous on one song by the file's artist, in the file's version, under
    // another title; not when the folder names a song (it agrees with the file, not the audio)
    if !strong.is_empty() && album.is_empty() {
        let first = strong[0];
        let one_song = strong.iter().all(|o| {
            match (
                o.title.as_deref(),
                o.artist.as_deref(),
                first.title.as_deref(),
                first.artist.as_deref(),
            ) {
                (Some(t), Some(a), Some(t0), Some(a0)) => {
                    plain_title(t) == plain_title(t0)
                        && plain_name(a) == plain_name(a0)
                        && same_version(t0, None, t, None, &[], &[a0, a])
                }
                _ => false,
            }
        });
        let artist_ok = first
            .artist
            .as_deref()
            .is_some_and(|a| same_artist(&tags.artist, a));
        let version_ok = first.title.as_deref().is_some_and(|t| {
            same_version(
                &tags.title,
                tags.album.as_deref(),
                t,
                None,
                &[],
                &[tags.artist.as_str(), first.artist.as_deref().unwrap_or_default()],
            )
        });
        if one_song
            && artist_ok
            && version_ok
            && !strong
                .iter()
                .any(|o| release_says_other_version(tags, o) || other_take(tags, o))
            && !tags_searched
        {
            return settle_group(tags, &strong, song.folder_release, Rule::R1b);
        }
    }
    if !strong.is_empty() {
        // The folder names the file's song, the audio others; or the audio others only
        return Verdict::Stay(if album.is_empty() {
            Stay::DifferentSongs
        } else {
            Stay::Folder
        });
    }
    // Editions of the folder that fit as well name different recordings: which recording is not
    // for the release's type (nor anything else here) to say
    let mut editions: Vec<&str> = album.iter().map(|o| o.recording.as_str()).collect();
    editions.sort_unstable();
    editions.dedup();
    if editions.len() > 1 {
        return Verdict::Stay(Stay::Editions);
    }
    // R3: the folder's release holds exactly one recording of the file's song among the options
    if let Some(folder) = song.folder_release {
        let mut on: Vec<&Opt> = song
            .options
            .iter()
            .filter(|o| o.release.as_deref() == Some(folder) && o.score >= R3_MIN && agrees(tags, o))
            .collect();
        on.sort_by(|a, b| a.recording.cmp(&b.recording));
        on.dedup_by(|a, b| a.recording == b.recording);
        if let [o] = on[..] {
            // An album that names a version: only an option on a release named so
            if album_says_version(tags) && !named(o, tags) {
                return Verdict::Stay(Stay::AlbumVersion);
            }
            if !release_says_other_version(tags, o) {
                return Verdict::Pick {
                    recording: o.recording.clone(),
                    release: Some(folder.to_string()),
                    score: o.score.min(0.99),
                    rule: Rule::R3,
                };
            }
        }
    }
    Verdict::Stay(Stay::Weak)
}

/// The release a folder is on, from its songs identified by a run, their tags or the owner
/// (never an automatic pick): the one most of them are on; none on a tie.
pub fn folder_release<'a>(releases: impl IntoIterator<Item = &'a str>) -> Option<String> {
    let mut n: std::collections::BTreeMap<&str, usize> = Default::default();
    for r in releases {
        *n.entry(r).or_default() += 1;
    }
    let mut ranked: Vec<(&str, usize)> = n.into_iter().collect();
    ranked.sort_by_key(|x| std::cmp::Reverse(x.1));
    let (best, top) = *ranked.first()?;
    if ranked.get(1).is_some_and(|x| x.1 == top) {
        return None;
    }
    Some(best.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tags(title: &str, artist: &str, album: &str) -> FileTags {
        FileTags {
            title: title.into(),
            artist: artist.into(),
            album: Some(album.into()),
            duration_ms: Some(200_000),
        }
    }

    fn rec(i: u32) -> String {
        format!("00000000-0000-4000-8000-{i:012}")
    }

    fn fp(i: u32, title: &str, artist: &str, score: f32) -> Opt {
        Opt {
            recording: rec(i),
            release: None,
            title: Some(title.into()),
            artist: Some(artist.into()),
            release_title: None,
            score,
            source: "fingerprint".into(),
            // As long as the files of [`tags`]
            duration_secs: Some(200.0),
            live_only: None,
        }
    }

    fn on(mut o: Opt, release: u32, release_title: &str) -> Opt {
        o.release = Some(rec(900 + release));
        o.release_title = Some(release_title.into());
        o
    }

    fn src(mut o: Opt, source: &str) -> Opt {
        o.source = source.into();
        o
    }

    fn run(t: &FileTags, options: &[Opt], folder: Option<&str>) -> Verdict {
        decide(&Song {
            tags: t,
            options,
            folder_release: folder,
            rejected: false,
            answer: true,
        })
    }

    fn picked(v: &Verdict) -> (String, Option<String>, Rule) {
        match v {
            Verdict::Pick {
                recording,
                release,
                rule,
                ..
            } => (recording.clone(), release.clone(), *rule),
            other => panic!("not settled: {other:?}"),
        }
    }

    #[test]
    fn r1_one_audio_under_several_recordings_picks_the_same_one_every_time() {
        let t = tags(
            "Over My Shoulder",
            "Mike + The Mechanics",
            "Beggar on a Beach of Gold",
        );
        let opts = vec![
            fp(7, "Over My Shoulder", "Mike + The Mechanics", 0.96),
            fp(
                3,
                "Over My Shoulder (2004 Remaster)",
                "Mike + The Mechanics",
                0.96,
            ),
            fp(5, "Over My Shoulder", "Mike + The Mechanics", 0.95),
        ];
        let v = run(&t, &opts, None);
        assert_eq!(
            picked(&v),
            (rec(3), None, Rule::R1),
            "the lowest id; a remaster is the same"
        );
        let mut rev = opts.clone();
        rev.reverse();
        assert_eq!(run(&t, &rev, None), v, "never the answer's order");
        // One on a release named like the file's album comes first
        let mut named = opts.clone();
        named[2] = on(named[2].clone(), 1, "Beggar on a Beach of Gold");
        assert_eq!(
            picked(&run(&t, &named, None)),
            (rec(5), None, Rule::R1),
            "the edition stays open"
        );
        // One on the folder's release comes first, and keeps it
        let mut folder = opts.clone();
        folder[0] = on(folder[0].clone(), 2, "Hits");
        let f = rec(902);
        assert_eq!(
            picked(&run(&t, &folder, Some(&f))),
            (rec(7), Some(f.clone()), Rule::R1)
        );
        // Weak and search options don't stand in the way
        let mut more = opts.clone();
        more.push(fp(9, "Another Song", "Someone", 0.6));
        more.push(src(
            fp(8, "Over My Shoulder (Live)", "Mike + The Mechanics", 1.0),
            "search",
        ));
        assert_eq!(picked(&run(&t, &more, None)).0, rec(3));
    }

    #[test]
    fn r1b_the_audio_is_unanimous_and_the_title_tag_says_otherwise() {
        let t = tags("Track 01", "Band", "Album");
        let opts = vec![fp(2, "Real Song", "Band", 0.97), fp(1, "Real Song", "Band", 0.93)];
        assert_eq!(picked(&run(&t, &opts, None)), (rec(1), None, Rule::R1b));
        // Another artist: stays
        let t2 = tags("Track 01", "Other Band", "Album");
        assert_eq!(run(&t2, &opts, None), Verdict::Stay(Stay::DifferentSongs));
        // The audio not unanimous: stays
        let split = vec![
            fp(2, "Real Song", "Band", 0.97),
            fp(1, "Other Song", "Band", 0.93),
        ];
        assert_eq!(run(&t, &split, None), Verdict::Stay(Stay::DifferentSongs));
        // The file says another version than the audio: stays
        let live = tags("Track 01 (Live)", "Band", "Album");
        assert_eq!(run(&live, &opts, None), Verdict::Stay(Stay::DifferentSongs));
        // The folder names the file's song: the folder and the audio disagree
        let mut folder = opts.clone();
        folder.push(src(fp(5, "Track 01", "Band", 0.9), "album"));
        assert_eq!(run(&t, &folder, None), Verdict::Stay(Stay::Folder));
    }

    #[test]
    fn r2_the_tags_and_the_audio_agree_on_one_song_among_others() {
        let t = tags("Song", "Band", "Album");
        let opts = vec![
            fp(4, "Song", "Band", 0.95),
            fp(2, "Quite Another Song", "Band", 0.95),
            fp(3, "A Third Song", "Someone", 0.95),
        ];
        assert_eq!(picked(&run(&t, &opts, None)), (rec(4), None, Rule::R2));
        // Nothing agrees with the tags: the fingerprint's songs are a real question
        let t2 = tags("Unrelated", "Band", "Album");
        assert_eq!(run(&t2, &opts, None), Verdict::Stay(Stay::DifferentSongs));
    }

    #[test]
    fn r3_the_folders_release_names_one_recording() {
        let t = tags("Song", "Band", "Album");
        let folder = rec(901);
        let opts = vec![
            on(src(fp(1, "Song", "Band", 0.95), "album"), 1, "Album"),
            on(src(fp(2, "Song", "Band", 0.95), "album"), 2, "Album"),
        ];
        // Two editions of the folder naming different recordings: never decided by which one the
        // folder's other songs went to (owner, 2026-11-03: types choose the release, not the
        // recording)
        assert_eq!(run(&t, &opts, Some(&folder)), Verdict::Stay(Stay::Editions));
        assert_eq!(run(&t, &opts, None), Verdict::Stay(Stay::Editions));
        // One edition's recording: the folder's
        assert_eq!(
            picked(&run(&t, &opts[..1], Some(&folder))),
            (rec(1), Some(folder.clone()), Rule::R3)
        );
        // Search options: one on the folder's release
        let s = vec![
            on(src(fp(5, "Song", "Band", 0.9), "search"), 1, "Album"),
            on(src(fp(6, "Song", "Band", 0.95), "search"), 3, "Best Of"),
        ];
        assert_eq!(
            picked(&run(&t, &s, Some(&folder))),
            (rec(5), Some(folder.clone()), Rule::R3)
        );
        // Too weak to publish: stays
        let weak = vec![on(src(fp(5, "Song", "Band", 0.7), "search"), 1, "Album")];
        assert_eq!(run(&t, &weak, Some(&folder)), Verdict::Stay(Stay::Weak));
    }

    #[test]
    fn genuine_conflicts_stay() {
        let t = tags("Song", "Band", "Album");
        // Live against studio
        let live = vec![fp(1, "Song", "Band", 0.95), fp(2, "Song (Live)", "Band", 0.95)];
        assert_eq!(run(&t, &live, None), Verdict::Stay(Stay::Version));
        let remix = vec![fp(2, "Song (Bob Remix)", "Band", 0.92)];
        assert_eq!(run(&t, &remix, None), Verdict::Stay(Stay::Version));
        // The folder's live take against the audio
        let folder_live = vec![src(fp(1, "Song (Live at Wembley)", "Band", 0.95), "album")];
        assert_eq!(run(&t, &folder_live, None), Verdict::Stay(Stay::Version));
        // Only on a release named live, for a studio file
        let named_live = vec![on(fp(1, "Song", "Band", 0.95), 1, "Live at Wembley")];
        assert_eq!(run(&t, &named_live, None), Verdict::Stay(Stay::Version));
        // One audio under recordings of another length, or only on live releases (from the run's
        // kept AcoustID answer)
        let mut longer = fp(2, "Song", "Band", 0.95);
        longer.duration_secs = Some(215.0);
        assert_eq!(
            run(&t, &[fp(1, "Song", "Band", 0.95), longer], None),
            Verdict::Stay(Stay::Version)
        );
        let mut close = fp(2, "Song", "Band", 0.95);
        close.duration_secs = Some(201.5);
        assert!(matches!(
            run(&t, &[fp(1, "Song", "Band", 0.95), close], None),
            Verdict::Pick { .. }
        ));
        let mut on_live = fp(2, "Song", "Band", 0.95);
        on_live.live_only = Some(true);
        assert_eq!(
            run(&t, &[fp(1, "Song", "Band", 0.95), on_live], None),
            Verdict::Stay(Stay::Version)
        );
        // The audio names another song, a search by the tags the file's: they disagree
        let mut searched = fp(3, "Song", "Band", 0.9);
        searched.source = "search".into();
        assert_eq!(
            run(&t, &[fp(1, "Other", "Band", 0.97), searched], None),
            Verdict::Stay(Stay::DifferentSongs)
        );
        // Another artist
        let cover = vec![fp(1, "Song", "Band", 0.95), fp(2, "Song", "Other Band", 0.95)];
        assert_eq!(run(&t, &cover, None), Verdict::Stay(Stay::Artist));
        // Different songs, none the file's
        let other = vec![fp(1, "This", "Band", 0.95), fp(2, "That", "Band", 0.95)];
        assert_eq!(run(&t, &other, None), Verdict::Stay(Stay::DifferentSongs));
        // A rejected recording for the song: the owner decides
        let opts = vec![fp(1, "Song", "Band", 0.95)];
        let v = decide(&Song {
            tags: &t,
            options: &opts,
            folder_release: None,
            rejected: true,
            answer: true,
        });
        assert_eq!(v, Verdict::Stay(Stay::Rejected));
        // An album that names a version, no option on it
        let rr = tags("Song", "Band", "Album (Re-Recorded)");
        assert_eq!(run(&rr, &opts, None), Verdict::Stay(Stay::AlbumVersion));
        // The folder names another song than the audio
        let disagree = vec![
            fp(1, "Song", "Band", 0.95),
            src(fp(2, "Other", "Band", 0.9), "album"),
        ];
        assert_eq!(run(&t, &disagree, None), Verdict::Stay(Stay::Folder));
        // Weak only
        let weak = vec![
            fp(1, "Song", "Band", 0.85),
            src(fp(2, "Song", "Band", 1.0), "search"),
        ];
        assert_eq!(run(&t, &weak, None), Verdict::Stay(Stay::Weak));
        // No tags to compare
        assert_eq!(
            run(&tags("", "Band", "Album"), &opts, None),
            Verdict::Stay(Stay::NoTags)
        );
        assert_eq!(run(&t, &[], None), Verdict::Stay(Stay::Empty));
    }

    /// Review r1: the run's whole answer decides, a length must be known, and the album's
    /// version is checked against the recording picked.
    #[test]
    fn the_runs_whole_answer_and_known_lengths_decide() {
        let t = tags("Song", "Band", "Album");
        let opts = vec![fp(1, "Song", "Band", 0.95), fp(2, "Song", "Band", 0.95)];
        // No kept answer: the fingerprint options settle nothing
        let v = decide(&Song {
            tags: &t,
            options: &opts,
            folder_release: None,
            rejected: false,
            answer: false,
        });
        assert_eq!(v, Verdict::Stay(Stay::NoAnswer));
        // A sixth sure recording the run didn't keep as an option: a live take, from the answer
        let answer: Vec<crate::identify::AcoustIdResult> = serde_json::from_value(serde_json::json!([
            {"id": "a", "score": 0.95, "recordings": [
                {"id": rec(1), "title": "Song", "duration": 200.0, "artists": [{"name": "Band"}]},
                {"id": rec(2), "title": "Song", "duration": 201.0, "artists": [{"name": "Band"}]},
                {"id": rec(7), "title": "Song (Live)", "duration": 260.0, "artists": [{"name": "Band"}]}
            ]}
        ]))
        .unwrap();
        let none = |_: &str| false;
        let merged = with_answer(opts.clone(), &answer, &none);
        assert_eq!(merged.len(), 3);
        assert_eq!(run(&t, &merged, None), Verdict::Stay(Stay::Version));
        // Without it, the answer's lengths settle on the lowest id
        let fine: Vec<crate::identify::AcoustIdResult> = serde_json::from_value(serde_json::json!([
            {"id": "a", "score": 0.95, "recordings": [
                {"id": rec(2), "title": "Song", "duration": 201.0, "artists": [{"name": "Band"}]},
                {"id": rec(1), "title": "Song", "artists": [{"name": "Band"}]}
            ]}
        ]))
        .unwrap();
        let mut unknown = opts.clone();
        unknown.iter_mut().for_each(|o| o.duration_secs = None);
        let merged = with_answer(unknown.clone(), &fine, &none);
        assert_eq!(
            picked(&run(&t, &merged, None)).0,
            rec(2),
            "only a recording whose length is known to fit"
        );
        // No length known at all: stays
        assert_eq!(run(&t, &unknown, None), Verdict::Stay(Stay::LengthUnknown));
        // A rejected recording of the answer stays out
        let merged = with_answer(opts.clone(), &answer, &|r: &str| r == rec(7));
        assert_eq!(merged.len(), 2);
        // The album says live: a search on that release doesn't let a studio recording through
        let live = tags("Song", "Band", "Live at X");
        let mut searched = on(fp(9, "Song", "Band", 0.9), 1, "Live at X");
        searched.source = "search".into();
        assert_eq!(
            run(&live, &[fp(1, "Song", "Band", 0.95), searched], None),
            Verdict::Stay(Stay::AlbumVersion)
        );
    }

    #[test]
    fn the_folders_release_is_the_most_common_one() {
        assert_eq!(folder_release(["a", "a", "b"]).as_deref(), Some("a"));
        assert_eq!(folder_release(["a", "b"]), None);
        assert_eq!(folder_release(Vec::<&str>::new()), None);
    }
}
