//! Songs added to the phone: picked in the page (the system's file picker), sent to this backend
//! with the node's own upload routes (`/v1/library/uploads`, resumable 4 MiB parts) and kept in the
//! app's own storage (`phone/<pubky>/music/`), named as a node names song files on the homeserver
//! (`<BLAKE3 16 hex>-<name>`, `pm_appdata::files`). Tags are read with pm-library's reader; the
//! songs are listed as the offline page lists what a device keeps (`/v1/offline`), and play from
//! `/v1/library/tracks/{id}/file`.

use super::{err, Phone};
use axum::body::Bytes;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path as FsPath, PathBuf};
use std::sync::{Arc, Mutex};

/// One part of an upload (the page's chunk size).
pub const CHUNK: u64 = 4 * 1024 * 1024;
/// The largest song taken.
const MAX_SIZE: u64 = 2 * 1024 * 1024 * 1024;
/// What an upload always leaves free on the phone.
const KEEP_FREE: u64 = 200 * 1024 * 1024;

/// One song on the phone.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Song {
    pub id: i64,
    /// Its file in `music/` (`<hash>-<name>`).
    pub file: String,
    pub title: String,
    #[serde(default)]
    pub artists: Vec<String>,
    #[serde(default)]
    pub album: Option<String>,
    #[serde(default)]
    pub album_artist: Option<String>,
    #[serde(default)]
    pub year: Option<i32>,
    #[serde(default)]
    pub track_no: Option<u32>,
    #[serde(default)]
    pub disc_no: Option<u32>,
    #[serde(default)]
    pub duration_ms: Option<u64>,
    pub container: String,
    pub format: pm_model::AudioFormat,
    pub bytes: u64,
    /// Its album's cover in `music/covers/`, when the file had one.
    #[serde(default)]
    pub cover: Option<String>,
    /// The song's canonical URL from its tags (recording MBID, else ISRC), as the node assigns one:
    /// what a bookmark or a listen of it points at.
    #[serde(default)]
    pub canonical: Option<String>,
}

/// A stable id from text (albums and artists), positive and within JavaScript's integers.
fn id_of(text: &str) -> i64 {
    let h = blake3::hash(text.to_lowercase().as_bytes());
    (u64::from_le_bytes(h.as_bytes()[..8].try_into().expect("8 bytes")) >> 12) as i64
}

impl Song {
    pub fn album_id(&self) -> Option<i64> {
        let album = self.album.as_ref()?;
        let by = self
            .album_artist
            .clone()
            .or_else(|| self.artists.first().cloned())
            .unwrap_or_default();
        Some(id_of(&format!("{by}\u{0}{album}")))
    }

    /// The song as the node's library answers it (what the page's rows read).
    pub fn track(&self) -> Value {
        json!({
            "id": self.id,
            "title": self.title,
            "artists": self.artists.iter().map(|a| json!({"id": id_of(a), "name": a})).collect::<Vec<_>>(),
            "album": self.album.as_ref().map(|t| json!({"id": self.album_id(), "title": t, "year": self.year})),
            "duration_ms": self.duration_ms,
            "track_no": self.track_no,
            "disc_no": self.disc_no,
            "file": {"container": self.container, "format": self.format},
            // What a Bookmark of it points at (the page's heart reads it)
            "canonical": self.canonical.as_ref().map(|u| json!({ "url": u })),
        })
    }
}

/// An upload on its way.
#[derive(Debug, Clone, Serialize)]
pub struct Upload {
    pub id: String,
    pub name: String,
    pub size: u64,
    pub offset: u64,
    /// `receiving`, `storing`, `stored`, `failed`.
    pub state: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(skip)]
    part: PathBuf,
    /// A part is being written now: another one waits for it (409).
    #[serde(skip)]
    writing: bool,
}

/// The phone's songs and its uploads.
pub struct Songs {
    dir: PathBuf,
    index: Mutex<Vec<Song>>,
    uploads: Mutex<HashMap<String, Upload>>,
    /// One song added at a time (two copies of one file never meet at its path).
    adding: Mutex<()>,
    /// Why the list on the phone can't be read, when it can't: then it is left as it is, no song
    /// is listed and none can be added (saved, the new list would replace the old one).
    unreadable: Option<String>,
}

