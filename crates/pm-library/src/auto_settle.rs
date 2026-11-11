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
//! - **R4**: no sure fingerprint option may be this audio, a search by the file's tags found the
//!   file's song, and a fingerprint option of it (score 0.5 or more) has a known length within
//!   3 s of the file's: two kinds of evidence, the tags' and the audio's, agree.
//! - **R5**: no sure fingerprint option may be this audio, and strong searches by the file's tags
//!   (score 0.9 or more) found exactly the file's song, of a known length within 3 s of the
//!   file's (the brief's exact title, artist and length).
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

use crate::album_fit::{marks_in, qualifiers, qualifiers_credited, same_version, version_marks};
use crate::matching::{plain_name, plain_title};
use crate::same_song::{file_is_live, named_like_album, same_artist, same_song_as, FileTags};
use pm_model::Confidence;

/// A fingerprint option at or above this is sure (AcoustID's accept threshold, not lowered).
pub const STRONG: f32 = Confidence::FINGERPRINT_ACCEPT;
/// The least fingerprint score that, with a search by the file's tags agreeing and a length that
/// fits, settles a song (R4): the audio's own evidence, weaker than a sure match.
pub const R4_AUDIO: f32 = 0.5;
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
    /// One of the song's stored options (else it came from the kept AcoustID answer only).
    pub stored: bool,
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
            stored: true,
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
                    stored: false,
                }),
            }
        }
    }
    options.into_iter().map(|o| o.with_audio(results)).collect()
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
    R4,
    R5,
}

impl Rule {
    /// Every rule.
    pub const ALL: [Rule; 6] = [Rule::R1, Rule::R1b, Rule::R2, Rule::R3, Rule::R4, Rule::R5];

    pub fn name(self) -> &'static str {
        match self {
            Rule::R1 => "R1",
            Rule::R1b => "R1b",
            Rule::R2 => "R2",
            Rule::R3 => "R3",
            Rule::R4 => "R4",
            Rule::R5 => "R5",
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
    /// Every reason.
    pub const ALL: [Stay; 12] = [
        Stay::Rejected,
        Stay::Version,
        Stay::Artist,
        Stay::AlbumVersion,
        Stay::Folder,
        Stay::DifferentSongs,
        Stay::Weak,
        Stay::NoTags,
        Stay::Editions,
        Stay::NoAnswer,
        Stay::LengthUnknown,
        Stay::Empty,
    ];

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
        (Some(t), Some(a)) => same_song_here(tags, t, Some(a)) && artist_match(&tags.artist, a).agrees(),
        _ => false,
    }
}

/// The version the file's album names, when it names one: a version word in its name ("Live at
/// Wembley", "The Remix Album") or a qualifier saying it was recorded again ("Re-Recorded",
/// "Taylor's Version"). Other qualifiers (a disc, an edition, a country) name no version.
fn album_version(tags: &FileTags) -> Option<&'static str> {
    let album = tags.album.as_deref().unwrap_or_default();
    if let Some(m) = marks_in(album).first() {
        return Some(m);
    }
    let again = qualifiers(album).iter().any(|q| {
        q.split_whitespace()
            .any(|w| ["recorded", "rerecorded", "version", "taylor"].contains(&w))
    });
    again.then_some("version")
}

/// Whether a title is the file's song: [`same_song_as`], and on a live album a live recording of
/// it too (the album says what the file's title doesn't).
fn same_song_here(tags: &FileTags, title: &str, credit: Option<&str>) -> bool {
    same_song_as(tags, title, None, credit)
        || (album_version(tags) == Some("live")
            && plain_title(&tags.title) == plain_title(title)
            && same_version(
                &tags.title,
                tags.album.as_deref(),
                title,
                None,
                &["live"],
                &[tags.artist.as_str(), credit.unwrap_or_default()],
            ))
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

/// The settled group's recording: one of the song's stored options first, then on the folder's
/// release, then on a release named like the file's album, then the lowest id. Its release when the folder names it (the folder's release,
/// or the folder's own fit of the album lane).
fn pick<'a>(tags: &FileTags, pool: &[&'a Opt], folder: Option<&str>) -> Option<(&'a Opt, Option<String>)> {
    fn on_folder(o: &Opt, folder: Option<&str>) -> bool {
        folder.is_some() && o.release.as_deref() == folder
    }
    let key = |o: &'a Opt| {
        (
            !o.stored,
            !on_folder(o, folder),
            !named(o, tags),
            o.recording.as_str(),
        )
    };
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

/// The best score of a group of options.
fn best_score(g: &[&Opt]) -> f32 {
    g.iter().map(|o| o.score).fold(0.0f32, f32::max)
}

/// Decide one song ([module docs](self)).
pub fn decide(song: &Song) -> Verdict {
    decide_why(song).0
}

