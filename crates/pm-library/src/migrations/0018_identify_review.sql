-- "Not this": a recording the owner rejected for a song is never suggested again for it, nor
-- made its identity by a later run (a retry or an answer from the cache).
CREATE TABLE identify_rejections (
    track_id       INTEGER NOT NULL REFERENCES tracks(id) ON DELETE CASCADE,
    recording_mbid TEXT NOT NULL,
    rejected_at    INTEGER NOT NULL,
    PRIMARY KEY (track_id, recording_mbid)
);
-- MusicBrainz answers identification asked for (release searches and lookups, recording searches),
-- by service address, path and query: a later run asks again only for what it has not asked
-- within the cache's time (Library::IDENTIFY_MB_TTL_SECS), and the table stays bounded.
CREATE TABLE identify_mb_cache (
    key        TEXT PRIMARY KEY,
    body       TEXT,              -- NULL: MusicBrainz has nothing there (404)
    fetched_at INTEGER NOT NULL
);
CREATE INDEX identify_mb_cache_age ON identify_mb_cache(fetched_at);
-- A file's content key (Library::file_key) while its size and modification time stay the same:
-- a later run reads nothing of an unchanged file.
CREATE TABLE identify_file_keys (
    path     TEXT PRIMARY KEY,
    size     INTEGER NOT NULL,
    mtime    INTEGER NOT NULL,
    file_key TEXT NOT NULL
);
-- What a song looked like when identification last checked it (crate::identify::input_key): a
-- "not found" song whose tags, file or the node's services changed since is checked again.
ALTER TABLE identify_state ADD COLUMN input_key TEXT;
-- Suggestion ids are never given again (AUTOINCREMENT): an id a stale page holds names no other
-- suggestion. The rows are copied as they are.
CREATE TABLE identify_suggestions_ids (
    id             INTEGER PRIMARY KEY AUTOINCREMENT,
    track_id       INTEGER NOT NULL REFERENCES tracks(id) ON DELETE CASCADE,
    recording_mbid TEXT NOT NULL,
    release_mbid   TEXT,
    title          TEXT,
    artist         TEXT,
    release_title  TEXT,
    score          REAL NOT NULL,
    source         TEXT NOT NULL,
    created_at     INTEGER NOT NULL,
    UNIQUE (track_id, recording_mbid)
);
INSERT INTO identify_suggestions_ids(id, track_id, recording_mbid, release_mbid, title, artist, release_title, score, source, created_at)
    SELECT id, track_id, recording_mbid, release_mbid, title, artist, release_title, score, source, created_at FROM identify_suggestions;
DROP TABLE identify_suggestions;
ALTER TABLE identify_suggestions_ids RENAME TO identify_suggestions;
CREATE INDEX identify_suggestions_track ON identify_suggestions(track_id);