impl Songs {
    pub fn open(dir: &FsPath) -> std::io::Result<Self> {
        let dir = dir.join("music");
        std::fs::create_dir_all(dir.join("covers"))?;
        // Parts of uploads cut off by a restart start again
        for e in std::fs::read_dir(&dir)?.flatten() {
            if e.path().extension().is_some_and(|x| x == "part") {
                let _ = std::fs::remove_file(e.path());
            }
        }
        // No list yet: none. One that can't be read stays as it is (phone mode still starts)
        let list = dir.join("songs.json");
        let (index, unreadable) = match std::fs::read(&list) {
            Ok(b) => match serde_json::from_slice(&b) {
                Ok(v) => (v, None),
                Err(e) => (Vec::new(), Some(e.to_string())),
            },
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => (Vec::new(), None),
            Err(e) => (Vec::new(), Some(e.to_string())),
        };
        let unreadable = unreadable.map(|e| {
            tracing::error!(
                "phone mode: the list of songs ({}) can't be read ({e}); left as it is",
                list.display()
            );
            format!("this phone's list of songs can't be read ({e}), so no song can be added now")
        });
        Ok(Self {
            dir,
            index: Mutex::new(index),
            uploads: Mutex::default(),
            adding: Mutex::default(),
            unreadable,
        })
    }

    pub fn list(&self) -> Vec<Song> {
        self.index.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    pub fn get(&self, id: i64) -> Option<Song> {
        self.list().into_iter().find(|s| s.id == id)
    }

    pub fn path_of(&self, s: &Song) -> PathBuf {
        self.dir.join(&s.file)
    }

    fn save(&self, list: &[Song]) -> std::io::Result<()> {
        use std::io::Write;
        let p = self.dir.join("songs.json");
        let tmp = p.with_extension("json.tmp");
        let b = serde_json::to_vec(list).map_err(std::io::Error::other)?;
        // On the disk before it replaces the list (a power cut never leaves it empty)
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(&b)?;
        f.sync_all()?;
        std::fs::rename(&tmp, &p).inspect_err(|_| {
            let _ = std::fs::remove_file(&tmp);
        })
    }

    /// A whole uploaded file: named by its hash, its tags read, listed. The same file twice is
    /// one song.
    pub fn store(&self, part: &FsPath, name: &str) -> Result<Song, String> {
        let _one = self.adding.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(why) = &self.unreadable {
            let _ = std::fs::remove_file(part);
            return Err(why.clone());
        }
        // Hashed as it is read (a song may be large)
        let mut hasher = blake3::Hasher::new();
        let mut f = std::fs::File::open(part).map_err(|e| e.to_string())?;
        std::io::copy(&mut f, &mut hasher).map_err(|e| e.to_string())?;
        let hash = hasher.finalize().to_hex().to_string();
        let safe = pm_appdata::files::safe_file_name(FsPath::new(name));
        let file = pm_appdata::files::remote_name_for(&hash, &safe);
        if let Some(s) = self.list().into_iter().find(|s| s.file == file) {
            let _ = std::fs::remove_file(part);
            return Ok(s);
        }
        let path = self.dir.join(&file);
        std::fs::rename(part, &path).map_err(|e| e.to_string())?;
        let (probe, tags) = match pm_library::tags::probe_and_read(&path) {
            Ok(x) => x,
            Err(e) => {
                let _ = std::fs::remove_file(&path);
                return Err(format!("not a song this phone can play: {e}"));
            }
        };
        let mut index = self.index.lock().unwrap_or_else(|e| e.into_inner());
        let id = index.iter().map(|s| s.id).max().unwrap_or(0) + 1;
        let stem = FsPath::new(name)
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "Song".into());
        let mut song = Song {
            id,
            file,
            title: tags
                .title
                .clone()
                .filter(|t| !t.trim().is_empty())
                .unwrap_or(stem),
            artists: tags.artists.clone(),
            album: tags.album.clone(),
            album_artist: tags.album_artist.clone(),
            year: tags.year,
            track_no: tags.track_no,
            disc_no: tags.disc_no,
            duration_ms: probe.duration_ms,
            container: probe.container.clone(),
            format: probe.format,
            bytes: std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0),
            cover: None,
            canonical: pm_library::canonical::assign(&tags).map(|(u, _)| u.to_url()),
        };
        // A cover written for this song alone goes again if the song can't be listed
        let mut new_cover = None;
        if let (Some(pic), Some(aid)) = (&tags.picture, song.album_id()) {
            let cover = format!("{aid}.{}", pic.extension());
            let at = self.dir.join("covers").join(&cover);
            let there = at.exists();
            if std::fs::write(&at, &pic.data).is_ok() {
                song.cover = Some(cover);
                new_cover = (!there).then_some(at);
            }
        }
        // Listed only once the list is saved: a song missing after a restart is never answered
        // as added
        let mut new = index.clone();
        new.push(song.clone());
        if let Err(e) = self.save(&new) {
            tracing::warn!("phone mode: the songs list could not be saved: {e}");
            let _ = std::fs::remove_file(&path);
            if let Some(c) = new_cover {
                let _ = std::fs::remove_file(c);
            }
            return Err(format!(
                "this phone couldn't save its list of songs ({e}): add it again"
            ));
        }
        *index = new;
        Ok(song)
    }

    /// The cover of an album, when one of its songs had one.
    pub fn cover(&self, album: i64) -> Option<PathBuf> {
        self.list()
            .into_iter()
            .filter(|s| s.album_id() == Some(album))
            .find_map(|s| s.cover)
            .map(|c| self.dir.join("covers").join(c))
            .filter(|p| p.is_file())
    }

    /// Songs whose title, artist or album has every word of `q`.
    pub fn find(&self, q: &str, limit: usize) -> Vec<Song> {
        let words: Vec<String> = q.split_whitespace().map(str::to_lowercase).collect();
        if words.is_empty() {
            return Vec::new();
        }
        self.list()
            .into_iter()
            .filter(|s| {
                let text = format!(
                    "{} {} {}",
                    s.title,
                    s.artists.join(" "),
                    s.album.clone().unwrap_or_default()
                )
                .to_lowercase();
                words.iter().all(|w| text.contains(w))
            })
            .take(limit)
            .collect()
    }
}