/// [`decide`], and for a song that stays, what in particular kept it (words of [`WHY_WORDS`]
/// and [`MARK_WORDS`] only, never a title or name): `title:live`, `folder:remix`, `various`, `contains`,
/// `tags_row:strong`, `live_only`…
pub fn decide_why(song: &Song) -> (Verdict, Option<String>) {
    let stay = |s: Stay, why: Option<String>| (Verdict::Stay(s), why);
    let tags = song.tags;
    if song.rejected {
        return stay(Stay::Rejected, None);
    }
    if song.options.is_empty() {
        return stay(Stay::Empty, None);
    }
    let file_plain = plain_title(&tags.title);
    if file_plain.is_empty() || plain_name(&tags.artist).is_empty() {
        return stay(Stay::NoTags, None);
    }
    let strong: Vec<&Opt> = song
        .options
        .iter()
        .filter(|o| o.source == "fingerprint" && o.score >= STRONG)
        .collect();
    let album: Vec<&Opt> = song.options.iter().filter(|o| o.source == "album").collect();
    // A sure recording may be this audio unless its known length says it is another take: one
    // more than 3 s off is AcoustID linking the fingerprint to a recording this file isn't, and
    // neither decides nor is ever picked
    let near: Vec<&Opt> = strong.iter().copied().filter(|o| !length_off(o, tags)).collect();
    // The file's title in another version, from the audio or the folder: a real question
    let from_audio = near.iter().map(|o| (*o, "title"));
    let from_folder = album.iter().map(|o| (*o, "folder"));
    for (o, from) in from_audio.chain(from_folder) {
        let Some(t) = o.title.as_deref() else { continue };
        if plain_title(t) == file_plain && !same_song_here(tags, t, o.artist.as_deref()) {
            let gap = version_gap(tags, t, o.artist.as_deref());
            let side = version_side(tags, t, o.artist.as_deref());
            let len = length_word(o, tags);
            return stay(Stay::Version, Some(format!("{from}:{gap}:{side}:{len}")));
        }
    }
    // The file's song by another artist (or by nobody to compare with). When the audio names the
    // file's song under several artists and one of them is the file's, the tags decide (R2)
    let tags_artist_heard = near.iter().any(|o| agrees(tags, o));
    for o in near.iter().filter(|_| !tags_artist_heard) {
        let Some(t) = o.title.as_deref() else { continue };
        if !same_song_here(tags, t, o.artist.as_deref()) {
            continue;
        }
        // (and whether the audio is unanimous on one song, for the reason)
        let unanimous = if one_song(&near) { "unanimous" } else { "split" };
        match o.artist.as_deref().map(|a| artist_match(&tags.artist, a)) {
            Some(m) if m.agrees() => {}
            Some(m) => return stay(Stay::Artist, Some(format!("{}:{unanimous}", m.name()))),
            None => return stay(Stay::Artist, Some(format!("missing:{unanimous}"))),
        }
    }
    // The folder naming another song than the file says
    if album.iter().any(|o| !agrees(tags, o)) {
        return stay(Stay::Folder, None);
    }
    // Sure fingerprint options are checked against the run's whole answer, or settle nothing
    if !strong.is_empty() && !song.answer {
        return stay(Stay::NoAnswer, None);
    }
    let agreeing: Vec<&Opt> = near.iter().copied().filter(|o| agrees(tags, o)).collect();
    if !agreeing.is_empty() {
        // R1: every sure option that may be this audio is the file's song; R2: those that are,
        // are one song (they agree with the file, so they are), the rest name other songs
        let rule = if agreeing.len() == near.len() {
            Rule::R1
        } else {
            Rule::R2
        };
        // The audio's recordings: the folder's own option may be another recording of the song
        return settle_group(tags, &agreeing, song.folder_release, rule);
    }
    // A search by the file's tags that found the file's song: the tags and the audio disagree
    let tags_row = song.options.iter().any(|o| agrees(tags, o));
    let tags_searched = song
        .options
        .iter()
        .any(|o| o.source == "search" && agrees(tags, o));
    // R1b: the audio unanimous on one song by the file's artist, in the file's version, under
    // another title; not when the folder names a song (it agrees with the file, not the audio)
    if !near.is_empty() && album.is_empty() && !tags_searched {
        let first = near[0];
        let artist_ok = first
            .artist
            .as_deref()
            .is_some_and(|a| artist_match(&tags.artist, a).agrees());
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
        if one_song(&near) && artist_ok && version_ok {
            return settle_group(tags, &near, song.folder_release, Rule::R1b);
        }
    }
    // R4: no sure recording that may be this audio; the tags' search and the audio (a weaker
    // fingerprint option of the file's song, of a length that fits) agree
    if near.is_empty() && tags_searched {
        let heard: Vec<&Opt> = song
            .options
            .iter()
            .filter(|o| {
                o.source == "fingerprint" && o.score >= R4_AUDIO && agrees(tags, o) && length_fits(o, tags)
            })
            .collect();
        if !heard.is_empty() {
            let searched = song
                .options
                .iter()
                .filter(|o| o.source == "search" && agrees(tags, o))
                .map(|o| o.score)
                .fold(0.0f32, f32::max);
            return match settle_group(tags, &heard, song.folder_release, Rule::R4) {
                (
                    Verdict::Pick {
                        recording,
                        release,
                        score,
                        rule,
                    },
                    why,
                ) => (
                    Verdict::Pick {
                        recording,
                        release,
                        score: score.max(searched).min(0.99),
                        rule,
                    },
                    why,
                ),
                stays => stays,
            };
        }
    }
    // R5: no sure recording that may be this audio; strong searches by the file's tags found
    // exactly its song, of a known length that fits
    if near.is_empty() {
        let exact: Vec<&Opt> = song
            .options
            .iter()
            .filter(|o| o.source == "search" && o.score >= STRONG && agrees(tags, o) && length_fits(o, tags))
            .collect();
        if !exact.is_empty() {
            return settle_group(tags, &exact, song.folder_release, Rule::R5);
        }
    }
    let rows = match best_agreeing(tags, song.options) {
        Some(o) => {
            let src = match o.source.as_str() {
                "fingerprint" => "fingerprint",
                "album" => "album",
                _ => "search",
            };
            let bucket = match o.score {
                x if x >= 0.9 => "ge90",
                x if x >= 0.8 => "80to90",
                x if x >= 0.5 => "50to80",
                _ => "lt50",
            };
            let len = match off_by(o, tags) {
                Some((secs, share)) => {
                    let by = match secs {
                        x if x <= 3.0 => "len_fit",
                        x if x <= 5.0 => "len_off:3to5s",
                        x if x <= 10.0 => "len_off:5to10s",
                        x if x <= 30.0 => "len_off:10to30s",
                        _ => "len_off:gt30s",
                    };
                    if secs > 3.0 {
                        format!("{by}:{}", if share <= 0.02 { "rel2" } else { "relgt2" })
                    } else {
                        by.to_string()
                    }
                }
                None => "len_unknown".to_string(),
            };
            format!("tags_row:{src}:{bucket}:{len}")
        }
        None => "no_tags_row".to_string(),
    };
    if !strong.is_empty() {
        // The folder names the file's song, the audio others; or the audio others only
        let audio = if near.is_empty() { "no_strong" } else { "strong" };
        let reason = if album.is_empty() {
            Stay::DifferentSongs
        } else {
            Stay::Folder
        };
        return stay(reason, Some(format!("{rows}:{audio}")));
    }
    // Editions of the folder that fit as well name different recordings: which recording is not
    // for the release's type (nor anything else here) to say
    let mut editions: Vec<&str> = album.iter().map(|o| o.recording.as_str()).collect();
    editions.sort_unstable();
    editions.dedup();
    if editions.len() > 1 {
        return stay(Stay::Editions, None);
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
            if let Some(m) = album_version(tags).filter(|_| !named(o, tags)) {
                return stay(Stay::AlbumVersion, Some(format!("album:{m}")));
            }
            if !release_says_other_version(tags, o) {
                return (
                    Verdict::Pick {
                        recording: o.recording.clone(),
                        release: Some(folder.to_string()),
                        score: o.score.min(0.99),
                        rule: Rule::R3,
                    },
                    None,
                );
            }
        }
    }
    stay(Stay::Weak, Some(format!("{rows}:no_strong")))
}

