//! What the device keeps, for the app's storage screen and the marks on songs and albums: each
//! kept album, playlist and song with how many of its songs are here and their size, the total,
//! and which songs and albums are on the device.

use crate::store::{Index, KeepItem};
use serde::Serialize;
use serde_json::Value;
use std::collections::BTreeSet;

#[derive(Debug, Clone, Default, Serialize, PartialEq)]
pub struct Summary {
    /// Kept albums, playlists and songs, in that order.
    pub items: Vec<Item>,
    /// Songs on the device and their size.
    pub songs: usize,
    pub bytes: u64,
    /// The favourites are kept (the owner's devices).
    pub favourites: bool,
    /// Songs whose file is on the device (their rows get the mark).
    pub track_ids: Vec<i64>,
    /// Kept albums (their tiles get the mark).
    pub album_ids: Vec<i64>,
    pub playlist_ids: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, PartialEq)]
pub struct Item {
    /// `album`, `playlist` or `track`.
    pub kind: &'static str,
    pub id: String,
    pub title: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub sub: String,
    /// Its songs on the device, and their size.
    pub here: usize,
    pub bytes: u64,
    /// Its songs in all (an album's, as far as the device knows; a playlist's library songs).
    pub songs: usize,
    /// A playlist's songs that aren't in the library (a streaming service's): they stay online.
    pub online: usize,
}

fn album_of(t: &Value) -> Option<i64> {
    t.pointer("/album/id").and_then(Value::as_i64)
}

fn artist_of(t: &Value) -> String {
    t.pointer("/album/album_artist")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .or_else(|| t.pointer("/artists/0/name").and_then(Value::as_str))
        .unwrap_or_default()
        .to_string()
}