#[derive(Deserialize)]
pub(crate) struct NewUpload {
    name: String,
    size: u64,
}

/// `POST /v1/library/uploads {name, size}`: an upload starts (the node's shape).
pub(crate) async fn start(State(p): State<Arc<Phone>>, Json(b): Json<NewUpload>) -> Response {
    let name = pm_appdata::files::safe_file_name(FsPath::new(&b.name));
    if name.is_empty() || !pm_library::tags::is_supported(FsPath::new(&name)) {
        return err(
            StatusCode::BAD_REQUEST,
            &format!("{}: not a kind of music file this phone plays", b.name),
        );
    }
    if b.size == 0 || b.size > MAX_SIZE {
        return err(StatusCode::BAD_REQUEST, "that file's size can't be a song");
    }
    // The list can't be read: nothing is received for it
    if let Some(why) = &p.songs.unreadable {
        return err(StatusCode::INTERNAL_SERVER_ERROR, why);
    }
    if crate::sync::free_bytes(&p.songs.dir).is_some_and(|free| free < b.size + KEEP_FREE) {
        return err(
            StatusCode::INSUFFICIENT_STORAGE,
            "Not enough space on this phone for that song",
        );
    }
    let mut raw = [0u8; 12];
    let _ = getrandom::fill(&mut raw);
    let id: String = raw.iter().map(|x| format!("{x:02x}")).collect();
    let part = p.songs.dir.join(format!("{id}.part"));
    if let Err(e) = std::fs::File::create(&part) {
        return err(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string());
    }
    p.songs.uploads.lock().unwrap_or_else(|e| e.into_inner()).insert(
        id.clone(),
        Upload {
            id: id.clone(),
            name: b.name,
            size: b.size,
            offset: 0,
            state: "receiving".into(),
            error: None,
            part,
            writing: false,
        },
    );
    Json(json!({ "id": id, "offset": 0, "chunk_size": CHUNK, "already": false })).into_response()
}