/// The words that name a version ([`version_gap`]).
pub const MARK_WORDS: &[&str] = &[
    "live",
    "remix",
    "mix",
    "edit",
    "acoustic",
    "demo",
    "instrumental",
    "unplugged",
    "karaoke",
    "acapella",
    "mono",
    "extended",
    "rehearsal",
    "stereo",
    "explicit",
    "clean",
    "version",
    "take",
    "session",
    "cover",
    "radio",
    "original",
    "main",
    "single",
    "bonus",
    "deluxe",
    "remaster",
    "edition",
    "feat",
    "part",
    "reprise",
    "intro",
    "outro",
    "interlude",
    "medley",
    "dub",
    "club",
    "vip",
    "rework",
    "alternate",
    "early",
    "rough",
    "outtake",
    "bbc",
    "peel",
    "soundtrack",
    "theme",
    "number",
    "other",
];

/// The other words a song's reason to stay is told in ([`decide_why`]), with [`MARK_WORDS`]:
/// nothing of a title or name.
pub const WHY_WORDS: &[&str] = &[
    "title",
    "folder",
    "various",
    "contains",
    "spelling",
    "different",
    "missing",
    "tags_row",
    "no_tags_row",
    "strong",
    "no_strong",
    "no_length",
    "live_only",
    "release_version",
    "studio_only",
    "album",
    "search",
    "fingerprint",
    "ge90",
    "80to90",
    "50to80",
    "lt50",
    "len_fit",
    "len_off",
    "len_unknown",
    "unanimous",
    "split",
    "rows",
    "tag",
    "both",
    "3to5s",
    "5to10s",
    "10to30s",
    "gt30s",
    "rel2",
    "relgt2",
    "input",
    "no_file",
    "identified",
    "left",
    "not_offered",
    "rejected",
    "changed",
];