/// The storage screen's view of `ix`.
pub fn summary(ix: &Index) -> Summary {
    let size = |id: &i64| ix.tracks.get(id).map(|t| t.bytes);
    let label = |item: &KeepItem| ix.labels.get(&item.key()).cloned().unwrap_or_default();
    let mut items = Vec::new();
    for a in &ix.keep.albums {
        let songs: Vec<(&i64, &crate::store::KeptTrack)> = ix
            .tracks
            .iter()
            .filter(|(_, t)| album_of(&t.track) == Some(*a))
            .collect();
        let l = label(&KeepItem::Album(*a));
        let first = songs.first().map(|(_, t)| &t.track);
        // Its songs in all as the page counted them when it was kept; unknown: 0
        let total = l.songs.map_or(0, |n| n as usize);
        items.push(Item {
            kind: "album",
            id: a.to_string(),
            title: first
                .and_then(|t| t.pointer("/album/title").and_then(Value::as_str))
                .map(str::to_string)
                .filter(|t| !t.is_empty())
                .unwrap_or_else(|| {
                    if l.title.is_empty() {
                        "Album".into()
                    } else {
                        l.title.clone()
                    }
                }),
            sub: first.map(artist_of).filter(|s| !s.is_empty()).unwrap_or(l.sub),
            here: songs.len(),
            bytes: songs.iter().map(|(_, t)| t.bytes).sum(),
            songs: if total > 0 { total.max(songs.len()) } else { 0 },
            online: 0,
        });
    }
    for p in &ix.keep.playlists {
        let l = label(&KeepItem::Playlist(p.clone()));
        let node = ix.playlists.get(p);
        // The playlist's library songs (`tracks`, null where an entry isn't one) and its entries
        let lib: Vec<i64> = node
            .and_then(|v| v.get("tracks"))
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(|t| t.get("id").and_then(Value::as_i64))
                    .collect()
            })
            .unwrap_or_default();
        let entries = node
            .and_then(|v| v.get("entries").or_else(|| v.get("tracks")))
            .and_then(Value::as_array)
            .map_or(lib.len(), Vec::len);
        let here: Vec<u64> = lib.iter().filter_map(size).collect();
        items.push(Item {
            kind: "playlist",
            id: p.clone(),
            title: node
                .and_then(|v| v.pointer("/summary/name").and_then(Value::as_str))
                .map(str::to_string)
                .filter(|t| !t.is_empty())
                .unwrap_or_else(|| {
                    if l.title.is_empty() {
                        "Playlist".into()
                    } else {
                        l.title.clone()
                    }
                }),
            sub: String::new(),
            here: here.len(),
            bytes: here.iter().sum(),
            songs: lib.len(),
            online: entries.saturating_sub(lib.len()),
        });
    }
    for id in &ix.keep.tracks {
        let l = label(&KeepItem::Track(*id));
        let t = ix.tracks.get(id);
        items.push(Item {
            kind: "track",
            id: id.to_string(),
            title: t
                .and_then(|t| t.track.get("title").and_then(Value::as_str))
                .map(str::to_string)
                .unwrap_or_else(|| {
                    if l.title.is_empty() {
                        "Song".into()
                    } else {
                        l.title.clone()
                    }
                }),
            sub: t
                .map(|t| artist_of(&t.track))
                .filter(|s| !s.is_empty())
                .unwrap_or(l.sub),
            here: usize::from(t.is_some()),
            bytes: t.map_or(0, |t| t.bytes),
            songs: 1,
            online: 0,
        });
    }
    // Kept albums with songs here (one still waiting gets no mark yet)
    let album_ids: BTreeSet<i64> = ix
        .keep
        .albums
        .iter()
        .copied()
        .filter(|a| ix.tracks.values().any(|t| album_of(&t.track) == Some(*a)))
        .collect();
    Summary {
        items,
        songs: ix.tracks.len(),
        bytes: ix.bytes(),
        favourites: ix.keep.favourites,
        track_ids: ix.tracks.keys().copied().collect(),
        album_ids: album_ids.into_iter().collect(),
        playlist_ids: ix.keep.playlists.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::{Keep, KeptTrack, Label};
    use serde_json::json;

    fn kept(id: i64, album: i64, bytes: u64) -> KeptTrack {
        KeptTrack {
            track: json!({"id": id, "title": format!("Song {id}"), "artists": [{"name": "Sefa"}],
                          "album": {"id": album, "title": "Journey", "album_artist": "Sefa"}}),
            file: format!("{id}.flac"),
            bytes,
            format: "original".into(),
        }
    }

    #[test]
    fn each_kept_thing_with_its_songs_here_and_size() {
        let mut ix = Index {
            keep: Keep {
                tracks: vec![9, 4],
                albums: vec![7, 8],
                playlists: vec!["p1".into()],
                ..Keep::default()
            },
            ..Index::default()
        };
        for (id, album, bytes) in [(1, 7, 100), (2, 7, 200), (4, 5, 50)] {
            ix.tracks.insert(id, kept(id, album, bytes));
        }
        // A playlist of two library songs (one here) and a streaming entry
        ix.playlists.insert(
            "p1".into(),
            json!({"summary": {"name": "Mix"}, "tracks": [{"id": 2}, {"id": 3}, null], "entries": [{}, {}, {}]}),
        );
        ix.labels.insert(
            "album:7".into(),
            Label {
                title: "Journey".into(),
                sub: "Sefa".into(),
                songs: Some(12),
            },
        );
        // Kept before any of its songs arrived: the page's name for it
        ix.labels.insert(
            "album:8".into(),
            Label {
                title: "Later".into(),
                sub: "Dr. Peacock".into(),
                songs: None,
            },
        );
        let s = summary(&ix);
        let by = |k: &str, id: &str| {
            s.items
                .iter()
                .find(|i| i.kind == k && i.id == id)
                .cloned()
                .unwrap()
        };
        let a = by("album", "7");
        assert_eq!(
            (a.title.as_str(), a.sub.as_str(), a.here, a.bytes),
            ("Journey", "Sefa", 2, 300)
        );
        assert_eq!(a.songs, 12, "its songs in all, as the page counted them");
        let later = by("album", "8");
        assert_eq!(
            (later.title.as_str(), later.sub.as_str(), later.here),
            ("Later", "Dr. Peacock", 0)
        );
        let p = by("playlist", "p1");
        assert_eq!(
            (p.title.as_str(), p.here, p.bytes, p.songs, p.online),
            ("Mix", 1, 200, 2, 1)
        );
        let t = by("track", "4");
        assert_eq!((t.title.as_str(), t.here, t.bytes), ("Song 4", 1, 50));
        let gone = by("track", "9");
        assert_eq!((gone.title.as_str(), gone.here, gone.bytes), ("Song", 0, 0));
        assert_eq!(
            s.items.iter().map(|i| i.kind).collect::<Vec<_>>(),
            ["album", "album", "playlist", "track", "track"]
        );
        assert_eq!((s.songs, s.bytes), (3, 350));
        assert_eq!(s.track_ids, [1, 2, 4]);
        assert_eq!(s.album_ids, [7], "album 8 has no song here yet");
    }

    #[test]
    fn keeping_and_letting_go() {
        let mut k = Keep::default();
        assert!(k.set(&KeepItem::Album(7), true));
        assert!(!k.set(&KeepItem::Album(7), true), "kept once");
        assert!(k.set(&KeepItem::Playlist("p".into()), true));
        assert!(k.set(&KeepItem::Album(7), false));
        assert_eq!((k.albums.len(), k.playlists.len()), (0, 1));
        k.favourites = true;
        k.format = Some(crate::store::Format {
            format: "aac".into(),
            bitrate: None,
        });
        k.clear();
        assert!(
            k.playlists.is_empty() && !k.favourites && k.format.is_some(),
            "the format stays"
        );
        assert_eq!(KeepItem::parse("album", "7"), Some(KeepItem::Album(7)));
        assert_eq!(KeepItem::parse("track", "x"), None);
        assert_eq!(KeepItem::parse("tidal", "1"), None, "only the library's things");
    }
}