/// `GET /v1/library/uploads/{id}`.
pub(crate) async fn get(State(p): State<Arc<Phone>>, Path(id): Path<String>) -> Response {
    match p.songs.uploads.lock().unwrap_or_else(|e| e.into_inner()).get(&id) {
        Some(u) => Json(u.clone()).into_response(),
        None => err(StatusCode::NOT_FOUND, "no such upload"),
    }
}

#[derive(Deserialize)]
pub(crate) struct At {
    offset: u64,
}

/// Takes the next part of `u` for writing, in the same step as the checks (the caller holds the
/// uploads' lock): a part at another offset, or while one is being written, is refused (409 with
/// where the upload is), as is one that doesn't fit the file. `None`: taken.
fn claim(u: &mut Upload, offset: u64, len: u64) -> Option<Response> {
    if u.state != "receiving" || u.writing || offset != u.offset {
        return Some(
            (
                StatusCode::CONFLICT,
                Json(json!({"offset": u.offset, "state": u.state})),
            )
                .into_response(),
        );
    }
    if len > CHUNK || u.offset + len > u.size {
        return Some(err(StatusCode::BAD_REQUEST, "that part doesn't fit the file"));
    }
    u.writing = true;
    None
}

/// `PUT /v1/library/uploads/{id}?offset=`: the next part; the last one stores the song.
pub(crate) async fn put_part(
    State(p): State<Arc<Phone>>,
    Path(id): Path<String>,
    Query(at): Query<At>,
    body: Bytes,
) -> Response {
    let (part, name, size, offset) = {
        let mut ups = p.songs.uploads.lock().unwrap_or_else(|e| e.into_inner());
        let Some(u) = ups.get_mut(&id) else {
            return err(StatusCode::NOT_FOUND, "no such upload");
        };
        if let Some(refused) = claim(u, at.offset, body.len() as u64) {
            return refused;
        }
        (u.part.clone(), u.name.clone(), u.size, u.offset)
    };
    // The write and what follows run on their own task: a request dropped meanwhile (a reload)
    // leaves the upload as the write left it, never claimed for good
    match tokio::spawn(write_part(p, id, part, name, size, offset, body)).await {
        Ok(r) => r,
        Err(_) => err(StatusCode::INTERNAL_SERVER_ERROR, "the part stopped"),
    }
}

/// Writes a claimed part at `offset` and moves the upload on (the last part stores the song).
async fn write_part(
    p: Arc<Phone>,
    id: String,
    part: PathBuf,
    name: String,
    size: u64,
    offset: u64,
    body: Bytes,
) -> Response {
    // From the acknowledged offset: what a failed write left after it is cut off first
    // (off the runtime that also streams the music)
    let at = part.clone();
    let len = body.len() as u64;
    let appended = tokio::task::spawn_blocking(move || {
        use std::io::{Seek, Write};
        std::fs::OpenOptions::new()
            .write(true)
            .open(&at)
            .and_then(|mut f| {
                f.set_len(offset)?;
                f.seek(std::io::SeekFrom::Start(offset))?;
                f.write_all(&body)?;
                f.sync_data()
            })
    })
    .await
    .unwrap_or_else(|e| Err(std::io::Error::other(e.to_string())));
    if let Err(e) = appended {
        if let Some(u) = p
            .songs
            .uploads
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get_mut(&id)
        {
            u.writing = false;
        }
        return err(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string());
    }
    let now = offset + len;
    let done = now == size;
    {
        let mut ups = p.songs.uploads.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(u) = ups.get_mut(&id) {
            u.offset = now;
            u.writing = false;
            if done {
                u.state = "storing".into();
            }
        }
    }
    if !done {
        return Json(json!({"offset": now, "state": "receiving"})).into_response();
    }
    let p2 = p.clone();
    let stored = tokio::task::spawn_blocking(move || {
        // Whole: exactly the size the upload said
        match std::fs::metadata(&part).map(|m| m.len()) {
            Ok(n) if n == size => p2.songs.store(&part, &name),
            _ => {
                let _ = std::fs::remove_file(&part);
                Err("the file arrived incomplete: add it again".into())
            }
        }
    })
    .await
    .unwrap_or_else(|_| Err("storing stopped".into()));
    let mut ups = p.songs.uploads.lock().unwrap_or_else(|e| e.into_inner());
    let u = ups.get_mut(&id).expect("this upload");
    match stored {
        Ok(s) => {
            tracing::info!(song = s.id, "phone mode: a song added");
            u.state = "stored".into();
            // To the homeserver too, when that is on
            super::files::copy_soon(&p);
        }
        Err(e) => {
            u.state = "failed".into();
            u.error = Some(e);
        }
    }
    Json(json!({"offset": now, "state": u.state, "error": u.error})).into_response()
}