/// The word that names how a candidate's version differs from the file's (its qualifiers the file
/// doesn't have, else the file's it doesn't have): one of [`WHY_WORDS`], `other` for any word not
/// listed there.
fn version_gap(tags: &FileTags, title: &str, credit: Option<&str>) -> &'static str {
    let credits = [tags.artist.as_str(), credit.unwrap_or_default()];
    let file = qualifiers_credited(&tags.title, &credits);
    let cand = qualifiers_credited(title, &credits);
    let extra: Vec<&String> = cand.iter().filter(|q| !file.contains(q)).collect();
    let text = if extra.is_empty() {
        file.join(" ")
    } else {
        extra.iter().map(|q| q.as_str()).collect::<Vec<_>>().join(" ")
    };
    if let Some(m) = marks_in(&text).first() {
        return m;
    }
    let lower = text.to_lowercase();
    let words: Vec<&str> = lower
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .collect();
    if let Some(w) = words
        .iter()
        .find_map(|w| MARK_WORDS.iter().find(|x| *x == w).copied())
    {
        return w;
    }
    if words.iter().any(|w| w.chars().all(|c| c.is_ascii_digit())) {
        return "number";
    }
    "other"
}

/// How a recording's credit compares with the file's artist.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ArtistMatch {
    /// The same name, or that name featuring someone ([`same_artist`]).
    Same,
    /// The same once accents, a leading "The" and punctuation are set aside.
    Spelling,
    /// One credit's artists are all in the other's ("A & B" and "A").
    Contains,
    /// "Various Artists" on either side: nothing to compare.
    Various,
    Different,
}

impl ArtistMatch {
    fn agrees(self) -> bool {
        matches!(self, Self::Same | Self::Spelling | Self::Contains)
    }

    fn name(self) -> &'static str {
        match self {
            Self::Same => "same",
            Self::Spelling => "spelling",
            Self::Contains => "contains",
            Self::Various => "various",
            Self::Different => "different",
        }
    }
}

/// A name as people compare them: lower case, accents off, punctuation as spaces, no leading
/// "The".
fn fold(s: &str) -> String {
    let plain: String = s
        .to_lowercase()
        .chars()
        .map(|c| match c {
            'à' | 'á' | 'â' | 'ã' | 'ä' | 'å' | 'ā' | 'ă' | 'ą' => 'a',
            'ç' | 'ć' | 'č' => 'c',
            'ď' | 'đ' => 'd',
            'è' | 'é' | 'ê' | 'ë' | 'ē' | 'ė' | 'ę' | 'ě' => 'e',
            'ì' | 'í' | 'î' | 'ï' | 'ī' | 'į' | 'ı' => 'i',
            'ł' | 'ľ' | 'ĺ' => 'l',
            'ñ' | 'ń' | 'ň' => 'n',
            'ò' | 'ó' | 'ô' | 'õ' | 'ö' | 'ø' | 'ō' | 'ő' => 'o',
            'ŕ' | 'ř' => 'r',
            'ś' | 'š' | 'ş' | 'ß' => 's',
            'ť' | 'ţ' => 't',
            'ù' | 'ú' | 'û' | 'ü' | 'ū' | 'ů' | 'ű' => 'u',
            'ý' | 'ÿ' => 'y',
            'ź' | 'ż' | 'ž' => 'z',
            c => c,
        })
        .collect();
    let p = plain_name(&plain);
    match p.strip_prefix("the ") {
        Some(rest) if !rest.is_empty() => rest.to_string(),
        _ => p,
    }
}

/// The artists a credit names ("A feat. B & C" → a, b, c), each [`fold`]ed.
fn artist_parts(s: &str) -> Vec<String> {
    let mut t = format!(" {} ", s.to_lowercase());
    for sep in [
        " featuring ",
        " feat. ",
        " feat ",
        " ft. ",
        " ft ",
        " with ",
        " and ",
        " x ",
        " vs. ",
        " vs ",
    ] {
        t = t.replace(sep, "|");
    }
    t.split(['|', ',', ';', '/', '&', '+'])
        .map(fold)
        .filter(|p| !p.is_empty())
        .collect()
}

fn various(s: &str) -> bool {
    matches!(
        fold(s).as_str(),
        "various artists" | "various" | "va" | "various artist"
    )
}

/// How a recording's credit compares with the file's artist ([`ArtistMatch`]).
fn artist_match(file: &str, credit: &str) -> ArtistMatch {
    if various(file) || various(credit) {
        return ArtistMatch::Various;
    }
    if same_artist(file, credit) {
        return ArtistMatch::Same;
    }
    let (f, c) = (fold(file), fold(credit));
    if !f.is_empty() && f == c {
        return ArtistMatch::Spelling;
    }
    let (fp, cp) = (artist_parts(file), artist_parts(credit));
    if !fp.is_empty()
        && !cp.is_empty()
        && (fp.iter().all(|x| cp.contains(x)) || cp.iter().all(|x| fp.contains(x)))
    {
        return ArtistMatch::Contains;
    }
    ArtistMatch::Different
}