/// A song's place on its album: disc, number, id.
type SongAt = (u32, u32, i64);

/// `GET /v1/offline`: the phone's songs as the offline page lists what a device keeps. A list of
/// songs or of Bookmarks that can't be read is an error (the page says why), never "none".
pub(crate) async fn listing(State(p): State<Arc<Phone>>) -> Response {
    if let Some(why) = &p.songs.unreadable {
        return err(StatusCode::INTERNAL_SERVER_ERROR, why);
    }
    let bookmarks = match super::data::load(&p) {
        Ok(l) => l.bookmarks,
        Err(e) => return err(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
    };
    let songs = p.songs.list();
    let mut albums: BTreeMap<i64, (Value, Vec<SongAt>)> = BTreeMap::new();
    for s in &songs {
        if let Some(aid) = s.album_id() {
            let a = albums.entry(aid).or_insert_with(|| {
                (
                    json!({"id": aid, "title": s.album, "year": s.year,
                           "artist": s.album_artist.clone().or_else(|| s.artists.first().cloned())}),
                    Vec::new(),
                )
            });
            a.1.push((s.disc_no.unwrap_or(1), s.track_no.unwrap_or(0), s.id));
        }
    }
    let covered: BTreeSet<i64> = songs
        .iter()
        .filter(|s| s.cover.is_some())
        .filter_map(Song::album_id)
        .collect();
    let mut albums: Vec<Value> = albums
        .into_iter()
        .map(|(aid, (mut a, mut ids))| {
            ids.sort();
            a["track_ids"] = json!(ids.iter().map(|x| x.2).collect::<Vec<_>>());
            a["cover"] = json!(covered.contains(&aid));
            a
        })
        .collect();
    albums.sort_by_key(|a| a["title"].as_str().unwrap_or_default().to_lowercase());
    let bytes: u64 = songs.iter().map(|s| s.bytes).sum();
    let tracks: Vec<Value> = songs
        .iter()
        .map(|s| {
            let mut t = s.track();
            t["offline"] = json!({"bytes": s.bytes, "format": "original"});
            t
        })
        .collect();
    Json(json!({
        "offline": false,
        "device_id": Value::Null,
        "synced_at": Value::Null,
        "bytes": bytes,
        "queued": 0,
        "tracks": tracks,
        "albums": albums,
        "playlists": [],
        "favourites": {"track_ids": favourite_ids(&songs, &bookmarks)},
    }))
    .into_response()
}

/// `GET /v1/library/tracks/{id}`.
pub(crate) async fn track(State(p): State<Arc<Phone>>, Path(id): Path<i64>) -> Response {
    match p.songs.get(id) {
        Some(s) => Json(s.track()).into_response(),
        None => err(StatusCode::NOT_FOUND, "not on this phone"),
    }
}

/// `GET /v1/library/tracks/{id}/file`: the song, with ranges.
pub(crate) async fn file(State(p): State<Arc<Phone>>, Path(id): Path<i64>, headers: HeaderMap) -> Response {
    match p.songs.get(id) {
        Some(s) => crate::server::send_file(&p.songs.path_of(&s), &headers).await,
        None => err(StatusCode::NOT_FOUND, "not on this phone"),
    }
}

/// `GET /v1/library/albums/{id}/cover`.
pub(crate) async fn cover(State(p): State<Arc<Phone>>, Path(id): Path<i64>, headers: HeaderMap) -> Response {
    match p.songs.cover(id) {
        Some(c) => crate::server::send_file(&c, &headers).await,
        None => err(StatusCode::NOT_FOUND, "no cover on this phone"),
    }
}

/// The ids of the songs whose canonical URL is bookmarked.
fn favourite_ids(songs: &[Song], bookmarks: &std::collections::BTreeMap<String, i64>) -> Vec<i64> {
    songs
        .iter()
        .filter(|s| s.canonical.as_ref().is_some_and(|c| bookmarks.contains_key(c)))
        .map(|s| s.id)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[tokio::test]
    async fn a_dropped_request_never_leaves_its_part_claimed() {
        let dir = tempfile::tempdir().unwrap();
        let p = Phone::open(super::super::Config {
            dir: dir.path().to_path_buf(),
            cache: dir.path().join("streams"),
            version: "test".into(),
            pubky: None,
            tidal: Default::default(),
        })
        .unwrap();
        let r = start(
            State(p.clone()),
            Json(NewUpload {
                name: "a.flac".into(),
                size: 8,
            }),
        )
        .await;
        let v: Value =
            serde_json::from_slice(&axum::body::to_bytes(r.into_body(), usize::MAX).await.unwrap()).unwrap();
        let id = v["id"].as_str().unwrap().to_string();
        // The page reloads while the first part is written: its request is dropped
        let put = |offset: u64, b: &'static [u8]| {
            put_part(
                State(p.clone()),
                Path(id.clone()),
                Query(At { offset }),
                Bytes::from_static(b),
            )
        };
        let _ = tokio::time::timeout(Duration::ZERO, put(0, b"fLaC")).await;
        // The write still ends, and the upload goes on from there
        let mut at = None;
        for _ in 0..100 {
            let u = p.songs.uploads.lock().unwrap().get(&id).cloned().unwrap();
            if !u.writing {
                at = Some(u.offset);
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert_eq!(at, Some(4), "the part was written and the upload released");
        let r = put(4, b"data").await;
        assert_eq!(r.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn a_list_that_cant_be_read_is_never_shown_as_none() {
        let dir = tempfile::tempdir().unwrap();
        let open = || {
            Phone::open(super::super::Config {
                dir: dir.path().to_path_buf(),
                cache: dir.path().join("streams"),
                version: "test".into(),
                pubky: None,
                tidal: Default::default(),
            })
            .unwrap()
        };
        let p = open();
        let part = dir.path().join("x.part");
        std::fs::copy(
            format!(
                "{}/../pm-audio/tests/fixtures/60_mono_audio.flac",
                env!("CARGO_MANIFEST_DIR")
            ),
            &part,
        )
        .unwrap();
        p.songs.store(&part, "60_mono_audio.flac").unwrap();
        assert_eq!(listing(State(p.clone())).await.status(), StatusCode::OK);
        // The Bookmarks can't be read: an error, not "no favourites"
        let data = dir.path().join("data.json");
        std::fs::write(&data, b"{\"bookmarks\": {").unwrap();
        let r = listing(State(p.clone())).await;
        assert_eq!(r.status(), StatusCode::INTERNAL_SERVER_ERROR);
        std::fs::remove_file(&data).unwrap();
        // The list of songs can't be read (cut short): an error that says so, not "no songs"
        drop(p);
        let list = dir.path().join("music").join("songs.json");
        let whole = std::fs::read(&list).unwrap();
        std::fs::write(&list, &whole[..whole.len() - 5]).unwrap();
        let p = open();
        let r = listing(State(p.clone())).await;
        assert_eq!(r.status(), StatusCode::INTERNAL_SERVER_ERROR);
        let body = axum::body::to_bytes(r.into_body(), 1 << 20).await.unwrap();
        assert!(String::from_utf8_lossy(&body).contains("list of songs"));
    }

    #[test]
    fn a_list_of_songs_that_cant_be_read_stays_as_it_is() {
        let dir = tempfile::tempdir().unwrap();
        let part = |f: &str| {
            let part = dir.path().join("x.part");
            std::fs::copy(
                format!("{}/../pm-audio/tests/fixtures/{f}", env!("CARGO_MANIFEST_DIR")),
                &part,
            )
            .unwrap();
            part
        };
        let songs = Songs::open(dir.path()).unwrap();
        songs
            .store(&part("60_mono_audio.flac"), "60_mono_audio.flac")
            .unwrap();
        drop(songs);
        // The list is cut short (a power cut, a full disk)
        let list = dir.path().join("music").join("songs.json");
        let whole = std::fs::read(&list).unwrap();
        let cut = &whole[..whole.len() - 5];
        std::fs::write(&list, cut).unwrap();
        // Phone mode still starts; no song can be added (saved, the new list would be the only
        // one), and the list stays as it is
        let again = Songs::open(dir.path()).unwrap();
        assert!(again.list().is_empty());
        let e = again
            .store(&part("14_wasted_bits.flac"), "14_wasted_bits.flac")
            .unwrap_err();
        assert!(e.contains("list of songs"), "{e}");
        assert_eq!(std::fs::read(&list).unwrap(), cut);
    }

    #[tokio::test]
    async fn a_song_that_cant_be_listed_is_not_added() {
        let dir = tempfile::tempdir().unwrap();
        let p = Phone::open(super::super::Config {
            dir: dir.path().to_path_buf(),
            cache: dir.path().join("streams"),
            version: "test".into(),
            pubky: None,
            tidal: Default::default(),
        })
        .unwrap();
        // The list can't be saved now (a folder in its place)
        let music = dir.path().join("music");
        std::fs::create_dir(music.join("songs.json")).unwrap();
        let song = std::fs::read(format!(
            "{}/../pm-audio/tests/fixtures/60_mono_audio.flac",
            env!("CARGO_MANIFEST_DIR")
        ))
        .unwrap();
        let r = start(
            State(p.clone()),
            Json(NewUpload {
                name: "60_mono_audio.flac".into(),
                size: song.len() as u64,
            }),
        )
        .await;
        let v: Value =
            serde_json::from_slice(&axum::body::to_bytes(r.into_body(), usize::MAX).await.unwrap()).unwrap();
        let id = v["id"].as_str().unwrap().to_string();
        let r = put_part(
            State(p.clone()),
            Path(id.clone()),
            Query(At { offset: 0 }),
            Bytes::from(song),
        )
        .await;
        let v: Value =
            serde_json::from_slice(&axum::body::to_bytes(r.into_body(), usize::MAX).await.unwrap()).unwrap();
        assert_eq!(v["state"], "failed", "{v}");
        assert!(v["error"].as_str().unwrap().contains("list of songs"), "{v}");
        assert!(p.songs.list().is_empty());
        // Nothing left behind: no song file, no cover made for it
        let left: Vec<_> = std::fs::read_dir(&music)
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .filter(|n| n != "covers" && n != "songs.json")
            .collect();
        assert!(left.is_empty(), "{left:?}");
        assert_eq!(std::fs::read_dir(music.join("covers")).unwrap().count(), 0);
    }

    #[test]
    fn one_part_at_a_time() {
        let mut u = Upload {
            id: "u".into(),
            name: "a.flac".into(),
            size: 10,
            offset: 0,
            state: "receiving".into(),
            error: None,
            part: PathBuf::new(),
            writing: false,
        };
        assert!(claim(&mut u, 0, 4).is_none());
        // The same part again while the first is being written (a page that resumed): refused
        let again = claim(&mut u, 0, 4).unwrap();
        assert_eq!(again.status(), StatusCode::CONFLICT);
        // Written: the next part is taken
        u.writing = false;
        u.offset = 4;
        assert!(claim(&mut u, 4, 6).is_none());
        u.writing = false;
        // Too long for the file, or at another offset
        assert_eq!(claim(&mut u, 4, 7).unwrap().status(), StatusCode::BAD_REQUEST);
        assert_eq!(claim(&mut u, 3, 1).unwrap().status(), StatusCode::CONFLICT);
    }

    #[test]
    fn ids_are_stable_and_safe_for_the_page() {
        assert_eq!(id_of("Band"), id_of("band"));
        assert!(id_of("x") > 0 && id_of("x") < (1i64 << 53));
    }
}