/// Settle on one of a group of the file's song's recordings (R1, R1b, R2). An album that names
/// a version needs one on a release named so. Only a recording whose length is known to fit can
/// be picked (as a run's own rule for one audio under several recordings requires), never one
/// only on live releases for a file that isn't live, nor one on a release whose name says
/// another version: those are left out, and when none is left the song stays.
fn settle_group(
    tags: &FileTags,
    group: &[&Opt],
    folder: Option<&str>,
    rule: Rule,
) -> (Verdict, Option<String>) {
    // An album that names a version: a recording on a release named so; on a live album, a live
    // recording (the album says the file is live)
    let album = album_version(tags);
    let live_album = album == Some("live");
    if let Some(m) = album.filter(|_| !live_album && !group.iter().any(|o| named(o, tags))) {
        return (Verdict::Stay(Stay::AlbumVersion), Some(format!("album:{m}")));
    }
    let live_file = file_is_live(tags) || live_album;
    let mut why = "no_length";
    let pool: Vec<&Opt> = group
        .iter()
        .copied()
        .filter(|o| {
            if !length_fits(o, tags) {
                return false;
            }
            if o.live_only == Some(true) && !live_file {
                why = "live_only";
                return false;
            }
            if release_says_other_version(tags, o) {
                why = "release_version";
                return false;
            }
            let live = o.live_only == Some(true)
                || o.title
                    .as_deref()
                    .is_some_and(|t| version_marks(t).contains(&"live"))
                || named(o, tags);
            if live_album && !live {
                why = "studio_only";
                return false;
            }
            true
        })
        .collect();
    match pick(tags, &pool, folder) {
        Some((o, release)) => (
            Verdict::Pick {
                recording: o.recording.clone(),
                release,
                score: best_score(group).min(0.99),
                rule,
            },
            None,
        ),
        None if why == "no_length" => (Verdict::Stay(Stay::LengthUnknown), Some(why.into())),
        None if why == "studio_only" => (Verdict::Stay(Stay::AlbumVersion), Some("album:live".into())),
        None => (Verdict::Stay(Stay::Version), Some(why.into())),
    }
}

/// How far a recording's known length is off the file's: seconds, and the share of the file's
/// length (`None`: a length unknown).
fn off_by(o: &Opt, tags: &FileTags) -> Option<(f64, f64)> {
    let (d, ms) = (o.duration_secs?, tags.duration_ms?);
    let file = ms as f64 / 1000.0;
    let secs = (d - file).abs();
    Some((secs, if file > 0.0 { secs / file } else { 1.0 }))
}

/// A recording's length against the file's, in a word.
fn length_word(o: &Opt, tags: &FileTags) -> &'static str {
    if length_fits(o, tags) {
        "len_fit"
    } else if length_off(o, tags) {
        "len_off"
    } else {
        "len_unknown"
    }
}

/// Which side names a version the other doesn't: the candidate (`rows`), the file (`tag`) or
/// each its own (`both`).
fn version_side(tags: &FileTags, title: &str, credit: Option<&str>) -> &'static str {
    let credits = [tags.artist.as_str(), credit.unwrap_or_default()];
    let file = qualifiers_credited(&tags.title, &credits);
    let cand = qualifiers_credited(title, &credits);
    let rows = cand.iter().any(|q| !file.contains(q));
    let tag = file.iter().any(|q| !cand.contains(q));
    match (rows, tag) {
        (true, true) => "both",
        (true, false) => "rows",
        _ => "tag",
    }
}

/// Whether every one of these options is one song: the same title, version and artist.
fn one_song(group: &[&Opt]) -> bool {
    let Some(first) = group.first() else {
        return false;
    };
    group.iter().all(|o| {
        match (
            o.title.as_deref(),
            o.artist.as_deref(),
            first.title.as_deref(),
            first.artist.as_deref(),
        ) {
            (Some(t), Some(a), Some(t0), Some(a0)) => {
                plain_title(t) == plain_title(t0)
                    && fold(a) == fold(a0)
                    && same_version(t0, None, t, None, &[], &[a0, a])
            }
            _ => false,
        }
    })
}

/// The option that agrees with the file's tags with the best score, when one does.
fn best_agreeing<'a>(tags: &FileTags, options: &'a [Opt]) -> Option<&'a Opt> {
    options
        .iter()
        .filter(|o| agrees(tags, o))
        .max_by(|a, b| a.score.total_cmp(&b.score))
}

/// Whether a recording's known length is more than 3 s off the file's: another take.
fn length_off(o: &Opt, tags: &FileTags) -> bool {
    matches!((o.duration_secs, tags.duration_ms), (Some(d), Some(ms)) if (d - ms as f64 / 1000.0).abs() > crate::identify::DURATION_SLACK_SECS)
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
            stored: true,
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
        // Search options (below an exact search's 0.9, R5): one on the folder's release
        let s = vec![
            on(src(fp(5, "Song", "Band", 0.88), "search"), 1, "Album"),
            on(src(fp(6, "Song", "Band", 0.86), "search"), 3, "Best Of"),
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
        // Review r2 (the node's replay): a sure recording of a known length far off is another
        // take AcoustID links to this fingerprint, not this audio: it neither decides nor is
        // picked (here, the lower id, a live take with a plain title)
        let mut longer = fp(0, "Song", "Band", 0.95);
        longer.duration_secs = Some(215.0);
        assert_eq!(
            picked(&run(&t, &[fp(1, "Song", "Band", 0.95), longer.clone()], None)).0,
            rec(1)
        );
        let mut longer_live = fp(0, "Song (Live)", "Band", 0.95);
        longer_live.duration_secs = Some(260.0);
        assert_eq!(
            picked(&run(&t, &[fp(1, "Song", "Band", 0.95), longer_live], None)).0,
            rec(1)
        );
        // ... alone, it leaves nothing to settle on
        assert_eq!(run(&t, &[longer], None), Verdict::Stay(Stay::DifferentSongs));
        // ... a live take of this length is a real question
        let mut live_here = fp(0, "Song (Live)", "Band", 0.95);
        live_here.duration_secs = Some(201.0);
        let (v, why) = decide_why(&Song {
            tags: &t,
            options: &[fp(1, "Song", "Band", 0.95), live_here],
            folder_release: None,
            rejected: false,
            answer: true,
        });
        assert_eq!(
            (v, why.as_deref()),
            (Verdict::Stay(Stay::Version), Some("title:live:rows:len_fit"))
        );
        let mut close = fp(2, "Song", "Band", 0.95);
        close.duration_secs = Some(201.5);
        assert!(matches!(
            run(&t, &[fp(1, "Song", "Band", 0.95), close], None),
            Verdict::Pick { .. }
        ));
        // Only on live releases (the kept answer's release kinds), for a studio file: never picked
        let mut on_live = fp(0, "Song", "Band", 0.95);
        on_live.live_only = Some(true);
        assert_eq!(
            picked(&run(&t, &[fp(1, "Song", "Band", 0.95), on_live.clone()], None)).0,
            rec(1)
        );
        let (v, why) = decide_why(&Song {
            tags: &t,
            options: &[on_live],
            folder_release: None,
            rejected: false,
            answer: true,
        });
        assert_eq!(
            (v, why.as_deref()),
            (Verdict::Stay(Stay::Version), Some("live_only"))
        );
        // The audio names another song, a search by the tags the file's: they disagree
        let mut searched = fp(3, "Song", "Band", 0.9);
        searched.source = "search".into();
        assert_eq!(
            run(&t, &[fp(1, "Other", "Band", 0.97), searched], None),
            Verdict::Stay(Stay::DifferentSongs)
        );
        // Another artist
        // The file's song by another artist only (a cover): stays; heard under the file's artist
        // too, the tags decide (R2)
        let cover = vec![fp(2, "Song", "Other Band", 0.95)];
        assert_eq!(run(&t, &cover, None), Verdict::Stay(Stay::Artist));
        let both = vec![fp(1, "Song", "Band", 0.95), fp(2, "Song", "Other Band", 0.95)];
        assert_eq!(picked(&run(&t, &both, None)), (rec(1), None, Rule::R2));
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
        // Weak only: a weaker fingerprint and a search, neither of a known length
        let mut weak = vec![
            fp(1, "Song", "Band", 0.85),
            src(fp(2, "Song", "Band", 1.0), "search"),
        ];
        weak.iter_mut().for_each(|o| o.duration_secs = None);
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
        // (far off in length: another take, set aside)
        assert_eq!(picked(&run(&t, &merged, None)).0, rec(1));
        let near: Vec<crate::identify::AcoustIdResult> = serde_json::from_value(serde_json::json!([
            {"id": "a", "score": 0.95, "recordings": [
                {"id": rec(1), "title": "Song", "duration": 200.0, "artists": [{"name": "Band"}]},
                {"id": rec(7), "title": "Song (Live)", "duration": 202.0, "artists": [{"name": "Band"}]}
            ]}
        ]))
        .unwrap();
        let merged = with_answer(opts.clone(), &near, &none);
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

    /// Review r2: artists compared as people do; other artists, and "Various Artists", stay.
    #[test]
    fn artists_compare_by_spelling_and_credit_lists() {
        let agrees = |file: &str, credit: &str| artist_match(file, credit).agrees();
        assert!(agrees("Beyonce", "Beyoncé"));
        assert!(agrees("Beatles", "The Beatles"));
        assert!(agrees("Simon & Garfunkel", "Simon and Garfunkel"));
        assert!(agrees("Queen", "Queen & David Bowie"));
        assert!(agrees("Daft Punk feat. Pharrell Williams", "Daft Punk"));
        assert!(agrees("Mumford & Sons", "Mumford and Sons"));
        assert!(!agrees("Queen", "Bowie"));
        assert!(!agrees("Band", "Other Band"));
        assert_eq!(artist_match("Various Artists", "Queen"), ArtistMatch::Various);
        let t = tags("Song", "Beyonce", "Album");
        assert_eq!(
            picked(&run(&t, &[fp(1, "Song", "Beyoncé", 0.95)], None)).0,
            rec(1)
        );
        let (v, why) = decide_why(&Song {
            tags: &tags("Song", "Various Artists", "Album"),
            options: &[fp(1, "Song", "Queen", 0.95)],
            folder_release: None,
            rejected: false,
            answer: true,
        });
        assert_eq!(
            (v, why.as_deref()),
            (Verdict::Stay(Stay::Artist), Some("various:unanimous"))
        );
    }

    /// The words a reason to stay is told in come from a closed list.
    #[test]
    fn reasons_to_stay_use_listed_words_only() {
        let t = tags("Song", "Band", "Album");
        let cases: Vec<Vec<Opt>> = vec![
            vec![
                fp(1, "Song", "Band", 0.95),
                fp(2, "Song (Taylor's Version)", "Band", 0.95),
            ],
            vec![
                fp(1, "Song", "Band", 0.95),
                fp(2, "Song (Live at Wembley 1986)", "Band", 0.95),
            ],
            vec![fp(1, "This", "Other", 0.95)],
            vec![src(fp(1, "Song", "Band", 0.6), "search")],
            vec![
                fp(1, "Song", "Band", 0.95),
                fp(2, "Song (Version 2)", "Band", 0.95),
            ],
            vec![
                fp(1, "Song", "Band", 0.95),
                fp(2, "Song (Something Rare)", "Band", 0.95),
            ],
            vec![fp(1, "Song", "Other", 0.95)],
        ];
        for opts in cases {
            let (_, why) = decide_why(&Song {
                tags: &t,
                options: &opts,
                folder_release: None,
                rejected: false,
                answer: true,
            });
            let why = why.unwrap();
            for w in why.split(':') {
                assert!(WHY_WORDS.contains(&w) || MARK_WORDS.contains(&w), "{w}");
            }
        }
    }

    /// Review r3 (the node's replay): the classes that settle now, and those that still stay.
    #[test]
    fn review_r3_classes() {
        let t = tags("Song", "Band", "Album");
        let search = |i: u32, score: f32| src(fp(i, "Song", "Band", score), "search");
        // (a search of no known length: an exact search with one settles on its own, R5)
        let unsized_search = |i: u32, score: f32| {
            let mut o = search(i, score);
            o.duration_secs = None;
            o
        };
        let why = |t: &FileTags, opts: &[Opt]| {
            decide_why(&Song {
                tags: t,
                options: opts,
                folder_release: None,
                rejected: false,
                answer: true,
            })
        };
        // R4: the tags' search and a weaker fingerprint of the song, its length fitting
        let heard = fp(2, "Song", "Band", 0.6);
        let (v, _) = why(&t, &[unsized_search(1, 0.95), heard.clone()]);
        assert_eq!(picked(&v), (rec(2), None, Rule::R4));
        assert!(
            matches!(v, Verdict::Pick { score, .. } if score >= 0.9),
            "publishable: {v:?}"
        );
        // ... not without the audio's agreement, nor with a length unknown, nor with a sure
        // recording of another song that may be this audio
        assert_eq!(why(&t, &[unsized_search(1, 0.95)]).0, Verdict::Stay(Stay::Weak));
        let mut unknown = heard.clone();
        unknown.duration_secs = None;
        assert_eq!(
            why(&t, &[unsized_search(1, 0.95), unknown]).0,
            Verdict::Stay(Stay::Weak)
        );
        let other = fp(3, "Other Song", "Band", 0.95);
        assert_eq!(
            why(&t, &[search(1, 0.95), heard.clone(), other]).0,
            Verdict::Stay(Stay::DifferentSongs)
        );
        // ... a weak fingerprint too weak
        assert_eq!(
            why(&t, &[unsized_search(1, 0.95), fp(2, "Song", "Band", 0.4)]).0,
            Verdict::Stay(Stay::Weak)
        );
        // The weak reason says what the agreeing option is
        assert_eq!(
            why(&t, &[search(1, 0.85)]).1.as_deref(),
            Some("tags_row:search:80to90:len_fit:no_strong")
        );
        // An album's disc, edition or country names no version: settles
        for album in [
            "Album (Disc 1)",
            "Album (Deluxe Edition)",
            "Album (Collector's Edition)",
            "Album [Japan]",
        ] {
            let t = tags("Song", "Band", album);
            assert!(
                matches!(why(&t, &[fp(1, "Song", "Band", 0.95)]).0, Verdict::Pick { .. }),
                "{album}"
            );
        }
        // A re-recording names one: stays, and says so
        let rr = tags("Song", "Band", "Album (Re-Recorded)");
        assert_eq!(
            why(&rr, &[fp(1, "Song", "Band", 0.95)]),
            (Verdict::Stay(Stay::AlbumVersion), Some("album:version".into()))
        );
        // A live album: its live recordings settle, a studio one alone doesn't
        let live = tags("Song", "Band", "Live at Wembley");
        let mut on_stage = fp(4, "Song (Live)", "Band", 0.95);
        on_stage.live_only = Some(true);
        assert_eq!(picked(&why(&live, &[on_stage.clone()]).0).0, rec(4));
        let mut plain_live = fp(5, "Song", "Band", 0.95);
        plain_live.live_only = Some(true);
        assert_eq!(picked(&why(&live, &[plain_live]).0).0, rec(5));
        assert_eq!(
            why(&live, &[fp(1, "Song", "Band", 0.95)]),
            (Verdict::Stay(Stay::AlbumVersion), Some("album:live".into()))
        );
        // "Main Version" is the song's plain version
        assert_eq!(
            picked(&why(&t, &[fp(6, "Song (Main Version)", "Band", 0.95)]).0).0,
            rec(6)
        );
        // Another artist: whether the audio is unanimous is said
        assert_eq!(
            why(&t, &[fp(1, "Song", "Other Band", 0.95)]).1.as_deref(),
            Some("different:unanimous")
        );
    }

    /// Review r4 (the node's replay): exact tag searches settle (R5), a stored option is picked
    /// before one only the kept answer names, and the details say more.
    #[test]
    fn review_r4_classes() {
        let t = tags("Song", "Band", "Album");
        let search = |i: u32, score: f32| src(fp(i, "Song", "Band", score), "search");
        let why = |opts: &[Opt]| {
            decide_why(&Song {
                tags: &t,
                options: opts,
                folder_release: None,
                rejected: false,
                answer: true,
            })
        };
        // R5: exact searches of a known length that fits, nothing sure of another song
        assert_eq!(
            picked(&why(&[search(3, 0.95), search(2, 0.92)]).0),
            (rec(2), None, Rule::R5)
        );
        // ... a weak fingerprint of another song doesn't stand in the way
        assert_eq!(
            picked(&why(&[search(3, 0.95), fp(9, "Other", "Band", 0.6)]).0).0,
            rec(3)
        );
        // ... a sure recording of another song does (the audio says otherwise)
        assert_eq!(
            why(&[search(3, 0.95), fp(9, "Other", "Band", 0.95)]).0,
            Verdict::Stay(Stay::DifferentSongs)
        );
        // ... and a weaker search, or one whose length isn't known, settles nothing
        assert_eq!(why(&[search(3, 0.85)]).0, Verdict::Stay(Stay::Weak));
        let mut unknown = search(3, 0.95);
        unknown.duration_secs = None;
        assert_eq!(why(&[unknown]).0, Verdict::Stay(Stay::Weak));
        // A stored option before one only the kept answer names (lower id or not)
        let mut only_answer = fp(0, "Song", "Band", 0.95);
        only_answer.stored = false;
        assert_eq!(
            picked(&why(&[only_answer.clone(), fp(1, "Song", "Band", 0.95)]).0).0,
            rec(1)
        );
        // ... the answer's when it is the only one that fits
        let mut no_len = fp(1, "Song", "Band", 0.95);
        no_len.duration_secs = None;
        assert_eq!(picked(&why(&[only_answer, no_len]).0).0, rec(0));
        // How far off a tag-agreeing sure recording is
        let mut off = fp(1, "Song", "Band", 0.95);
        off.duration_secs = Some(207.0);
        assert_eq!(
            why(&[off]).1.as_deref(),
            Some("tags_row:fingerprint:ge90:len_off:5to10s:relgt2:no_strong")
        );
        let long = FileTags {
            duration_ms: Some(600_000),
            ..tags("Song", "Band", "Album")
        };
        let mut near_long = fp(1, "Song", "Band", 0.95);
        near_long.duration_secs = Some(605.0);
        let (_, d) = decide_why(&Song {
            tags: &long,
            options: &[near_long],
            folder_release: None,
            rejected: false,
            answer: true,
        });
        assert_eq!(
            d.as_deref(),
            Some("tags_row:fingerprint:ge90:len_off:3to5s:rel2:no_strong")
        );
        // Which side names the version
        assert_eq!(
            why(&[
                fp(1, "Song", "Band", 0.95),
                fp(2, "Song (Club Mix)", "Band", 0.95)
            ])
            .1
            .as_deref(),
            Some("title:mix:rows:len_fit")
        );
        let tagged = tags("Song (Club Mix)", "Band", "Album");
        let (_, d) = decide_why(&Song {
            tags: &tagged,
            options: &[fp(1, "Song", "Band", 0.95)],
            folder_release: None,
            rejected: false,
            answer: true,
        });
        assert_eq!(d.as_deref(), Some("title:mix:tag:len_fit"));
    }

    #[test]
    fn the_folders_release_is_the_most_common_one() {
        assert_eq!(folder_release(["a", "a", "b"]).as_deref(), Some("a"));
        assert_eq!(folder_release(["a", "b"]), None);
        assert_eq!(folder_release(Vec::<&str>::new()), None);
    }
}
