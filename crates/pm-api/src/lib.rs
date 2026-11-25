//! The node's native API (handoff §9): HTTP + WebSocket under `/v1`.
//!
//! Resources: `info`, `outputs`, `zones` (queue, transport, volume, repeat),
//! `library` (stats, tracks, albums, search, scan) and the event stream at
//! `/v1/events`. Bodies are in [`types`]; the same JSON shapes drive the
//! controller. OpenSubsonic (M4) will be a second router next to this one.
//!
//! Auth: an optional bearer token (`NodeState::token`). Without one the API
//! trusts whoever can reach the socket, so bind to localhost. Remote access
//! by pubkey (M5) wraps this API rather than replacing it.

pub mod about;
pub mod access;
pub mod access_token;
pub mod activity;
pub mod app_passwords;
pub mod archive;
pub mod art;
pub mod artist_info;
pub mod audius;
pub mod copy_guard;
pub mod copy_jobs;
pub mod renderer;

/// "1 album", "2 albums".
pub(crate) fn plural(n: u64, one: &str) -> String {
    if n == 1 {
        format!("1 {one}")
    } else {
        format!("{n} {one}s")
    }
}

pub mod accounts;
pub mod add_music;
pub mod announce;
pub mod apps;
pub mod audiobookshelf;
pub mod backup;
pub mod bandcamp;
pub mod bookmarks;
pub mod bundled;
mod capped;
mod client_mask;
pub mod comments;
pub mod dash;
pub mod deezer;
pub mod devices;
pub mod discover;
pub mod downloads;
pub mod drives;
pub mod events;
pub mod feedback;
pub mod files;
pub mod first_run;
pub mod first_steps;
pub mod gpodder;
pub mod groups;
pub mod health;
pub mod hls;
pub mod home;
pub mod identify;
pub mod imports;
pub mod item;
pub mod jamendo;
pub mod jellyfin;
pub mod lastfm;
pub mod library_edit;
pub mod library_match;
pub mod librespot;
pub mod listenbrainz;
pub mod listens;
pub mod loudness;
pub mod lrclib;
pub mod mbproxy;
pub mod new_releases;
pub mod opensubsonic;
pub mod party;
pub mod peer;
pub mod people;
pub mod personal;
pub mod personal_collections;
pub mod pinakes_reader;
pub mod play_from;
pub mod playlists;
pub mod plex;
pub mod podcasts;
pub mod prefs;
pub mod presence;
pub mod profiles;
pub mod project_apps;
pub mod qobuz;
pub mod quality;
pub mod radio;
pub mod radio_sources;
pub mod releases;
pub mod remote;
pub mod reviews;
pub mod roon_import;
pub mod search;
pub mod segmented;
pub(crate) mod send_queue;
pub mod shared;
pub mod signin;
pub mod smart;
pub mod smart_network;
pub mod soloist;
pub mod soundcloud;
pub mod sources;
pub mod spotify;
pub mod state;
pub mod storage;
pub mod streaming;
pub mod subsonic;
pub mod sync;
pub mod tag_fixes;
pub mod tags;
#[doc(hidden)]
pub mod test_hooks;
pub mod tidal;
pub mod tidal_unofficial;
pub mod tunnel;
pub mod types;
pub mod upnp_events;
pub mod usage;
pub mod vault;
pub mod vault_cache;
pub mod watch;
mod web;
pub mod webdav;
pub mod youtube;
pub mod ytdlp;

use axum::extract::{Path, Query, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, post, put};
use axum::{middleware, Json, Router};
use futures_util::StreamExt;
use pm_model::TrackId;
use pm_zones::{Insert, Output, OutputId, OutputInfo, QueueItem, Zone, ZoneId, ZoneState};
use sources::NodeSources;
pub use state::{NodeSettings, NodeState};
use std::sync::Arc;
use types::*;

// ---- errors ----

#[derive(Debug)]
pub struct ApiError {
    pub status: StatusCode,
    pub message: String,
}

impl ApiError {
    pub(crate) fn new(status: StatusCode, message: impl Into<String>) -> Self {
        Self {
            status,
            message: message.into(),
        }
    }
    pub(crate) fn not_found(what: impl Into<String>) -> Self {
        Self::new(StatusCode::NOT_FOUND, what)
    }
    pub(crate) fn bad_request(what: impl Into<String>) -> Self {
        Self::new(StatusCode::BAD_REQUEST, what)
    }
}

impl From<pm_library::LibraryError> for ApiError {
    fn from(e: pm_library::LibraryError) -> Self {
        Self::new(StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
    }
}

impl From<pm_zones::SinkError> for ApiError {
    fn from(e: pm_zones::SinkError) -> Self {
        Self::new(StatusCode::BAD_REQUEST, e.to_string())
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.status, Json(ErrorBody { error: self.message })).into_response()
    }
}

pub(crate) type ApiResult<T> = Result<Json<T>, ApiError>;

// ---- router ----

/// The framework's own refusals (a JSON body that doesn't fit, a wrong method) come as plain
/// text such as "Failed to deserialize the JSON body…". Pages show `error` to people, so those
/// become the node's error shape with a plain sentence; the detail goes to the log and `detail`
/// (STEWARD-FIX-same-artist.md: no raw errors in the UI).
async fn plain_errors(resp: Response) -> Response {
    let st = resp.status();
    let text = resp
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|c| c.starts_with("text/plain"));
    if !(st.is_client_error() || st.is_server_error()) || !text || st == StatusCode::UNAUTHORIZED {
        return resp;
    }
    let (_, body) = resp.into_parts();
    let raw = axum::body::to_bytes(body, 64 * 1024).await.unwrap_or_default();
    let raw = String::from_utf8_lossy(&raw).into_owned();
    tracing::warn!(status = %st, "request refused: {raw}");
    let error = match st.as_u16() {
        400 | 415 | 422 => "The node didn't understand that request. Reload the page and try again.",
        404 => "That isn't here (any more).",
        405 => "The node can't do that here.",
        413 => "That is too big for the node to take.",
        _ => "Something went wrong on the node. Try again.",
    };
    (st, Json(serde_json::json!({ "error": error, "detail": raw }))).into_response()
}

/// Marks a response from a deprecated path (RFC 9745 `Deprecation`, and the path to use).
async fn deprecated(mut resp: Response) -> Response {
    let h = resp.headers_mut();
    h.insert("deprecation", axum::http::HeaderValue::from_static("true"));
    h.insert(
        axum::http::header::LINK,
        axum::http::HeaderValue::from_static("</v1/bookmarks>; rel=\"successor-version\""),
    );
    resp
}

/// The `/v1` routes. Each names the capability it needs, per method, in [`access::ROUTES`].
fn v1_routes() -> access::Routes {
    access::Routes::new()
        .route("/info", get(info))
        .route("/health", get(health::health))
        .route("/node", get(groups::index))
        .route("/node/apps", get(apps::get))
        .route("/node/apps/{id}", put(apps::put))
        .route("/node/settings", get(node_settings).put(update_node_settings))
        .route("/node/token", post(access_token::replace))
        // App passwords: one per app, instead of the API token (app_passwords.rs)
        .route(
            "/node/app-passwords",
            get(app_passwords::get).post(app_passwords::post),
        )
        .route("/node/app-passwords/{id}", delete(app_passwords::delete))
        // Usage insights, the owner's side (usage.rs; listeners are refused under /node too)
        .route("/node/usage", get(usage::status).put(usage::set_gate))
        .route("/node/usage/clear", post(usage::clear_all))
        .route("/node/usage/summary", get(usage::summary))
        .route("/node/usage/export", get(usage::export))
        .route("/node/settings/backup", post(settings_backup_now))
        .route("/node/settings/restore", post(settings_restore_now))
        .route("/outputs", get(outputs))
        .route("/outputs/rescan", post(rescan_outputs))
        .route("/outputs/discovery", get(discovery_report))
        .route("/outputs/upnp", post(add_upnp_output))
        .route("/outputs/upnp/{address}", delete(remove_upnp_address))
        .route("/outputs/endpoints", post(add_endpoint))
        .route("/outputs/endpoints/{address}", delete(remove_endpoint))
        .route("/outputs/pull", post(create_pull))
        .route("/outputs/{id}/device", axum::routing::put(put_output_device))
        .route("/outputs/pull/{name}/url", get(pull_url))
        .route("/zones", get(zones).post(create_zone))
        .route("/zones/{id}", get(zone).delete(delete_zone))
        .route("/zones/{id}/queue", get(queue).post(enqueue).delete(clear_queue))
        .route(
            "/zones/{id}/queue/{item}",
            axum::routing::delete(remove_item).put(move_item),
        )
        .route("/zones/{id}/transport", post(transport))
        .route("/zones/{id}/announce", post(announce::announce))
        .route("/zones/{id}/volume", post(volume).get(read_volume))
        .route("/zones/{id}/repeat", axum::routing::put(repeat))
        .route("/zones/{id}/radio", axum::routing::put(radio))
        .route("/zones/{id}/crossfade", axum::routing::put(crossfade))
        .route("/zones/{id}/shuffle", post(shuffle_queue).put(set_shuffle))
        .route("/zones/{id}/transfer", post(transfer))
        .route("/zones/{id}/load", post(load))
        .route("/zones/{id}/sleep", get(sleep_get).put(sleep_set))
        .route("/zones/{id}/dsp", get(dsp_get).put(dsp_put).delete(dsp_reset))
        .route("/library/stats", get(library_stats))
        .route("/library/tracks", get(tracks))
        .route(
            "/library/tracks/{id}",
            get(track).delete(library_edit::hide_track),
        )
        .route(
            "/library/tracks/{id}/edit",
            get(library_edit::get_edit)
                .put(library_edit::edit_track)
                .delete(library_edit::undo_edit),
        )
        .route("/library/albums/{id}/edit", put(library_edit::edit_album))
        .route("/library/hidden", get(library_edit::hidden))
        .route("/library/hidden/restore", post(library_edit::restore))
        .route("/me/music", get(library_edit::my_music))
        .route("/library/tracks/{id}/lyrics", get(lyrics))
        .route("/lyrics", get(lrclib::get))
        .route("/library/tracks/{id}/versions", get(track_versions))
        .route("/library/albums/{id}/versions", get(album_versions))
        .route("/backup", get(backup::get_backup).post(backup::post_backup))
        .route(
            "/restore",
            post(backup::post_restore).layer(axum::extract::DefaultBodyLimit::max(4 << 30)),
        )
        .route(
            "/backup/schedule",
            get(backup::get_schedule).put(backup::put_schedule),
        )
        .route("/backup/schedule/run", post(backup::run_schedule))
        .route("/library/folders/move", post(move_folder))
        .route("/library/tree", get(copy_jobs::tree))
        .route("/library/tracks/{id}/file", get(track_file))
        .route("/library/tracks/{id}/download", get(downloads::download))
        .route("/library/search", get(search))
        .route("/library/loudness", get(loudness::get))
        .route("/library/loudness/measure", post(loudness::measure))
        .route("/library/matches", get(library_match::status))
        .route("/library/artists", get(artists))
        .route("/library/artists/{id}", get(artist))
        .route("/library/artists/{id}/tracks", get(artist_tracks))
        .route("/library/artists/{id}/info", get(artist_info_handler))
        .route("/library/artists/{id}/image", get(artist_image))
        .route("/library/genres", get(genres))
        .route("/library/genres/{id}", get(genre))
        .route("/library/albums", get(albums))
        .route("/library/albums/formats", get(album_formats))
        .route("/library/new-releases", get(new_releases))
        .route("/library/new-releases/refresh", post(new_releases_refresh))
        .route("/library/albums/{id}/tracks", get(album_tracks))
        .route("/library/albums/{id}/cover", get(album_cover))
        .route("/library/history", get(history).post(record_play))
        .route("/library/home", get(home))
        .route(
            "/library/folders",
            get(folders).post(add_folder).delete(remove_folder),
        )
        .route("/library/folders/suggest", get(first_run::suggest_folders))
        .route("/library/scan", post(scan))
        // Collections: the same resource as /playlists (playlists.rs); each caller their own
        // (a listener's: personal_collections.rs)
        .route(
            "/collections",
            get(personal_collections::list).post(personal_collections::create),
        )
        .route(
            "/collections/{id}",
            get(personal_collections::get)
                .put(personal_collections::update)
                .delete(personal_collections::delete),
        )
        .route("/collections/{id}/items", post(personal_collections::add_items))
        .route(
            "/collections/{id}/items/{index}",
            axum::routing::delete(personal_collections::remove_item),
        )
        .route(
            "/collections/{id}/local/{index}",
            axum::routing::delete(playlists::remove_local),
        )
        .route("/collections/{id}/play", post(personal_collections::play))
        .route("/collections/{id}/play-from", get(play_from::playlist_play_from))
        .route("/collections/{id}/order", put(playlists::set_order))
        .route(
            "/collections/{id}/visibility",
            put(personal_collections::set_visibility),
        )
        .route(
            "/collections/{id}/items/{index}/source",
            put(playlists::pin_source),
        )
        .route(
            "/playlists",
            get(personal_collections::list).post(personal_collections::create),
        )
        .route("/playlists/from-queue", post(playlists::from_queue))
        .route("/playlists/import", post(activity::import_playlist))
        .route("/playlists/import/file", post(activity::import_playlist_file))
        .route("/import/roon", post(roon_import::import))
        .route("/activity", get(activity::list).post(activity::record))
        .route(
            "/bookmarks",
            get(bookmarks::list).put(bookmarks::put).delete(bookmarks::delete),
        )
        .route(
            "/comments",
            get(comments::list)
                .post(comments::create)
                .delete(comments::delete),
        )
        .route("/posts", post(comments::share))
        // The name before social v1 bookmarks, for one release (docs/api.md, Deprecated)
        .route(
            "/favourites",
            get(bookmarks::list)
                .put(bookmarks::put)
                .delete(bookmarks::delete)
                .layer(middleware::map_response(deprecated)),
        )
        .route("/tags", get(tags::list).put(tags::put).delete(tags::delete))
        .route("/tags/bulk", post(tags::bulk))
        .route(
            "/reviews",
            get(reviews::list).put(reviews::put).delete(reviews::delete),
        )
        .route(
            "/playlists/{id}",
            get(personal_collections::get)
                .put(personal_collections::update)
                .delete(personal_collections::delete),
        )
        .route("/playlists/{id}/items", post(personal_collections::add_items))
        .route(
            "/playlists/{id}/local/{index}",
            axum::routing::delete(playlists::remove_local),
        )
        .route(
            "/playlists/{id}/items/{index}",
            axum::routing::delete(personal_collections::remove_item),
        )
        .route("/playlists/{id}/play", post(personal_collections::play))
        .route("/playlists/{id}/play-from", get(play_from::playlist_play_from))
        .route("/playlists/{id}/order", put(playlists::set_order))
        .route(
            "/playlists/{id}/visibility",
            put(personal_collections::set_visibility),
        )
        .route("/playlists/{id}/items/{index}/source", put(playlists::pin_source))
        .route("/shared/resolve", get(shared::resolve))
        .route("/shared/{user}/playlists", get(shared::list))
        .route("/shared/{user}/playlists/{id}", get(shared::get))
        .route("/shared/{user}/playlists/{id}/copy", post(shared::duplicate))
        .route("/shared/{user}/collections", get(shared::list_collections))
        .route("/shared/{user}/collections/{id}", get(shared::get_collection))
        .route(
            "/shared/{user}/collections/{id}/duplicate",
            post(shared::duplicate),
        )
        .route("/play-order", get(play_from::get_order).put(play_from::set_order))
        .route("/discover/trends", get(discover::trends))
        .route("/discover/charts", get(discover::charts))
        .route("/discover/search", get(discover::search))
        .route("/discover/item", get(discover::item_detail))
        .route("/cards", get(pinakes_reader::get_card))
        .route("/discover/play", get(discover::play_plan))
        .route("/discover/describe", post(discover::describe))
        .route("/discover/contents", get(discover::contents))
        .route("/search", get(search::search))
        .route("/search/tags", get(search::tag_suggestions))
        .route("/home/location", get(home::get_location).put(home::set_location))
        .route("/home/layout", get(home::get_layout).put(home::set_layout))
        .route("/smart", get(smart::list).post(smart::create))
        .route("/smart/preview", post(smart::preview))
        .route(
            "/smart/{id}",
            axum::routing::put(smart::update).delete(smart::delete),
        )
        .route("/smart/{id}/tracks", get(smart::tracks))
        .route("/home/{section}", get(home::section))
        .route("/item/{kind}/{id}", get(item::page))
        .route("/item/{kind}/{id}/extras", get(item::page_extras))
        .route(
            "/library/identify",
            get(identify::get_status).post(identify::start),
        )
        .route("/library/identify/settings", put(identify::put_settings))
        .route("/library/identify/key/check", post(identify::check_key))
        .route("/library/identify/write", post(identify::write_now))
        .route("/library/identify/fix", get(identify::fix_view))
        .route("/library/tag-fixes", get(tag_fixes::list).post(tag_fixes::apply))
        .route("/library/suggestions", get(identify::suggestions))
        .route("/library/suggestions/albums", get(identify::review_albums))
        .route("/library/suggestions/confirm", post(identify::confirm))
        .route("/library/suggestions/skip", post(identify::skip))
        .route("/library/suggestions/unskip", post(identify::unskip))
        .route("/library/tracks/{id}/candidates", get(identify::track_candidates))
        .route("/library/tracks/{id}/match", post(identify::set_match))
        .route("/library/suggestions/{id}/accept", post(identify::accept))
        .route("/library/suggestions/{id}", delete(identify::reject))
        .route("/discover/uri", get(discover::on_pubky))
        .route("/discover/tag/{label}", get(discover::by_tag))
        .route("/discover/friends/{viewer}", get(discover::friends))
        .route("/sources", get(streaming::list))
        .route("/sources/{source}/search", get(streaming::search))
        .route(
            "/sources/{source}/enabled",
            axum::routing::put(streaming::set_enabled),
        )
        .route(
            "/sources/{source}/signin",
            get(signin::get).put(signin::put).delete(signin::sign_out),
        )
        .route("/sources/{source}/quality", get(quality::get).put(quality::put))
        .route("/sources/{source}/signin/import", post(signin::import))
        .route("/sources/{source}/items/{id}", get(streaming::item))
        .route("/sources/{source}/artists/{id}", get(streaming::artist))
        .route("/sources/tidal/login", get(tidal::login))
        .route("/sources/tidal/status", get(tidal::status))
        .route(
            "/sources/tidal/unofficial",
            get(tidal_unofficial::get_status).put(tidal_unofficial::put),
        )
        .route(
            "/sources/tidal/unofficial/login",
            post(tidal_unofficial::login).delete(tidal_unofficial::logout),
        )
        .route(
            "/sources/tidal/unofficial/client/check",
            post(tidal_unofficial::check_client),
        )
        .route(
            "/sources/tidal/unofficial/browser",
            post(tidal_unofficial::start_browser),
        )
        .route(
            "/sources/tidal/unofficial/browser/finish",
            post(tidal_unofficial::finish_browser),
        )
        .route("/sources/tidal/session", axum::routing::delete(tidal::logout))
        .route("/sources/tidal/collection", get(tidal::collection))
        .route("/sources/tidal/callback", post(tidal::pasted_callback))
        .route("/sources/spotify/callback", post(spotify::pasted_callback))
        .route("/sources/tidal/stream/{id}", get(tidal::stream))
        .route("/sources/deezer/stream/{id}", get(deezer::stream))
        .route("/sources/jamendo/stream/{id}", get(jamendo::stream))
        .route("/sources/qobuz/stream/{id}", get(qobuz::stream))
        .route(
            "/sources/qobuz/unofficial",
            get(qobuz::get_status).put(qobuz::put),
        )
        .route(
            "/sources/qobuz/unofficial/login",
            post(qobuz::login).delete(qobuz::logout),
        )
        .route(
            "/sources/youtube/key",
            get(youtube::get_key).put(youtube::put_key),
        )
        .route(
            "/sources/youtube/unofficial",
            get(ytdlp::get_status).put(ytdlp::put),
        )
        .route(
            "/sources/youtube/unofficial/ytdlp",
            post(ytdlp::install).delete(ytdlp::uninstall),
        )
        .route("/sources/bandcamp/stream/{key}", get(bandcamp::stream))
        .route(
            "/sources/bandcamp/unofficial",
            get(bandcamp::get_status).put(bandcamp::put),
        )
        .route("/sources/bandcamp/collection", get(bandcamp::collection))
        .route(
            "/sources/bandcamp/collection/{id}/download",
            post(bandcamp::download),
        )
        .route("/sources/bandcamp/downloads", get(bandcamp::downloads))
        .route(
            "/sources/spotify/soloist",
            get(soloist::get_status).put(soloist::put),
        )
        .route(
            "/sources/spotify/soloist/binary",
            post(soloist::install).delete(soloist::uninstall),
        )
        .route("/sources/spotify/soloist/control", post(soloist::control))
        .route(
            "/sources/spotify/librespot",
            get(librespot::get_status).put(librespot::put),
        )
        .route("/sources/soundcloud/stream/{id}", get(soundcloud::stream))
        .route(
            "/sources/soundcloud/unofficial",
            get(soundcloud::get_status).put(soundcloud::put),
        )
        .route("/sources/soundcloud/likes", get(soundcloud::likes))
        .route("/sources/soundcloud/likes/import", post(soundcloud::import_likes))
        .route("/sources/soundcloud/playlists", get(soundcloud::playlists))
        .route(
            "/sources/soundcloud/playlists/{id}/import",
            post(soundcloud::import_playlist),
        )
        .route(
            "/sources/subsonic/server",
            get(opensubsonic::get_server)
                .put(opensubsonic::set_server)
                .delete(opensubsonic::forget_server),
        )
        .route("/sources/subsonic/stream/{id}", get(opensubsonic::stream))
        .route(
            "/sources/jellyfin/server",
            get(jellyfin::get_server)
                .put(jellyfin::sign_in)
                .delete(jellyfin::sign_out),
        )
        .route("/sources/jellyfin/stream/{id}", get(jellyfin::stream))
        .route(
            "/listenbrainz",
            get(listenbrainz::status)
                .put(listenbrainz::connect)
                .delete(listenbrainz::disconnect),
        )
        .route("/listenbrainz/import", post(listenbrainz::import_listens))
        .route(
            "/lastfm",
            get(lastfm::status).put(lastfm::update).delete(lastfm::disconnect),
        )
        .route("/lastfm/login", get(lastfm::login))
        .route("/library/artists/{id}/similar", get(lastfm::similar))
        .route(
            "/listenbrainz/recommendations",
            get(listenbrainz::recommendations),
        )
        .route("/listenbrainz/playlists", get(listenbrainz::playlists))
        .route(
            "/listenbrainz/playlists/{id}/import",
            post(listenbrainz::import_playlist),
        )
        .route("/sources/jellyfin/cover/{id}", get(jellyfin::cover))
        .route(
            "/sources/plex/server",
            get(plex::get_server).put(plex::connect).delete(plex::disconnect),
        )
        .route("/sources/plex/pin", post(plex::start_pin))
        .route("/sources/plex/pin/{id}", get(plex::poll_pin))
        .route("/sources/plex/stream/{id}", get(plex::stream))
        .route("/sources/plex/cover/{id}/{ver}", get(plex::cover))
        .route(
            "/sources/audiobookshelf/server",
            get(audiobookshelf::get_server)
                .put(audiobookshelf::connect)
                .delete(audiobookshelf::disconnect),
        )
        .route("/sources/audiobookshelf/play", post(audiobookshelf::play))
        .route(
            "/sources/audiobookshelf/continue",
            get(audiobookshelf::in_progress),
        )
        .route(
            "/sources/audiobookshelf/stream/{item}/{ino}",
            get(audiobookshelf::stream),
        )
        .route("/sources/audiobookshelf/cover/{item}", get(audiobookshelf::cover))
        .route("/sources/jellyfin/playlists", get(jellyfin::playlists))
        .route(
            "/sources/jellyfin/playlists/{id}/import",
            post(jellyfin::import_playlist),
        )
        .route("/sources/subsonic/cover/{id}", get(opensubsonic::cover))
        .route("/sources/subsonic/playlists", get(opensubsonic::playlists))
        .route(
            "/sources/subsonic/playlists/{id}/import",
            post(opensubsonic::import_playlist),
        )
        .route("/sources/jamendo/status", get(jamendo::status))
        .route("/sources/jamendo/login", post(jamendo::login))
        .route("/sources/jamendo/logout", post(jamendo::logout))
        .route(
            "/sources/jamendo/collection/import",
            post(jamendo::import_collection),
        )
        .route("/sources/spotify/login", get(spotify::login))
        .route("/sources/spotify/status", get(spotify::status))
        .route("/sources/spotify/app", put(spotify::set_app))
        .route("/sources/spotify/session", delete(spotify::logout))
        .route("/sources/spotify/collection", get(spotify::collection))
        .route(
            "/sources/spotify/collection/import",
            post(spotify::import_collection),
        )
        .route("/sources/spotify/playlists", get(spotify::playlists))
        .route(
            "/sources/spotify/playlists/{id}/import",
            post(spotify::import_playlist),
        )
        .route("/sources/spotify/devices", get(spotify::devices))
        .route("/sources/spotify/play", post(spotify::play))
        .route("/sources/tidal/collection/import", post(tidal::import_collection))
        .route("/sources/tidal/playlists", get(tidal::playlists))
        .route(
            "/sources/tidal/playlists/{id}/import",
            post(tidal::import_playlist),
        )
        .route("/radio/search", get(radio::search))
        .route("/radio/stations", get(radio::saved).put(radio::save))
        .route("/radio/stations/{id}", axum::routing::delete(radio::remove))
        .route("/radio/play", post(radio::play))
        .route("/podcasts/search", get(podcasts::search))
        .route(
            "/podcasts/subscriptions",
            get(podcasts::subscriptions).post(podcasts::subscribe),
        )
        .route(
            "/podcasts/subscriptions/{id}",
            get(podcasts::subscription).delete(podcasts::unsubscribe),
        )
        .route("/podcasts/new", get(podcasts::newest))
        .route("/podcasts/refresh", post(podcasts::refresh_route))
        .route(
            "/podcasts/gpodder",
            get(gpodder::get).put(gpodder::put).delete(gpodder::delete),
        )
        .route("/podcasts/gpodder/sync", post(gpodder::sync_now))
        .route("/podcasts/play", post(podcasts::play))
        .route(
            "/podcasts/progress/{id}",
            axum::routing::put(podcasts::set_progress),
        )
        .route("/radio/curated", get(radio_sources::curated_route))
        .route("/radio/countries", get(radio_sources::countries))
        .route("/radio/tags", get(radio_sources::tags))
        .route("/homeserver/bundled", get(bundled::status))
        // Everything about the bundled homeserver lives under it ("This homeserver" in Settings);
        // `/v1/remote/tunnel` stays as an alias for one release
        .route(
            "/homeserver/bundled/tunnel",
            get(tunnel::status).put(tunnel::setup).delete(tunnel::remove),
        )
        .route("/homeserver/bundled/owner-signup", post(bundled::owner_signup))
        .route(
            "/remote/tunnel",
            get(tunnel::status).put(tunnel::setup).delete(tunnel::remove),
        )
        .route("/storage", get(storage::status))
        .route("/storage/save", post(storage::save))
        .route("/library/uploads", get(add_music::list).post(add_music::start))
        .route(
            "/library/uploads/{id}",
            get(add_music::get)
                .put(add_music::put_chunk)
                .delete(add_music::cancel)
                .layer(axum::extract::DefaultBodyLimit::max(add_music::CHUNK_LIMIT)),
        )
        .route("/library/imports", post(add_music::import))
        .route("/profiles", get(profiles::list))
        .route("/profiles/{id}", get(profiles::page))
        .route("/profiles/{id}/avatar", get(profiles::avatar))
        .route(
            "/profiles/{id}/follow",
            axum::routing::put(profiles::follow).delete(profiles::unfollow),
        )
        .route("/library/webdav", get(webdav::list).post(webdav::add))
        .route("/library/drives", get(drives::list).post(drives::add))
        .route("/library/drives/{provider}/login", post(drives::login))
        .route("/library/drives/{provider}/folders", get(drives::folders))
        .route(
            "/library/drives/{provider}/account",
            axum::routing::delete(drives::sign_out),
        )
        .route("/library/drives/roots/{id}/scan", post(drives::rescan))
        .route(
            "/library/drives/roots/{id}",
            axum::routing::delete(drives::remove),
        )
        .route("/library/webdav/{id}", axum::routing::delete(webdav::remove))
        .route("/library/webdav/{id}/scan", post(webdav::rescan))
        .route("/me", get(accounts::me))
        .route("/me/prefs", get(prefs::get).put(prefs::put))
        .route(
            "/me/presence",
            get(presence::setting_get).put(presence::setting_put),
        )
        .route("/presence", get(presence::list))
        .route("/presence/along", post(presence::join).delete(presence::leave))
        .route("/me/first-steps", get(first_steps::get))
        .route("/me/token", axum::routing::delete(accounts::sign_out))
        // Sign-ins, each person's own: list them, end one, end all the others
        .route(
            "/me/sign-ins",
            get(accounts::my_sign_ins).delete(accounts::end_my_sign_ins),
        )
        .route("/me/sign-ins/{id}", delete(accounts::end_my_sign_in))
        // A listener's own homeserver, for their bookmarks (personal.rs)
        .route(
            "/me/homeserver",
            get(personal::status).delete(personal::disconnect),
        )
        .route("/me/homeserver/connect", post(personal::connect_start))
        .route("/me/homeserver/connect/poll", get(personal::connect_poll))
        // Usage insights, each person's own side (usage.rs: owner and listeners, never guests or
        // a node without a token; the handlers read bodies themselves, within their own limits)
        .route("/usage/me", get(usage::me).put(usage::choose))
        .route("/usage/me/notice", post(usage::notice))
        .route("/usage/me/clear", post(usage::clear_mine))
        .route("/usage/sessions", post(usage::open))
        .route("/usage/sessions/close", post(usage::close))
        .route("/usage/events", post(usage::ingest))
        .route("/auth/pubky/renewal", post(login_renewal))
        .route("/me/listens", get(listens::list).delete(listens::delete_all))
        .route("/me/listens/stats", get(listens::get_stats))
        .route(
            "/me/listens/settings",
            get(listens::get_settings).put(listens::put_settings),
        )
        .route("/me/listens/visibility", post(listens::move_all))
        .route("/people/zones", get(accounts::get_zones).put(accounts::set_zones))
        .route("/me/services", get(signin::my_services))
        .route(
            "/me/sources",
            get(accounts::my_sources).put(accounts::set_my_sources),
        )
        .route("/people", get(people::list))
        // Problem reports to the node's private inbox (feedback.rs)
        .route("/feedback", post(feedback::send))
        .route("/feedback/mine", get(feedback::mine))
        .route("/devices", get(devices::list))
        .route(
            "/devices/{id}",
            axum::routing::put(devices::register).delete(devices::forget),
        )
        .route(
            "/devices/{id}/downloads",
            get(devices::get_downloads).put(devices::put_downloads).layer(
                axum::extract::DefaultBodyLimit::max(devices::DOWNLOADS_BODY_LIMIT),
            ),
        )
        .route("/vault", get(vault::get))
        .route("/vault/register", post(vault::register))
        .route("/vault/sync", post(vault::sync_now))
        .route("/vault/devices/{key}/sign-out", post(vault::sign_out_device))
        .route("/vault/sign-out-everywhere", post(vault::sign_out_everywhere))
        .route("/vault/new-key", post(vault::new_key))
        .route("/people/invites", post(people::invite))
        .route(
            "/people/invites/{id}",
            axum::routing::delete(people::forget_invite),
        )
        .route(
            "/people/{key}",
            axum::routing::patch(people::change).delete(people::remove),
        )
        .route("/people/{key}/sign-ins", delete(people::end_sign_ins))
        .route("/people/{key}/sign-ins/{id}", delete(people::end_sign_in))
        .route("/pubky", get(pubky_status))
        .route("/pubky/sync", post(pubky_sync))
        .route("/pubky/login", post(pubky_login_start).get(pubky_login_poll))
        .route(
            "/pubky/files",
            get(files::list).post(files::upload).delete(files::remove),
        )
        .route("/pubky/files/mirror", post(files::mirror))
        .route("/pubky/files/upload", post(copy_jobs::start))
        .route("/pubky/files/upload/{id}", get(files::upload_progress))
        .route("/pubky/files/upload/{id}/pause", post(copy_jobs::pause))
        .route("/pubky/files/upload/{id}/resume", post(copy_jobs::resume))
        .route("/pubky/files/upload/{id}/cancel", post(copy_jobs::cancel_route))
        .route("/pubky/files/upload/{id}/retry", post(copy_jobs::retry))
        .route("/pubky/files/jobs", get(copy_jobs::jobs))
        .route("/pubky/files/jobs/last", delete(copy_jobs::dismiss_last))
        .route("/pubky/files/jobs/{id}", get(files::upload_progress))
        .route("/pubky/files/jobs/{id}/cancel", post(files::cancel_job))
        .route("/events", get(events::events_ws))
        // Party mode: the owner's parties, and what a guest token opens (party.rs)
        .route("/party", get(party::list).post(party::create))
        .route("/party/{id}", delete(party::end))
        .route("/party/guest", get(party::guest))
        .route("/party/guest/search", get(party::search))
        .route("/party/guest/queue", post(party::add))
}

pub fn router(state: Arc<NodeState>) -> Router {
    router_with_pages(state, Router::new())
}

/// The node's API next to `pages`: the web UI's pages and files (`pm_web::router()`), which
/// need no token. The pages are their own crate, so a change to the UI doesn't rebuild this one.
pub fn router_with_pages(state: Arc<NodeState>, pages: Router) -> Router {
    let (v1, _) = v1_routes().into_parts();
    let v1 = v1
        .route_layer(middleware::from_fn_with_state(state.clone(), auth))
        .layer(middleware::map_response(plain_errors))
        .with_state(state.clone());
    // The web UI's pages and files (pm-web), public like the routes without a token below
    pages
        .nest("/v1", v1)
        .merge(open_routes().into_open_router().with_state(state.clone()))
        .merge(subsonic::router(state.clone()))
        // The zones as UPnP/DLNA renderers (no token, like any renderer; `[renderer] enabled`),
        // to LAN peers at the node's address only, never another site's page (peer.rs)
        .merge(
            state
                .renderer
                .get()
                .map(|r| r.router().layer(middleware::from_fn(peer::renderer_only)))
                .unwrap_or_default(),
        )
        .layer(middleware::from_fn_with_state(
            state.clone(),
            peer::tokenless_direct_only,
        ))
        // `[api] behind_proxy` for the layers built without the state (peer::lan_only): outside
        // every one of them
        .layer(middleware::from_fn_with_state(
            state.clone(),
            peer::mark_behind_proxy,
        ))
        // Outermost: a node that is stopping answers 503 before any other layer runs, and the
        // request counts as in flight through all of them
        .layer(middleware::from_fn_with_state(
            state.clone(),
            peer::not_while_stopping,
        ))
}

/// The routes reached without a sign-in: the OAuth redirect targets and the Pubky Ring sign-in
/// that hands the token out, joining a party, and what carries a key of its own. Each is listed in
/// [`access::OPEN`] with why it is open; one that is not listed fails the tests.
fn open_routes() -> access::Routes {
    access::Routes::new()
        .route("/tidal/callback", get(tidal::callback))
        .route("/spotify/callback", get(spotify::callback))
        .route("/lastfm/callback", get(lastfm::callback))
        .route("/drives/{provider}/callback", get(drives::callback))
        .route("/jamendo/callback", get(jamendo::callback))
        .route("/v1/auth/pubky/start", post(controller_login_start))
        .route("/v1/auth/pubky/poll", get(controller_login_poll))
        .route("/v1/auth/pubky/owner/avatar", get(profiles::owner_avatar))
        // Joining a party from its link (party.rs; the link's page is pm-web's, public like every page)
        .route("/v1/party/join", post(party::join))
        // LAN discovery: the SSDP LOCATION (pm-discovery); names the node, nothing more.
        .route(pm_discovery::DESCRIPTION_PATH, get(upnp_description))
        // Renderers fetch pull streams with the stream's own key (`?k=`), or the token.
        .route("/v1/outputs/pull/{name}/{file}", get(pull_stream))
        // A renderer's volume events (GENA NOTIFY), to the token the node handed it; LAN only
        .route(
            "/v1/upnp/events/{token}",
            axum::routing::any(upnp_events::notify).layer(middleware::from_fn(peer::lan_only)),
        )
        // Album covers for TV screens, with that cover's key.
        .route("/v1/art/{id}", get(keyed_cover))
        // MusicBrainz caching proxy for Pinakes and the owner's tools.
        .route("/mb/ws/2/{*path}", get(mbproxy::forward))
}

/// Bind the API's address. A port in use is the one fatal port: the error names it and the
/// likely cause, so a restart loop says what to fix.
pub async fn bind(listen: &str) -> anyhow::Result<tokio::net::TcpListener> {
    match tokio::net::TcpListener::bind(listen).await {
        Ok(l) => {
            tracing::info!(%listen, "API port bound; serving once the node has started");
            Ok(l)
        }
        Err(e) => Err(anyhow::anyhow!(bind_error(listen, &e))),
    }
}

/// What to say when the API cannot listen on `listen`.
pub fn bind_error(listen: &str, e: &std::io::Error) -> String {
    match e.kind() {
        std::io::ErrorKind::AddrInUse => format!(
            "cannot listen on {listen}: the port is already in use, most likely by another Odeion node \
             on this machine (an older install, or a second container on the host network). Stop it, or set \
             [api] listen (PM_LISTEN) to a free port"
        ),
        std::io::ErrorKind::PermissionDenied => format!(
            "cannot listen on {listen}: permission denied (ports below 1024 need root); pick a port above 1024 \
             in [api] listen (PM_LISTEN)"
        ),
        _ => format!("cannot listen on {listen}: {e}"),
    }
}

/// Serve the API on an already bound listener (tests bind port 0).
pub async fn serve_listener(state: Arc<NodeState>, listener: tokio::net::TcpListener) -> anyhow::Result<()> {
    serve_listener_until(state, listener, std::future::pending()).await
}

/// How long requests being answered get to finish once the node is told to stop. Connections
/// that stay open by design (the event stream, audio to a player) are not waited for beyond it:
/// they end when the process does.
pub const STOP_GRACE: std::time::Duration = std::time::Duration::from_secs(2);

/// [`serve_listener`] until `stop` resolves (the node was asked to stop): no new connection is
/// taken here and no new request on any of the node's listeners ([`NodeState::begin_stop`]),
/// requests being answered, on this listener or another, get [`STOP_GRACE`] to finish, those
/// that have not by then are ended ([`NodeState::end_requests`]), and the call returns.
/// Connections still open then (the event stream, audio to a player) are left to end with the
/// process.
pub async fn serve_listener_until(
    state: Arc<NodeState>,
    listener: tokio::net::TcpListener,
    stop: impl std::future::Future<Output = ()> + Send + 'static,
) -> anyhow::Result<()> {
    let app = router(state.clone());
    serve_router_until(state, listener, app, stop).await
}

/// Serve the API and `pages` (the web UI, `pm_web::router()`) on an already bound listener.
pub async fn serve_listener_with_pages(
    state: Arc<NodeState>,
    listener: tokio::net::TcpListener,
    pages: Router,
) -> anyhow::Result<()> {
    serve_listener_with_pages_until(state, listener, pages, std::future::pending()).await
}

/// [`serve_listener_with_pages`] until `stop` resolves, as [`serve_listener_until`] does for the
/// API alone: what the node runs.
pub async fn serve_listener_with_pages_until(
    state: Arc<NodeState>,
    listener: tokio::net::TcpListener,
    pages: Router,
    stop: impl std::future::Future<Output = ()> + Send + 'static,
) -> anyhow::Result<()> {
    let app = router_with_pages(state.clone(), pages);
    serve_router_until(state, listener, app, stop).await
}

/// [`serve_listener_until`] for `app`, which carries the node's own layers (`router`,
/// `router_with_pages`).
pub(crate) async fn serve_router_until(
    state: Arc<NodeState>,
    listener: tokio::net::TcpListener,
    app: Router,
    stop: impl std::future::Future<Output = ()> + Send + 'static,
) -> anyhow::Result<()> {
    let (told, mut was_told) = tokio::sync::watch::channel(false);
    // Peer addresses: the renderer answers LAN peers only (peer.rs)
    let served = std::future::IntoFuture::into_future(
        axum::serve(
            listener,
            app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
        )
        .with_graceful_shutdown({
            let state = state.clone();
            async move {
                stop.await;
                // Every listener of this node, not only this one, stops taking requests
                state.begin_stop();
                let _ = told.send(true);
            }
        }),
    );
    let grace_over = async move {
        // (an error: `stop` never resolved and the server ended by itself)
        if was_told.wait_for(|t| *t).await.is_ok() {
            tokio::time::sleep(STOP_GRACE).await;
        } else {
            std::future::pending::<()>().await;
        }
    };
    // Done when this listener's connections are, and no request is being answered on another
    // of the node's listeners either (remote access has its own): those get the same time
    let all_done = async {
        let served = served.await;
        state.requests_done().await;
        served
    };
    tokio::select! {
        r = all_done => r?,
        () = grace_over => {
            state.end_requests();
            tracing::info!("requests not answered within the grace period were ended; open streams end with the node");
        }
    }
    Ok(())
}

async fn auth(
    State(state): State<Arc<NodeState>>,
    headers: HeaderMap,
    query: Query<std::collections::HashMap<String, String>>,
    params: Result<axum::extract::RawPathParams, axum::extract::rejection::RawPathParamsRejection>,
    mut req: axum::extract::Request,
    next: middleware::Next,
) -> Response {
    // The owner's API token, or a listener's personal one (accounts.rs)
    let route = req
        .extensions()
        .get::<axum::extract::MatchedPath>()
        .map(|m| m.as_str().to_string());
    let token = bearer_for(&headers, &query, req.method(), route.as_deref());
    // Stamp before authenticating: work delayed after admission must not inherit a later
    // removal/re-add or restore merely because music is enabled again when it resumes.
    let access_started = people::access_started(&state);
    let Some(caller) = accounts::caller_seen(&state, token.as_deref()) else {
        // An owner's browser session while the node has no identity loaded: not refused for good
        // (the page keeps it), it works again once the identity is. Log out still ends it, so the
        // browser can get to Log in (and the API token) meanwhile
        if let Some(t) = token
            .as_deref()
            .filter(|t| t.starts_with(accounts::OWNER_PREFIX))
            .filter(|_| state.sync_handle().is_none())
        {
            // (Paths here are inside `/v1`)
            if req.method() == axum::http::Method::DELETE && req.uri().path() == "/me/token" {
                return match accounts::end_owner_session(&state, t) {
                    Ok(ended) => {
                        Json(serde_json::json!({"signed_out": true, "ended": ended})).into_response()
                    }
                    Err(e) => e.into_response(),
                };
            }
            return ApiError::new(StatusCode::SERVICE_UNAVAILABLE, accounts::IDENTITY_NOT_LOADED)
                .into_response();
        }
        // The web page knows this answer (app.js NODE_REFUSED): it means "log in again"
        return ApiError::new(StatusCode::UNAUTHORIZED, "missing or wrong bearer token").into_response();
    };
    // What the matched route needs, and whether this caller may: the one place that decides
    let route = route.as_deref().unwrap_or_default();
    let at = access::At::of(route, params.iter().flat_map(|p| p.iter()));
    let refusal = access::decide(&state, &caller, access::required(req.method(), route), &at);
    if let Some(why) = refusal {
        // (an app is told as it is, not as a person would be)
        if caller.role == accounts::Role::App {
            return ApiError::new(StatusCode::FORBIDDEN, format!("{why}.")).into_response();
        }
        return accounts::refused(why).into_response();
    }
    // Who changed a zone last decides whether its plays are the owner's listens (listens.rs).
    // The zone is the one the router found in the path, decoded: the one the handler acts on
    if !matches!(*req.method(), axum::http::Method::GET | axum::http::Method::HEAD) {
        if let Some(zone) = at.zone.filter(|_| listens::zone_of_path(route).is_some()) {
            // (An app password's are the owner's)
            let owners = accounts::Who::of(&caller) == accounts::Who::Owner;
            listens::zone_changed_by(&state, zone, owners);
        }
    }
    // Listeners (and, to be safe, party guests) see file names, not where the owner keeps files
    let not_owner = caller.role != accounts::Role::Owner;
    // Who listens where (presence.rs): whoever plays on a zone
    let played = at
        .zone
        .filter(|_| presence::zone_written(req.method(), route).is_some())
        .map(|z| (z.to_string(), caller.clone()));
    // Under the node's source policy, a listener's catalogue reads may use the node's sign-ins
    let asking = match accounts::Who::of(&caller) {
        w @ accounts::Who::Listener(_) => w.resolved(&state.library.lock().expect("library poisoned")),
        w => w,
    };
    req.extensions_mut().insert(caller);
    // Catalogue reads (search, item pages) use the asking person's sign-in, or the app's
    let resp = personal::AUTHENTICATED_ACCESS
        .scope(access_started, accounts::ASKING.scope(asking, next.run(req)))
        .await;
    if let Some((zone, caller)) = played.filter(|_| resp.status().is_success()) {
        presence::played(&state, &zone, &caller);
    }
    if not_owner {
        let known = accounts::KnownTokens::lazy(&state);
        accounts::listener_response(
            resp,
            &accounts::library_roots(&state),
            &state.node_origins(),
            &|t| known.has(t),
        )
        .await
    } else {
        resp
    }
}

/// Where `?token=` stands in for the header: what a browser loads by address and can't set
/// headers on. Decided on the matched route (`/library/albums/{id}/cover`), never on the path a
/// request names, so a parameter called `cover` or `stream` opens nothing: the event stream, and
/// on GET/HEAD a route whose last fixed segment (before its parameters) is a song's file or
/// download, a cover, a picture, an avatar or a stream. Everything else takes the header only,
/// so a token in a link that is shared, saved or logged opens nothing else.
fn token_in_query_ok(method: &axum::http::Method, route: Option<&str>) -> bool {
    let Some(route) = route else { return false };
    let route = route.strip_prefix("/v1").unwrap_or(route);
    if route == "/events" {
        return true;
    }
    let last_fixed = route.split('/').rev().find(|seg| !seg.starts_with('{'));
    matches!(*method, axum::http::Method::GET | axum::http::Method::HEAD)
        && matches!(
            last_fixed,
            Some("file" | "download" | "cover" | "image" | "avatar" | "stream")
        )
}

/// The request's bearer token, or `?token=` on the routes a browser loads by address
/// ([`token_in_query_ok`]).
fn bearer_for(
    headers: &HeaderMap,
    query: &std::collections::HashMap<String, String>,
    method: &axum::http::Method,
    route: Option<&str>,
) -> Option<String> {
    headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .map(str::to_string)
        .or_else(|| {
            query
                .get("token")
                .cloned()
                .filter(|_| token_in_query_ok(method, route))
        })
}

/// The request's bearer token (or `?token=`, for WebSocket upgrades).
pub(crate) fn bearer(
    headers: &HeaderMap,
    query: &std::collections::HashMap<String, String>,
) -> Option<String> {
    headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .map(str::to_string)
        .or_else(|| query.get("token").cloned())
}

/// The request carries the API token (or the node has none).
fn token_ok(
    state: &NodeState,
    headers: &HeaderMap,
    query: &std::collections::HashMap<String, String>,
) -> bool {
    let Some(token) = state.token() else { return true };
    let bearer = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .map(str::to_string)
        // Browsers cannot set headers on WebSocket upgrades: allow `?token=`.
        .or_else(|| query.get("token").cloned());
    bearer.is_some_and(|b| crate::state::same_secret(&token, &b))
}

// ---- handlers: node ----

async fn info(
    State(s): State<Arc<NodeState>>,
    axum::Extension(c): axum::Extension<accounts::Caller>,
) -> ApiResult<Info> {
    let library = s.library.lock().expect("library poisoned").stats()?;
    let mut last_scan = s.last_scan.lock().expect("last scan lock").clone();
    // The unreadable files' paths are where the owner keeps music: a listener gets the count
    if c.role != accounts::Role::Owner {
        if let Some(l) = last_scan.as_mut() {
            l.errors.clear();
        }
    }
    Ok(Json(Info {
        id: s.node_id.clone(),
        remote: s.remote_id.read().expect("remote id poisoned").clone(),
        name: s.name.clone(),
        version: s.version.clone(),
        api: API_VERSION.into(),
        library,
        zones: s.zones.list().len(),
        outputs: s.outputs.len(),
        scanning: s.is_scanning(),
        last_scan,
    }))
}

async fn upnp_description(State(s): State<Arc<NodeState>>) -> Response {
    let xml = pm_discovery::description_xml(&s.announcement(0));
    ([(header::CONTENT_TYPE, "text/xml; charset=utf-8")], xml).into_response()
}

/// An output as `GET /v1/outputs` lists it.
#[derive(Debug, serde::Serialize)]
struct OutputView {
    #[serde(flatten)]
    info: OutputInfo,
    /// The speaker system that groups it with others of its kind itself (`sonos`, `heos`,
    /// `bluos`): a zone of only those is that system's own group, in sync (`POST /v1/zones`).
    #[serde(skip_serializing_if = "Option::is_none")]
    group_by: Option<&'static str>,
}

async fn outputs(State(s): State<Arc<NodeState>>) -> Json<Vec<OutputView>> {
    // One lock at a time
    fn ids<T>(m: &std::collections::HashMap<OutputId, T>) -> std::collections::HashSet<OutputId> {
        m.keys().cloned().collect()
    }
    let sonos = ids(&s.sonos.lock().expect("sonos lock"));
    let heos = ids(&s.heos.lock().expect("heos lock"));
    let bluos = ids(&s.bluos_players.lock().expect("bluos lock"));
    Json(
        s.outputs
            .list()
            .into_iter()
            .map(|info| {
                let group_by = if sonos.contains(&info.id) {
                    Some("sonos")
                } else if heos.contains(&info.id) {
                    Some("heos")
                } else if bluos.contains(&info.id) {
                    Some("bluos")
                } else {
                    None
                };
                OutputView { info, group_by }
            })
            .collect(),
    )
}

/// `PUT /v1/outputs/{id}/device {mode?}`: what a UI sends for the hardware behind an output (a
/// phone's USB DAC). It has one behaviour (Exclusive USB, else Android's bit-perfect, else shared
/// only where neither is possible): `mode: exclusive` asks Android's permission for Exclusive USB
/// again (Settings' "Allow Exclusive USB"); any other mode is that behaviour. An earlier UI's body
/// (`shared`, `bit_perfect`, `dac_volume`, `full_level_ok`) is taken and changes nothing. The
/// output's app gets it; the answer is the output as listed.
async fn put_output_device(
    State(s): State<Arc<NodeState>>,
    Path(id): Path<String>,
    Json(body): Json<DeviceSettingsPatch>,
) -> ApiResult<OutputInfo> {
    let out = s
        .outputs
        .get(&OutputId(id.clone()))
        .ok_or_else(|| ApiError::not_found(format!("no output {id}")))?;
    if out.device().is_none() {
        return Err(ApiError::bad_request(format!("{id} has no device to set")));
    }
    let mode = match body.mode {
        Some(pm_zones::PreferredMode::Unknown) => {
            return Err(ApiError::bad_request(
                "mode is auto, bit_perfect, exclusive or shared",
            ))
        }
        Some(pm_zones::PreferredMode::Exclusive) => pm_zones::PreferredMode::Exclusive,
        _ => pm_zones::PreferredMode::Auto,
    };
    let settings = pm_zones::DeviceSettings {
        mode,
        ..pm_zones::DeviceSettings::default()
    };
    out.configure_device(settings)
        .await
        .map_err(|e| ApiError::bad_request(e.to_string()))?;
    Ok(Json(out.info()))
}

/// The body of `PUT /v1/outputs/{id}/device`; an earlier UI's `dac_volume` and `full_level_ok`
/// are taken and not read.
#[derive(Debug, serde::Deserialize)]
struct DeviceSettingsPatch {
    #[serde(default)]
    mode: Option<pm_zones::PreferredMode>,
}

/// Create a pull output: a zone that plays to it streams WAV at
/// `/v1/outputs/pull/{name}/stream.wav` for any renderer or browser to fetch.
async fn create_pull(State(s): State<Arc<NodeState>>, Json(body): Json<CreatePull>) -> ApiResult<OutputInfo> {
    let name = body.name.trim();
    if name.is_empty()
        || !name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return Err(ApiError::bad_request("name must be letters, digits, '-' or '_'"));
    }
    let out = Arc::new(pm_zones::outputs::PullOutput::new(name));
    if s.outputs.get(&out.id()).is_some() {
        return Err(ApiError::new(
            StatusCode::CONFLICT,
            format!("pull output {name} exists"),
        ));
    }
    let info = out.info();
    s.pulls
        .lock()
        .expect("pulls lock")
        .insert(name.to_string(), out.clone());
    s.outputs.register(out.clone());
    s.create_zone(&format!("Stream {name}"), out);
    Ok(Json(info))
}

/// `GET /v1/outputs/pull/{name}/url`: the keyed URL to give a renderer or player by hand.
async fn pull_url(State(s): State<Arc<NodeState>>, Path(name): Path<String>) -> ApiResult<serde_json::Value> {
    if !s.pulls.lock().expect("pulls lock").contains_key(&name) {
        return Err(ApiError::not_found(format!("no pull output {name}")));
    }
    Ok(Json(serde_json::json!({ "url": s.pull_url(&name) })))
}

/// The live stream of a pull output. Waits up to 15 s for a zone to play into it; a listener that
/// leaves gives it back for the next one (see `pm_zones::outputs::pull`).
async fn pull_stream(
    State(s): State<Arc<NodeState>>,
    Path((name, file)): Path<(String, String)>,
    method: axum::http::Method,
    headers: HeaderMap,
    Query(q): Query<std::collections::HashMap<String, String>>,
) -> Result<Response, ApiError> {
    let keyed = q.get("k").is_some_and(|k| s.stream_key_ok(&name, k));
    if !keyed && !token_ok(&s, &headers, &q) {
        return Err(ApiError::new(
            StatusCode::UNAUTHORIZED,
            "missing or wrong stream key",
        ));
    }
    // WAV for everyone; DLNA LPCM (raw big-endian 16-bit) for TVs that take nothing else.
    let kind = match file.as_str() {
        "stream.wav" => pm_out_upnp::dlna::StreamKind::Wav {
            mime: "audio/wav".into(),
        },
        // OpenHome renderers (Linn): the same WAV, with the RIFF size 0 their pipeline reads as
        // "continuous stream" (see `wav_header_continuous`).
        "live.wav" => pm_out_upnp::dlna::StreamKind::Wav {
            mime: "audio/wav".into(),
        },
        // OpenHome renderers when the track fits: the same WAV as a file of fixed size, with its
        // Content-Length, so their own Pause holds (see `wav_header_sized_stream`).
        "sized.wav" => pm_out_upnp::dlna::StreamKind::Wav {
            mime: "audio/wav".into(),
        },
        "stream.l16" => pm_out_upnp::dlna::StreamKind::L16,
        // Slimproto players: the PCM without a header (squeezelite would play the header).
        "stream.pcm" => pm_out_upnp::dlna::StreamKind::Wav {
            mime: "application/octet-stream".into(),
        },
        _ => return Err(ApiError::not_found(format!("no stream {file}"))),
    };
    let out = s
        .pulls
        .lock()
        .expect("pulls lock")
        .get(&name)
        .cloned()
        .ok_or_else(|| ApiError::not_found(format!("no pull output {name}")))?;
    // DLNA devices check these on the answer (some ask with HEAD first).
    let dlna = [
        ("transferMode.dlna.org", "Streaming".to_string()),
        ("contentFeatures.dlna.org", kind.content_features()),
    ];
    let with_headers = |mime: String, body: axum::body::Body| {
        let mut r = (
            [
                (header::CONTENT_TYPE, mime),
                (header::CACHE_CONTROL, "no-store".to_string()),
                (header::ACCEPT_RANGES, "none".to_string()),
                // The body ends when the connection closes: say so, so a keep-alive client
                // (and hyper) treat this as the last answer on the connection.
                (header::CONNECTION, "close".to_string()),
            ],
            body,
        )
            .into_response();
        for (k, v) in &dlna {
            if let Ok(v) = axum::http::HeaderValue::from_str(v) {
                r.headers_mut().insert(*k, v);
            }
        }
        // An HTTP/1.0 answer: the body runs until the connection closes. HTTP/1.1 would send a
        // live stream chunked, and a Linn DS stops at once on a chunked stream (seen on the
        // owner's Klimax: it plays the same stream unchunked).
        *r.version_mut() = axum::http::Version::HTTP_10;
        r
    };
    // A HEAD must not take the live stream: the GET that follows needs it.
    if method == axum::http::Method::HEAD {
        let mime = match &kind {
            pm_out_upnp::dlna::StreamKind::L16 => "audio/L16".to_string(),
            k => k.mime(0, 0),
        };
        return Ok(with_headers(mime, axum::body::Body::empty()));
    }
    // `g`: the stream the node gave the renderer this URL for (pm-out-upnp); an older one's poll
    // never takes a newer stream
    let generation = q.get("g").and_then(|g| g.parse::<u64>().ok());
    let live = match out
        .take_stream_for(pm_zones::outputs::LISTENER_TIMEOUT, generation)
        .await
    {
        Ok(live) => live,
        Err(pm_zones::outputs::Missed::Timeout) => {
            return Err(ApiError::new(
                StatusCode::REQUEST_TIMEOUT,
                "no zone is playing into this stream; start playback to the Stream zone first",
            ))
        }
        // A renderer polling its old URI while a new stream opened: the fetch the node asks
        // for next gets it
        Err(pm_zones::outputs::Missed::Superseded) => {
            return Err(ApiError::new(
                StatusCode::GONE,
                "a new stream started after this request; ask again",
            ))
        }
    };
    let fmt = live.format;
    if kind == pm_out_upnp::dlna::StreamKind::L16 && fmt.bits != 16 {
        return Err(ApiError::new(
            StatusCode::CONFLICT,
            format!("LPCM is 16-bit; the zone plays {fmt}"),
        ));
    }
    let mime = kind.mime(fmt.rate_hz, fmt.channels);
    // Each listener gets its own header: one that leaves and comes back (OpenHome renderers do,
    // after reading the start) decodes the stream from the top again.
    let (wav, length) = match file.as_str() {
        "stream.wav" => (Some(pm_zones::outputs::wav_header(fmt)), None),
        "live.wav" => (Some(pm_zones::outputs::wav_header_continuous(fmt)), None),
        "sized.wav" => {
            let (h, len) = pm_zones::outputs::wav_header_sized_stream(fmt);
            (Some(h), Some(len))
        }
        _ => (None, None),
    };
    let to_l16 = kind == pm_out_upnp::dlna::StreamKind::L16;
    // The live stream moves into the body; when the listener goes, it is given back
    let chunks = futures_util::stream::unfold(live, |mut live| async move {
        live.next_chunk().await.map(|c| (c, live))
    });
    let mut l16 = L16::default();
    // A sized stream never goes past its length (the zone starts a fresh one before; this only
    // keeps the answer true to its Content-Length)
    let body = futures_util::stream::iter(wav)
        .chain(chunks.map(move |c| if to_l16 { l16.convert(&c) } else { c }))
        .scan(length.unwrap_or(u64::MAX), |left, mut c| {
            if *left == 0 {
                return futures_util::future::ready(None);
            }
            c.truncate(c.len().min(usize::try_from(*left).unwrap_or(usize::MAX)));
            *left -= c.len() as u64;
            futures_util::future::ready(Some(c))
        })
        .map(Ok::<_, std::convert::Infallible>);
    let mut r = with_headers(mime, axum::body::Body::from_stream(body));
    if let Some(len) = length {
        r.headers_mut()
            .insert(header::CONTENT_LENGTH, axum::http::HeaderValue::from(len));
    }
    Ok(r)
}

/// Little-endian 16-bit PCM to DLNA LPCM (big-endian), across chunk boundaries.
#[derive(Default)]
struct L16 {
    /// A sample's first byte when a chunk ended between its two bytes.
    carry: Option<u8>,
}

impl L16 {
    fn convert(&mut self, chunk: &[u8]) -> Vec<u8> {
        let mut data = chunk;
        let mut out = Vec::with_capacity(data.len() + 1);
        if let Some(lo) = self.carry.take() {
            match data.split_first() {
                Some((hi, rest)) => {
                    out.extend_from_slice(&[*hi, lo]);
                    data = rest;
                }
                None => {
                    self.carry = Some(lo);
                    return out;
                }
            }
        }
        let (pairs, rest) = data.as_chunks::<2>();
        if let [lo] = rest {
            self.carry = Some(*lo);
        }
        for [lo, hi] in pairs {
            out.extend_from_slice(&[*hi, *lo]);
        }
        out
    }
}

/// Discover outputs again (local devices, endpoints over mDNS and configured
/// addresses); new ones get a zone. Returns the full list.
/// `GET /v1/outputs/discovery`: scan the network and explain what was found,
/// for setups where a renderer does not show up (Docker bridge, VLANs).
async fn discovery_report(
    State(s): State<Arc<NodeState>>,
    Query(q): Query<std::collections::HashMap<String, String>>,
) -> ApiResult<DiscoveryReport> {
    let st = s.clone();
    let quick = q.get("quick").is_some_and(|v| v == "1");
    let (upnp, cast) = tokio::task::spawn_blocking(move || {
        if quick {
            return (Vec::new(), Vec::new());
        }
        let upnp = if st.upnp {
            pm_out_upnp::ssdp_probe(std::time::Duration::from_secs(3)).unwrap_or_default()
        } else {
            Vec::new()
        };
        let cast = if st.cast {
            pm_out_cast::discover(std::time::Duration::from_millis(1500))
        } else {
            Vec::new()
        };
        (upnp, cast)
    })
    .await
    .map_err(|e| ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let outputs = s.outputs.list();
    let have = |loc: &str| {
        outputs
            .iter()
            .any(|o| o.id.0.contains(loc) || o.name.contains(loc))
    };
    let local_address = local_address();
    let mut notes = Vec::new();
    if !s.upnp && !s.cast {
        notes.push("UPnP and Cast discovery are both off in the node config.".into());
    }
    if quick {
        // No scan ran; only the configuration notes below apply.
    } else if s.upnp && upnp.is_empty() {
        notes.push("No UPnP device answered at all: multicast is not reaching this node. In Docker, run the container on the host network; on a VLAN, SSDP must be relayed.".into());
    } else if s.upnp && !upnp.iter().any(|a| a.renderer) {
        notes.push("Devices answered, but none describes itself as a MediaRenderer with AVTransport. OpenHome-only players (some Linn models) need the UPnP AV service enabled, or are reached through another renderer front-end.".into());
    }
    if s.public_url.contains("0.0.0.0")
        || s.public_url.contains("127.0.0.1")
        || s.public_url.contains("localhost")
    {
        notes.push(format!(
            "public_url is {}, which renderers cannot reach; set [api] public_url (PM_PUBLIC_URL) to http://{}:7788.",
            s.public_url,
            local_address.as_deref().unwrap_or("<node ip>")
        ));
    }
    if s.public_url_guessed() {
        if let Some(why) = doubtful_guess(&s.public_url) {
            notes.push(format!(
                "public_url is not set, so the node picked {}, which {why}. If speakers stay silent, set [api] public_url (PM_PUBLIC_URL) to the address they reach.",
                s.public_url
            ));
        }
    }
    Ok(Json(DiscoveryReport {
        upnp: upnp
            .into_iter()
            .map(|a| DiscoveredUpnp {
                output: have(&a.location),
                from: a.from,
                location: a.location,
                server: a.server,
                friendly_name: a.friendly_name,
                renderer: a.renderer,
            })
            .collect(),
        cast: cast
            .into_iter()
            .map(|d| DiscoveredCast {
                id: d.id,
                friendly_name: d.friendly_name,
                addr: d.addr.to_string(),
            })
            .collect(),
        public_url: s.public_url.clone(),
        local_address,
        notes,
        upnp_enabled: s.upnp,
        cast_enabled: s.cast,
        saved_addresses: s.upnp_addresses(),
        saved_endpoints: s.endpoint_addresses(),
        node: s.name.clone(),
    }))
}

/// Why an address the node picked for itself may not be the one speakers reach: a VPN's or
/// another non-home address, or Docker's default bridge (the container is not on the host
/// network). `None` when it looks like a home network address.
fn doubtful_guess(public_url: &str) -> Option<&'static str> {
    let host = url::Url::parse(public_url).ok()?.host_str()?.to_string();
    let std::net::IpAddr::V4(ip) = host.parse().ok()? else {
        return None;
    };
    if ip.octets()[..2] == [172, 17] {
        Some("looks like Docker's bridge network (run the container on the host network)")
    } else if !ip.is_private() && !ip.is_loopback() && !ip.is_unspecified() {
        Some("is not a home network address (a VPN?)")
    } else {
        None
    }
}

/// The address this node talks to the network from.
fn local_address() -> Option<String> {
    std::net::UdpSocket::bind("0.0.0.0:0")
        .and_then(|u| u.connect("1.1.1.1:80").map(|_| u))
        .and_then(|u| u.local_addr())
        .ok()
        .map(|a| a.ip().to_string())
}

/// `POST /v1/outputs/upnp {address}`: add a renderer multicast cannot find
/// (another subnet or VLAN); the address is kept for the next start.
async fn add_upnp_output(
    State(s): State<Arc<NodeState>>,
    Json(body): Json<AddRenderer>,
) -> ApiResult<Vec<OutputInfo>> {
    let st = s.clone();
    let found = tokio::task::spawn_blocking(move || st.add_upnp_address(&body.address))
        .await
        .map_err(|e| ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .map_err(|e| {
            // Say which node tried: people run several (a cloud workspace has no
            // route to a renderer at home).
            let from = local_address().unwrap_or_else(|| "this node".into());
            ApiError::bad_request(format!("{e} (asked from {} at {from})", s.name))
        })?;
    let names: Vec<&str> = found.iter().map(|r| r.friendly_name.as_str()).collect();
    tracing::info!(?names, "renderers added by address");
    Ok(Json(s.outputs.list()))
}

/// `DELETE /v1/outputs/upnp/{address}`: stop re-adding it at start.
async fn remove_upnp_address(
    State(s): State<Arc<NodeState>>,
    Path(address): Path<String>,
) -> ApiResult<Vec<String>> {
    if !s.remove_upnp_address(&address) {
        return Err(ApiError::not_found(format!("{address} is not a saved address")));
    }
    Ok(Json(s.upnp_addresses()))
}

/// `POST /v1/outputs/endpoints {address, key}`: the desktop app registers its player at
/// `host:port` with the key it made; the outputs become zones, and the address and key are kept
/// for the next start (a new key for the same address replaces the old).
async fn add_endpoint(
    State(s): State<Arc<NodeState>>,
    Json(body): Json<AddEndpoint>,
) -> ApiResult<Vec<OutputInfo>> {
    let key = pm_stream::Key::parse(body.key.expose()).map_err(|_| {
        ApiError::bad_request(if body.key.is_empty() {
            "the player's key is missing: update the Odeion app".to_string()
        } else {
            "the player's key is not 32 to 256 visible ASCII characters".to_string()
        })
    })?;
    let st = s.clone();
    let address = body.address.clone();
    let outs = tokio::task::spawn_blocking(move || st.add_endpoint_address(&address, Some(&key)))
        .await
        .map_err(|e| ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .map_err(|e| {
            let from = local_address().unwrap_or_else(|| "this node".into());
            ApiError::bad_request(format!(
                "cannot reach the endpoint at {}: {e} (asked from {} at {from})",
                body.address, s.name
            ))
        })?;
    tracing::info!(address = %body.address, outputs = outs.len(), "endpoint added by address");
    Ok(Json(outs))
}

/// `DELETE /v1/outputs/endpoints/{address}`: forget it; its zones go too.
async fn remove_endpoint(
    State(s): State<Arc<NodeState>>,
    Path(address): Path<String>,
) -> ApiResult<Vec<String>> {
    if !s.remove_endpoint_address(&address) {
        return Err(ApiError::not_found(format!("{address} is not a saved endpoint")));
    }
    Ok(Json(s.endpoint_addresses()))
}

async fn rescan_outputs(State(s): State<Arc<NodeState>>) -> ApiResult<Vec<OutputInfo>> {
    let st = s.clone();
    let new = tokio::task::spawn_blocking(move || st.rescan_outputs())
        .await
        .map_err(|e| ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    if !new.is_empty() {
        tracing::info!(?new, "new outputs");
    }
    Ok(Json(s.outputs.list()))
}

// ---- handlers: zones ----

fn zone_of(s: &NodeState, id: &str) -> Result<Arc<Zone>, ApiError> {
    s.zones
        .get(&ZoneId::new(id))
        .ok_or_else(|| ApiError::not_found(format!("no zone {id}")))
}

async fn zones(
    State(s): State<Arc<NodeState>>,
    axum::Extension(c): axum::Extension<accounts::Caller>,
) -> Json<Vec<ZoneState>> {
    // A listener sees only the zones open to listeners
    Json(events::visible(&s, &c, s.zones.list()))
}

async fn create_zone(
    State(s): State<Arc<NodeState>>,
    axum::Extension(caller): axum::Extension<accounts::Caller>,
    Json(body): Json<CreateZone>,
) -> ApiResult<ZoneState> {
    if body.name.trim().is_empty() {
        return Err(ApiError::bad_request("zone name is empty"));
    }
    let mut ids: Vec<String> = body.outputs.clone();
    if let Some(o) = body.output.clone() {
        if !ids.contains(&o) {
            ids.insert(0, o);
        }
    }
    if ids.is_empty() {
        return Err(ApiError::bad_request("give `output` or `outputs`"));
    }
    // An app password groups speakers (and ungroups them, `delete_zone`): two or more
    if caller.role == accounts::Role::App && ids.iter().collect::<std::collections::HashSet<_>>().len() < 2 {
        return Err(ApiError::new(
            StatusCode::FORBIDDEN,
            "An app password groups two speakers or more; it doesn't add a zone.",
        ));
    }
    let output = zone_output(&s, body.name.trim(), &ids).await?;
    let zone = s.create_zone(body.name.trim(), output);
    Ok(Json(zone.state()))
}

/// The output a zone named `name` plays on: the one output, or a group of them (the first leads),
/// grouped by the speakers' own system when they all share one (Sonos, BluOS, HEOS), else in
/// sync by the node.
pub(crate) async fn zone_output(
    s: &NodeState,
    name: &str,
    ids: &[String],
) -> Result<Arc<dyn pm_zones::Output>, ApiError> {
    let mut members = Vec::with_capacity(ids.len());
    for id in ids {
        let out = s
            .outputs
            .get(&OutputId(id.clone()))
            .ok_or_else(|| ApiError::not_found(format!("no output {id}")))?;
        members.push(out);
    }
    // Sonos players only: Sonos groups them itself, in tight sync (pm_out_upnp::sonos).
    let sonos: Vec<Arc<pm_out_upnp::UpnpOutput>> = {
        let map = s.sonos.lock().expect("sonos lock");
        ids.iter()
            .filter_map(|id| map.get(&OutputId(id.clone())).cloned())
            .collect()
    };
    // HEOS players only (Denon, Marantz) whose CLI knows them all: HEOS groups them itself
    // (pm_out_upnp::heos).
    let mut heos: Vec<Arc<pm_out_upnp::UpnpOutput>> = {
        let map = s.heos.lock().expect("heos lock");
        ids.iter()
            .filter_map(|id| map.get(&OutputId(id.clone())).cloned())
            .collect()
    };
    // BluOS players only (Bluesound, NAD): BluOS groups them itself (pm_out_bluos)
    let bluos: Vec<Arc<pm_out_bluos::BluosOutput>> = {
        let map = s.bluos_players.lock().expect("bluos lock");
        ids.iter()
            .filter_map(|id| map.get(&OutputId(id.clone())).cloned())
            .collect()
    };
    if members.len() > 1 && heos.len() == ids.len() {
        let renderers: Vec<pm_out_upnp::Renderer> = heos.iter().map(|o| o.renderer().clone()).collect();
        let ok = tokio::task::spawn_blocking(move || {
            pm_out_upnp::heos::can_group(&renderers, pm_out_upnp::heos::CLI_PORT)
        })
        .await
        .unwrap_or(false);
        if !ok {
            heos.clear();
        }
    }
    let out: Arc<dyn pm_zones::Output> = if members.len() == 1 {
        members.remove(0)
    } else if sonos.len() == ids.len() {
        let mut sonos = sonos;
        let lead = sonos.remove(0);
        Arc::new(pm_out_upnp::sonos::SonosGroup::new(name, lead, sonos))
    } else if bluos.len() == ids.len() {
        let mut bluos = bluos;
        let lead = bluos.remove(0);
        Arc::new(pm_out_bluos::BluosGroup::new(name, lead, bluos))
    } else if heos.len() == ids.len() {
        let mut heos = heos;
        let lead = heos.remove(0);
        Arc::new(pm_out_upnp::heos::HeosGroup::new(name, lead, heos))
    } else {
        Arc::new(
            pm_zones::outputs::GroupOutput::new(name, members)
                .map_err(|e| ApiError::bad_request(e.to_string()))?,
        )
    };
    if ids.len() > 1 {
        // Every group of speakers: each keeps its own volume limit in it (the strictest holds)
        let members: Vec<OutputId> = ids.iter().map(|i| OutputId(i.clone())).collect();
        s.sources.note_group(&out.id(), &members);
    }
    Ok(out)
}

async fn zone(State(s): State<Arc<NodeState>>, Path(id): Path<String>) -> ApiResult<ZoneState> {
    Ok(Json(zone_of(&s, &id)?.state()))
}

async fn delete_zone(
    State(s): State<Arc<NodeState>>,
    Path(id): Path<String>,
    axum::Extension(caller): axum::Extension<accounts::Caller>,
) -> Result<StatusCode, ApiError> {
    // An app password ungroups (`access::Capability::ZoneGroup`): a zone on a group of speakers
    // (an output that is no one speaker), not a speaker's own zone or someone's listen-along group
    if caller.role == accounts::Role::App {
        let out = zone_of(&s, &id)?.output().id();
        // (the node's groups are `group:…`; Sonos, HEOS and BluOS groups `<kind>-group:…`)
        let group = out
            .0
            .split_once(':')
            .is_some_and(|(kind, _)| kind == "group" || kind.ends_with("-group"));
        if !group || presence::is_along(&s, &id) {
            return Err(ApiError::new(
                StatusCode::FORBIDDEN,
                "An app password ungroups speakers; it doesn't remove a zone.",
            ));
        }
    }
    // A listen-along group ends as when its host ends it: the music goes back, not lost
    if presence::delete_group(&s, &id).await? {
        return Ok(StatusCode::NO_CONTENT);
    }
    let zone = zone_of(&s, &id)?;
    zone.stop();
    s.remove_zone(zone.id());
    Ok(StatusCode::NO_CONTENT)
}

async fn queue(State(s): State<Arc<NodeState>>, Path(id): Path<String>) -> ApiResult<Vec<QueueItem>> {
    Ok(Json(zone_of(&s, &id)?.queue()))
}

/// The node's addresses for a request's links: its own, and the one the request came to (the
/// Host it named, under the scheme a proxy says): a cover link a page made under `localhost` or
/// a tunnel name is the node's too, and loses its sign-in like one under `public_url`.
fn origins_for(s: &NodeState, headers: &HeaderMap) -> Vec<String> {
    let mut out = s.node_origins();
    let h = |n: &str| {
        headers
            .get(n)
            .and_then(|v| v.to_str().ok())
            .map(|v| v.split(',').next().unwrap_or("").trim().to_string())
            .filter(|v| !v.is_empty())
    };
    if let Some(host) = h("x-forwarded-host").or_else(|| h("host")) {
        let proto = h("x-forwarded-proto").unwrap_or_else(|| "http".into());
        out.push(format!("{proto}://{host}"));
    }
    out
}

/// Items on their way into a zone lose the sign-ins their links carry: on the node's links
/// (`public_url`, the address this request came to) every one, on any other host's `/v1/` link
/// the node's own. The queue looks again (`pm_zones::Queue::add`); this look knows the request.
fn drop_sign_ins(s: &NodeState, headers: &HeaderMap, items: &mut [QueueItem]) {
    let origins = origins_for(s, headers);
    let known = accounts::KnownTokens::lazy(s);
    for it in items {
        it.without_node_token(&origins, &|t| known.has(t));
    }
}

/// Whether `asked` items fit `zone`'s queue (it holds [`pm_zones::MAX_QUEUE_ITEMS`] and no
/// more): in place of it (`replace`) or after it. Every route that puts a list into a queue asks
/// this first, before it stops, clears or looks up anything: a queue that took only part would
/// leave the rest lost (a move from This browser empties the browser's own queue after it).
pub(crate) fn queue_room(zone: &Zone, asked: usize, replace: bool) -> Result<(), ApiError> {
    let has = if replace { 0 } else { zone.queue_len() };
    if has.saturating_add(asked) > pm_zones::MAX_QUEUE_ITEMS {
        return Err(ApiError::bad_request(format!(
            "a queue holds at most {} items (this one has {has}, {asked} more were sent)",
            pm_zones::MAX_QUEUE_ITEMS
        )));
    }
    Ok(())
}

async fn enqueue(
    State(s): State<Arc<NodeState>>,
    Path(id): Path<String>,
    axum::Extension(caller): axum::Extension<accounts::Caller>,
    headers: HeaderMap,
    Json(body): Json<Enqueue>,
) -> ApiResult<Enqueued> {
    let zone = zone_of(&s, &id)?;
    // Before any song is looked up: a queue holds so many items and no more
    let asked = body.track_ids.len() + body.items.len();
    queue_room(&zone, asked, body.replace)?;
    let mut items = Vec::with_capacity(asked);
    {
        let lib = s.library.lock().expect("library poisoned");
        for tid in &body.track_ids {
            let track = lib
                .get_track(TrackId(*tid))?
                .ok_or_else(|| ApiError::not_found(format!("no track {tid}")))?;
            if let Some(item) = NodeSources::item_for(&track) {
                items.push(item);
            }
        }
    }
    for mut it in body.items {
        it.uri = it.uri.trim().to_string();
        if it.uri.is_empty() {
            return Err(ApiError::bad_request("item uri is empty"));
        }
        accounts::check_item(&caller, &mut it)?;
        items.push(it);
    }
    if items.is_empty() {
        return Err(ApiError::bad_request("nothing to enqueue"));
    }
    drop_sign_ins(&s, &headers, &mut items);
    // Who queued them, from the request, never from the body: their own accounts play them
    let by = accounts::Who::of(&caller).by();
    for it in &mut items {
        it.by = by.clone();
    }
    if body.replace {
        zone.stop();
        zone.clear();
    }
    let at = match body.at {
        QueueAt::End => Insert::End,
        QueueAt::Next => Insert::Next,
    };
    let ids = zone.enqueue(items, at);
    // A queue replaced with shuffle on plays shuffled, the order sent kept as the original one
    if body.replace {
        zone.shuffle_replacement();
    }
    if body.play {
        if body.replace {
            zone.play_index(0);
        } else {
            zone.play();
        }
    }
    settle(&zone).await;
    Ok(Json(Enqueued { ids }))
}

async fn clear_queue(
    State(s): State<Arc<NodeState>>,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    let zone = zone_of(&s, &id)?;
    zone.stop();
    zone.clear();
    settle(&zone).await;
    Ok(StatusCode::NO_CONTENT)
}

/// Wait (bounded) until the engine has applied every command sent so far,
/// so that a read right after a write sees the new state.
async fn settle(zone: &Zone) {
    let _ = tokio::time::timeout(std::time::Duration::from_secs(2), zone.wait_acked()).await;
}

async fn remove_item(
    State(s): State<Arc<NodeState>>,
    Path((id, item)): Path<(String, u64)>,
) -> Result<StatusCode, ApiError> {
    let zone = zone_of(&s, &id)?;
    if zone.remove(item) {
        settle(&zone).await;
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(ApiError::not_found(format!("no queue item {item}")))
    }
}

async fn move_item(
    State(s): State<Arc<NodeState>>,
    Path((id, item)): Path<(String, u64)>,
    Json(body): Json<MoveItem>,
) -> Result<StatusCode, ApiError> {
    let zone = zone_of(&s, &id)?;
    if zone.move_item(item, body.to_index) {
        settle(&zone).await;
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(ApiError::not_found(format!("no queue item {item}")))
    }
}

async fn transport(
    State(s): State<Arc<NodeState>>,
    Path(id): Path<String>,
    Json(body): Json<TransportAction>,
) -> ApiResult<ZoneState> {
    let zone = zone_of(&s, &id)?;
    match body {
        TransportAction::Play { index: Some(i) } => zone.play_index(i),
        TransportAction::Play { index: None } => zone.play(),
        TransportAction::Pause => zone.pause(),
        TransportAction::Toggle => zone.toggle_pause(),
        TransportAction::Stop => zone.stop(),
        TransportAction::Next => zone.next(),
        TransportAction::Previous => zone.previous(),
        TransportAction::Seek { position_ms } => zone.seek_ms(position_ms),
    }
    // Reply with the state after the engine applied the command (bounded wait).
    Ok(Json(
        tokio::time::timeout(std::time::Duration::from_secs(2), zone.wait_acked())
            .await
            .unwrap_or_else(|_| zone.state()),
    ))
}

async fn volume(
    State(s): State<Arc<NodeState>>,
    Path(id): Path<String>,
    Json(body): Json<SetVolume>,
) -> Result<StatusCode, ApiError> {
    let zone = zone_of(&s, &id)?;
    // The zone holds the request to its volume limit (DSP settings), as it does for every
    // controller (pm_zones::Zone::set_volume)
    zone.set_volume(body.volume).await?;
    announce::note_volume(zone.id(), &body.volume);
    Ok(StatusCode::NO_CONTENT)
}

/// The zone's level as its output reports it now, read from the device and published to every
/// controller (as the poll does). A controller that saw the level change on the device itself (a
/// phone's volume keys, read by the app on the phone) asks for it instead of setting it: a set
/// could land after a newer change on the device and undo it. `null` when the output can't say.
async fn read_volume(
    State(s): State<Arc<NodeState>>,
    Path(id): Path<String>,
) -> ApiResult<Option<pm_model::ZoneVolume>> {
    let zone = zone_of(&s, &id)?;
    let read = tokio::time::timeout(std::time::Duration::from_secs(4), zone.refresh_volume()).await;
    Ok(Json(read.ok().flatten()))
}

async fn repeat(
    State(s): State<Arc<NodeState>>,
    Path(id): Path<String>,
    Json(body): Json<SetRepeat>,
) -> Result<StatusCode, ApiError> {
    let zone = zone_of(&s, &id)?;
    zone.set_repeat(body.repeat);
    settle(&zone).await;
    Ok(StatusCode::NO_CONTENT)
}

/// Library radio: keep the zone playing with similar music after the last item.
async fn radio(
    State(s): State<Arc<NodeState>>,
    Path(id): Path<String>,
    Json(body): Json<SetRadio>,
) -> Result<StatusCode, ApiError> {
    let zone = zone_of(&s, &id)?;
    zone.set_radio(body.on);
    settle(&zone).await;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Debug, serde::Deserialize)]
struct SetCrossfade {
    /// Seconds, 0 (off) to 12.
    seconds: f64,
}

/// `PUT /v1/zones/{id}/crossfade {seconds}`: fade each track into the next (0 = off). Songs of
/// the same album following each other stay gapless.
async fn crossfade(
    State(s): State<Arc<NodeState>>,
    Path(id): Path<String>,
    Json(body): Json<SetCrossfade>,
) -> Result<StatusCode, ApiError> {
    let zone = zone_of(&s, &id)?;
    if !(0.0..=12.0).contains(&body.seconds) {
        return Err(ApiError::bad_request("crossfade is 0 to 12 seconds"));
    }
    let ms = (body.seconds * 1000.0).round() as u64;
    let key = state::crossfade_key(zone.id());
    {
        let lib = s.library.lock().expect("library poisoned");
        if ms == 0 {
            lib.delete_setting(&key)?;
        } else {
            lib.set_setting(&key, &ms.to_string())?;
        }
    }
    zone.set_crossfade_ms(ms);
    settle(&zone).await;
    Ok(StatusCode::NO_CONTENT)
}

/// `POST /v1/zones/{id}/shuffle` (older clients): shuffle mode on, the playing item untouched;
/// when it is on already, the upcoming items are shuffled again.
async fn shuffle_queue(
    State(s): State<Arc<NodeState>>,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    let zone = zone_of(&s, &id)?;
    zone.shuffle_upcoming();
    settle(&zone).await;
    Ok(StatusCode::NO_CONTENT)
}

/// `PUT /v1/zones/{id}/shuffle {shuffle}`: shuffle mode on (the upcoming items are shuffled, the
/// current one stays) or off (every item back in its original order, the current one in its
/// place). The zone's state.
async fn set_shuffle(
    State(s): State<Arc<NodeState>>,
    Path(id): Path<String>,
    Json(body): Json<SetShuffle>,
) -> ApiResult<ZoneState> {
    let zone = zone_of(&s, &id)?;
    zone.set_shuffle(body.shuffle);
    settle(&zone).await;
    Ok(Json(zone.state()))
}

fn sleep_view(s: &NodeState, zone: &ZoneId) -> SleepView {
    let ends_at_ms = s.sleep_ends_at(zone);
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    SleepView {
        ends_at_ms,
        remaining_ms: ends_at_ms.map(|e| e.saturating_sub(now)),
    }
}

/// The zone's sleep timer, if any.
async fn sleep_get(State(s): State<Arc<NodeState>>, Path(id): Path<String>) -> ApiResult<SleepView> {
    let zone = zone_of(&s, &id)?;
    Ok(Json(sleep_view(&s, zone.id())))
}

/// Pause the zone after `seconds` (`null` cancels any timer).
async fn sleep_set(
    State(s): State<Arc<NodeState>>,
    Path(id): Path<String>,
    Json(body): Json<SetSleep>,
) -> ApiResult<SleepView> {
    let zone = zone_of(&s, &id)?;
    if body.seconds == Some(0) {
        return Err(ApiError::bad_request(
            "seconds must be positive, or null to cancel",
        ));
    }
    s.set_sleep(zone.clone(), body.seconds);
    Ok(Json(sleep_view(&s, zone.id())))
}

/// Move the queue and playback position to another zone. The target stops
/// whatever it was doing; the source is left stopped and empty.
async fn transfer(
    State(s): State<Arc<NodeState>>,
    axum::Extension(c): axum::Extension<accounts::Caller>,
    Path(id): Path<String>,
    Json(body): Json<TransferRequest>,
) -> ApiResult<ZoneState> {
    // The middleware checked the zone in the path; the target needs the same right
    if !accounts::may_use_zone(&s, &c, &body.to) {
        return Err(accounts::refused(
            "the owner has not opened that zone to listeners",
        ));
    }
    if body.to == id {
        return Err(ApiError::bad_request("cannot transfer a zone to itself"));
    }
    let from = zone_of(&s, &id)?;
    let to = zone_of(&s, &body.to)?;
    let st = from.state();
    if st.queue.is_empty() {
        return Err(ApiError::bad_request("nothing to transfer: the queue is empty"));
    }
    let index = st.now_playing.as_ref().map(|n| n.index).or(st.queue_index);
    let position_ms = st.now_playing.as_ref().map(|n| n.position_ms).unwrap_or(0);
    // The shuffle mode with its original order, read before the source is emptied
    let shuffle = kept_shuffle(&from);
    from.stop();
    from.clear();
    presence::played(&s, &body.to, &c);
    // The target is in the hands of whoever moved the music there, as the source is (listens.rs)
    // (an app password's are the owner's, as in `auth`)
    listens::zone_changed_by(&s, &body.to, accounts::Who::of(&c) == accounts::Who::Owner);
    let moved = load_zone(
        &to,
        st.queue,
        index,
        position_ms,
        st.transport,
        st.repeat,
        shuffle,
        st.radio,
    )
    .await;
    let _ = tokio::time::timeout(std::time::Duration::from_secs(5), from.wait_acked()).await;
    Ok(Json(moved))
}

/// The shuffle mode a loaded queue gets ([`load_zone`]).
pub(crate) enum LoadShuffle {
    Off,
    /// On, fresh: `index`'s item first, every other item shuffled after it.
    On,
    /// As it was on the zone it comes from: the items are already in play order, and this is
    /// their original order ([`Zone::shuffle_order`]), so shuffle off still puts them back.
    Kept(Vec<usize>),
}

/// `zone`'s shuffle mode for a queue moved from it, with its original order.
pub(crate) fn kept_shuffle(zone: &Zone) -> LoadShuffle {
    match zone.shuffle_order() {
        Some(order) => LoadShuffle::Kept(order),
        None => LoadShuffle::Off,
    }
}

/// Fills `to` the way a move leaves it: stopped and emptied, then `queue`, `repeat`, `shuffle`
/// and `radio`; `index` plays from `position_ms` (paused when the source was paused; nothing
/// starts when it was stopped). Seeking needs the track open, so it waits for that (bounded)
/// first.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn load_zone(
    to: &Zone,
    queue: Vec<QueueItem>,
    index: Option<usize>,
    position_ms: u64,
    transport: pm_zones::Transport,
    repeat: pm_zones::Repeat,
    shuffle: LoadShuffle,
    radio: bool,
) -> ZoneState {
    to.stop();
    to.clear();
    to.set_repeat(repeat);
    to.set_radio(radio);
    // Off until the items are in: a queue loaded in order with shuffle off is in order
    to.set_shuffle(false);
    let mut events = to.subscribe();
    // (what the queue took: it holds so many items and no more)
    let len = to.enqueue(queue, Insert::End).len();
    let mut index = index.filter(|i| *i < len);
    match shuffle {
        LoadShuffle::Off => {}
        LoadShuffle::On => {
            to.shuffle_from(index);
            index = index.map(|_| 0);
        }
        LoadShuffle::Kept(order) => to.restore_shuffle(&order),
    }
    if let (Some(i), true) = (index, transport != pm_zones::Transport::Stopped) {
        to.play_index(i);
        let started = tokio::time::timeout(std::time::Duration::from_secs(10), async {
            loop {
                match events.recv().await {
                    Ok(pm_zones::ZoneEvent::TrackStarted { .. }) => break true,
                    Ok(pm_zones::ZoneEvent::Error { .. }) | Err(_) => break false,
                    Ok(_) => {}
                }
            }
        })
        .await
        .unwrap_or(false);
        if started {
            if position_ms > 1_000 {
                to.seek_ms(position_ms);
            }
            if transport == pm_zones::Transport::Paused {
                to.pause();
            }
        }
    }
    tokio::time::timeout(std::time::Duration::from_secs(5), to.wait_acked())
        .await
        .unwrap_or_else(|_| to.state())
}

/// `POST /v1/zones/{id}/load`: a player the node does not run (This browser) moves its queue
/// here. The zone stops and empties, takes the items, repeat and radio, and plays `index` from
/// `position_ms`; paused stays paused. A listener needs the zone open to listeners (the
/// middleware checks the path).
async fn load(
    State(s): State<Arc<NodeState>>,
    Path(id): Path<String>,
    axum::Extension(caller): axum::Extension<accounts::Caller>,
    headers: HeaderMap,
    Json(body): Json<LoadRequest>,
) -> ApiResult<ZoneState> {
    let mut body = body;
    if body.items.is_empty() {
        return Err(ApiError::bad_request("nothing to move: the queue is empty"));
    }
    let to = zone_of(&s, &id)?;
    // In place of the zone's queue, whole or not at all: nothing is stopped or cleared for a
    // list the queue can't hold (the browser empties its own queue after a move)
    queue_room(&to, body.items.len(), true)?;
    drop_sign_ins(&s, &headers, &mut body.items);
    for it in &mut body.items {
        it.uri = it.uri.trim().to_string();
        if it.uri.is_empty() {
            return Err(ApiError::bad_request("item uri is empty"));
        }
        accounts::check_item(&caller, it)?;
        // Moved by this person from their browser: played with their accounts (the owner's
        // whole: no fetch limit left from someone else's queue either)
        it.by = accounts::Who::of(&caller).by();
        if caller.role == accounts::Role::Owner {
            it.limit = None;
        }
    }
    Ok(Json(
        load_zone(
            &to,
            body.items,
            body.index,
            body.position_ms,
            body.transport,
            body.repeat,
            if body.shuffle {
                LoadShuffle::On
            } else {
                LoadShuffle::Off
            },
            body.radio,
        )
        .await,
    ))
}

// ---- handlers: DSP per zone ----

/// The pipeline settings for the zone's output (EQ, headroom, software
/// volume, resampling policy). Changes apply at the next track start.
async fn dsp_get(
    State(s): State<Arc<NodeState>>,
    Path(id): Path<String>,
) -> ApiResult<pm_audio::PipelineConfig> {
    let zone = zone_of(&s, &id)?;
    Ok(Json(s.sources.dsp_for(&zone.output().id())))
}

async fn dsp_put(
    State(s): State<Arc<NodeState>>,
    Path(id): Path<String>,
    Json(cfg): Json<pm_audio::PipelineConfig>,
) -> ApiResult<pm_audio::PipelineConfig> {
    let zone = zone_of(&s, &id)?;
    for b in &cfg.eq {
        // Validate against a common rate; the real plan validates again per track.
        b.validate(48_000).map_err(ApiError::bad_request)?;
    }
    if cfg.headroom_db.is_some_and(|d| !(-30.0..=0.0).contains(&d)) {
        return Err(ApiError::bad_request("headroom_db must be between -30 and 0"));
    }
    if cfg.volume_db.is_some_and(|d| !(-90.0..=0.0).contains(&d)) {
        return Err(ApiError::bad_request("volume_db must be between -90 and 0"));
    }
    if cfg.max_volume_db.is_some_and(|d| !(-90.0..=0.0).contains(&d)) {
        return Err(ApiError::bad_request("max_volume_db must be between -90 and 0"));
    }
    let mut cfg = cfg;
    if let (Some(v), Some(max)) = (cfg.volume_db, cfg.max_volume_db) {
        if v > max {
            cfg.volume_db = Some(max);
        }
    }
    s.sources.set_dsp(&zone.output().id(), Some(cfg.clone()))?;
    Ok(Json(cfg))
}

async fn dsp_reset(State(s): State<Arc<NodeState>>, Path(id): Path<String>) -> Result<StatusCode, ApiError> {
    let zone = zone_of(&s, &id)?;
    s.sources.set_dsp(&zone.output().id(), None)?;
    Ok(StatusCode::NO_CONTENT)
}

// ---- handlers: library ----

async fn library_stats(State(s): State<Arc<NodeState>>) -> ApiResult<pm_library::LibraryStats> {
    Ok(Json(s.library.lock().expect("library poisoned").stats()?))
}

async fn tracks(State(s): State<Arc<NodeState>>, Query(p): Query<Page>) -> ApiResult<Tracks> {
    let limit = p.limit.unwrap_or(100).min(1000);
    let tracks = s
        .library
        .lock()
        .expect("library poisoned")
        .list_tracks(p.after.map(TrackId), limit)?;
    Ok(Json(Tracks::plain(tracks)))
}

async fn track(State(s): State<Arc<NodeState>>, Path(id): Path<i64>) -> ApiResult<pm_model::Track> {
    let t = s
        .library
        .lock()
        .expect("library poisoned")
        .get_track(TrackId(id))?
        .ok_or_else(|| ApiError::not_found(format!("no track {id}")))?;
    Ok(Json(t))
}

/// Add Pinakes's links after the direct lookups, skipping pages already
/// listed (same URL) and services the node already answered for.
fn merge_versions(out: &mut Vec<Version>, extra: Vec<Version>) {
    for v in extra {
        let dup = out.iter().any(|o| {
            o.url.trim_end_matches('/') == v.url.trim_end_matches('/')
                || (o.source == v.source && matches!(o.source.as_str(), "tidal" | "deezer" | "spotify"))
        });
        if !dup {
            out.push(v);
        }
    }
}

/// Where else a library track streams from (TIDAL and Deezer by ISRC, then
/// every service Pinakes lists for it); empty when nothing is known.
pub(crate) async fn track_versions(
    State(s): State<Arc<NodeState>>,
    Path(id): Path<i64>,
) -> ApiResult<Vec<Version>> {
    let track = s
        .library
        .lock()
        .expect("library poisoned")
        .get_track(TrackId(id))?
        .ok_or_else(|| ApiError::not_found(format!("no track {id}")))?;
    // The canonical identity first (tags, fingerprint or the owner's
    // confirmation), then the raw tag ids.
    let canonical = track.canonical.as_ref().map(|(u, _)| u.clone());
    let hub = canonical
        .as_ref()
        .filter(|u| {
            matches!(
                u,
                pm_model::MusicUri::Recording { .. } | pm_model::MusicUri::Isrc { .. }
            )
        })
        .map(|u| u.to_url());
    let pinakes_key = match &canonical {
        Some(pm_model::MusicUri::Recording { mbid }) => Some(format!("mbid:{mbid}")),
        Some(pm_model::MusicUri::Isrc { isrc }) => Some(format!("isrc:{isrc}")),
        _ => None,
    }
    .or_else(|| track.recording_mbid.as_ref().map(|m| format!("mbid:{m}")))
    .or_else(|| track.isrc.as_ref().map(|i| format!("isrc:{i}")));
    let key = format!("pinakes.version.track.{id}");
    let (t, d, sp, p) = tokio::join!(
        tidal::track_versions(&s, &track),
        deezer::track_versions(&s, &track),
        spotify::track_versions(&s, &track),
        async {
            // The MusicBrainz URL's own card (a hub), then everything tagged with the key.
            let mut v = match &hub {
                Some(h) => discover::hub_versions(&s, h).await,
                None => Vec::new(),
            };
            if let Some(k) = pinakes_key {
                let by_key = discover::also_on_versions(&s, &k, &key).await;
                merge_versions(&mut v, by_key);
            }
            v
        }
    );
    let mut out = t.unwrap_or_default();
    out.extend(d.unwrap_or_default());
    out.extend(sp.unwrap_or_default());
    merge_versions(&mut out, p);
    Ok(Json(out))
}

/// Where else a library album streams from (TIDAL and Deezer by UPC).
pub(crate) async fn album_versions(
    State(s): State<Arc<NodeState>>,
    Path(id): Path<i64>,
) -> ApiResult<Vec<Version>> {
    let album = s
        .library
        .lock()
        .expect("library poisoned")
        .get_album(id)?
        .ok_or_else(|| ApiError::not_found(format!("no album {id}")))?;
    let pinakes_key = album
        .release_mbid
        .as_ref()
        .or(album.release_group_mbid.as_ref())
        .map(|m| format!("mbid:{m}"))
        .or_else(|| album.upc.as_ref().map(|u| format!("upc:{u}")));
    let key = format!("pinakes.version.album.{id}");
    let (t, d, sp, p) = tokio::join!(
        tidal::album_versions(&s, &album),
        deezer::album_versions(&s, &album),
        spotify::album_versions(&s, &album),
        async {
            match pinakes_key {
                Some(k) => discover::also_on_versions(&s, &k, &key).await,
                None => Vec::new(),
            }
        }
    );
    let mut out = t.unwrap_or_default();
    out.extend(d.unwrap_or_default());
    out.extend(sp.unwrap_or_default());
    merge_versions(&mut out, p);
    Ok(Json(out))
}

/// Lyrics from the file's tags or a sidecar `.lrc`; 404 when the track has none.
async fn lyrics(State(s): State<Arc<NodeState>>, Path(id): Path<i64>) -> ApiResult<Lyrics> {
    let (found, track) = {
        let lib = s.library.lock().expect("library poisoned");
        (lib.lyrics(TrackId(id))?, lib.get_track(TrackId(id))?)
    };
    // None in the file or beside it: LRCLIB, by title, artist, album and length
    let found = match (found, track) {
        (Some(f), _) => Some(f),
        (None, Some(t)) => {
            let artist = t.artists.first().map(|a| a.name.clone()).unwrap_or_default();
            let album = t.album.as_ref().map(|a| a.title.clone());
            lrclib::find(&s, &t.title, &artist, album.as_deref(), t.duration_ms)
                .await
                .map(|(text, _)| (text, "lrclib".to_string()))
        }
        _ => None,
    };
    let Some((text, source)) = found else {
        return Err(ApiError::not_found(format!("no lyrics for track {id}")));
    };
    let (lines, synced) = pm_library::lyrics::parse(&text);
    Ok(Json(Lyrics {
        track_id: id,
        source,
        synced,
        text,
        lines,
    }))
}

/// The track's audio file as stored, for a controller that plays locally
/// (the web UI's browser zone). Served only to authenticated callers of this node; never a
/// public URL (constraint 2).
/// A whole body or the byte range a browser asked for (seeking, resuming),
/// with `Accept-Ranges` so the browser knows seeking is possible.
/// `bytes` is shared, not copied: a range is a slice of it.
pub(crate) fn ranged_response(headers: &HeaderMap, bytes: impl Into<bytes::Bytes>, mime: &str) -> Response {
    let bytes: bytes::Bytes = bytes.into();
    let total = bytes.len();
    let range = headers
        .get(header::RANGE)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("bytes="))
        .and_then(|v| {
            let (a, b) = v.split_once('-')?;
            let start: usize = a.parse().ok()?;
            let end: usize = if b.is_empty() {
                total.saturating_sub(1)
            } else {
                b.parse().ok()?
            };
            (start < total && start <= end).then_some((start, end.min(total - 1)))
        });
    let mut resp = match range {
        Some((start, end)) => (
            StatusCode::PARTIAL_CONTENT,
            [
                (header::CONTENT_TYPE, mime.to_string()),
                (header::CONTENT_RANGE, format!("bytes {start}-{end}/{total}")),
            ],
            axum::body::Body::from(bytes.slice(start..=end)),
        )
            .into_response(),
        None => (
            StatusCode::OK,
            [(header::CONTENT_TYPE, mime.to_string())],
            axum::body::Body::from(bytes),
        )
            .into_response(),
    };
    resp.headers_mut().insert(
        header::ACCEPT_RANGES,
        axum::http::HeaderValue::from_static("bytes"),
    );
    resp
}

async fn track_file(
    State(s): State<Arc<NodeState>>,
    Path(id): Path<i64>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    let t = s
        .library
        .lock()
        .expect("library poisoned")
        .get_track(TrackId(id))?
        .ok_or_else(|| ApiError::not_found(format!("no track {id}")))?;
    let mime = match t.file.container.as_str() {
        "flac" => "audio/flac",
        "wav" => "audio/wav",
        "aiff" => "audio/aiff",
        "isomp4" | "mp4" | "m4a" => "audio/mp4",
        "dsf" => "audio/x-dsf",
        "dff" => "audio/x-dff",
        "mp3" => "audio/mpeg",
        "aac" => "audio/aac",
        "vorbis" => "audio/ogg",
        _ => "application/octet-stream",
    };
    let name = t
        .file
        .path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| format!("track-{id}"));
    downloads::stream_file(&s, id, &t.file, &headers, mime, Some(("inline", &name))).await
}

async fn search(State(s): State<Arc<NodeState>>, Query(q): Query<SearchQuery>) -> ApiResult<SearchResults> {
    let limit = q.limit.unwrap_or(50).min(500);
    let needle = q.q.trim().to_lowercase();
    // Streaming services answer alongside the library (their part is bounded in time).
    let sources = if q.sources.unwrap_or(1) != 0 && !needle.is_empty() {
        streaming::unified_search(&s, q.q.trim(), 12).await
    } else {
        Vec::new()
    };
    let lib = s.library.lock().expect("library poisoned");
    let tracks = lib.search_tracks(&q.q, limit)?;
    let albums = if needle.is_empty() {
        Vec::new()
    } else {
        lib.search_albums(&q.q, 24)?
    };
    let contains = |name: &str| !needle.is_empty() && name.to_lowercase().contains(&needle);
    let artists = lib
        .list_artists()?
        .into_iter()
        .filter(|a| contains(&a.name))
        .take(24)
        .collect();
    let genres = lib
        .list_genres()?
        .into_iter()
        .filter(|g| contains(&g.name))
        .take(12)
        .collect();
    Ok(Json(SearchResults {
        tracks,
        albums,
        artists,
        genres,
        sources,
    }))
}

async fn artists(State(s): State<Arc<NodeState>>) -> ApiResult<Vec<pm_library::ArtistSummary>> {
    let lib = s.library.lock().expect("library poisoned");
    let mut all = lib.list_artists()?;
    // Songs without any artist, as "Unknown artist" at the end (its page is `/artist/lib-0`)
    all.extend(lib.unknown_artist()?);
    Ok(Json(all))
}

async fn artist(State(s): State<Arc<NodeState>>, Path(id): Path<i64>) -> ApiResult<ArtistDetail> {
    let lib = s.library.lock().expect("library poisoned");
    let artist = lib
        .get_artist(id)?
        .ok_or_else(|| ApiError::not_found(format!("no artist {id}")))?;
    let albums = lib.artist_albums(id)?;
    Ok(Json(ArtistDetail { artist, albums }))
}

/// Biography, links and whether a portrait is cached (fetched on first request).
async fn artist_info_handler(
    State(s): State<Arc<NodeState>>,
    Path(id): Path<i64>,
    Query(q): Query<item::LangQuery>,
    headers: HeaderMap,
) -> ApiResult<ArtistInfo> {
    if s.library
        .lock()
        .expect("library poisoned")
        .get_artist(id)?
        .is_none()
    {
        return Err(ApiError::not_found(format!("no artist {id}")));
    }
    let lang = about::language(q.lang.as_deref(), &headers);
    Ok(Json(artist_info::artist_info(&s, id, &lang).await?))
}

async fn artist_image(State(s): State<Arc<NodeState>>, Path(id): Path<i64>) -> Result<Response, ApiError> {
    let path = artist_info::image_path(&s, id);
    // The page's own fetch (in the reader's language) brings the portrait: wait for it rather
    // than fetch it all again
    for _ in 0..50 {
        if path.is_file() || !artist_info::fetching(id) {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    }
    if !path.is_file() {
        // The first request may have to fetch it.
        let info = artist_info::artist_info(&s, id, "en").await?;
        if !info.image {
            // Else the picture of the service artist the owner matched it to
            let matched = s
                .library
                .lock()
                .expect("library poisoned")
                .get_setting(&format!("artist.picture.{id}"))
                .ok()
                .flatten();
            let got = match matched {
                Some(url) => match s.http_client().get(&url).send().await {
                    Ok(r) if r.status().is_success() => capped::bytes(r, art::RELAYED_PICTURE_MAX).await.ok(),
                    _ => None,
                },
                None => None,
            };
            let Some(bytes) = got.filter(|b| b.len() > 100) else {
                return Err(ApiError::not_found(format!("no portrait for artist {id}")));
            };
            let _ = tokio::fs::create_dir_all(&s.art_dir).await;
            tokio::fs::write(&path, &bytes)
                .await
                .map_err(|e| ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
        }
    }
    let bytes = tokio::fs::read(&path)
        .await
        .map_err(|e| ApiError::not_found(format!("portrait for artist {id}: {e}")))?;
    art::picture(bytes, "private, max-age=86400")
}

async fn artist_tracks(State(s): State<Arc<NodeState>>, Path(id): Path<i64>) -> ApiResult<Tracks> {
    let songs = s.library.lock().expect("library poisoned").artist_songs(id)?;
    Ok(Json(Tracks::songs(songs)))
}

async fn albums(State(s): State<Arc<NodeState>>, Query(p): Query<Page>) -> ApiResult<Vec<pm_model::Album>> {
    let limit = p.limit.unwrap_or(100).min(1000);
    Ok(Json(
        s.library
            .lock()
            .expect("library poisoned")
            .list_albums_focused(p.focus, p.after, limit)?,
    ))
}

/// Release groups MusicBrainz lists for the library's artists from the last
/// year; a background refresh starts when any artist's list is stale.
async fn new_releases(State(s): State<Arc<NodeState>>) -> ApiResult<NewReleasesPage> {
    Ok(Json(new_releases::page(&s)?))
}

/// Force a refresh of stale artists now (one MusicBrainz request per second).
async fn new_releases_refresh(State(s): State<Arc<NodeState>>) -> ApiResult<NewReleasesPage> {
    if s.contact().is_empty() {
        return Err(ApiError::bad_request(
            "set a contact first (Settings): MusicBrainz requires one in the User-Agent",
        ));
    }
    new_releases::kick_refresh(s.clone());
    Ok(Json(new_releases::page(&s)?))
}

/// The best file format per album, for badges and Focus in clients.
async fn album_formats(State(s): State<Arc<NodeState>>) -> ApiResult<Vec<pm_library::AlbumFormat>> {
    Ok(Json(s.library.lock().expect("library poisoned").album_formats()?))
}

async fn genres(State(s): State<Arc<NodeState>>) -> ApiResult<Vec<pm_library::GenreSummary>> {
    Ok(Json(s.library.lock().expect("library poisoned").list_genres()?))
}

async fn genre(State(s): State<Arc<NodeState>>, Path(id): Path<i64>) -> ApiResult<GenreDetail> {
    let lib = s.library.lock().expect("library poisoned");
    let genre = lib
        .get_genre(id)?
        .ok_or_else(|| ApiError::not_found(format!("no genre {id}")))?;
    Ok(Json(GenreDetail {
        albums: lib.genre_albums(id)?,
        tracks: lib.genre_tracks(id, 500)?,
        genre,
    }))
}

async fn album_tracks(State(s): State<Arc<NodeState>>, Path(id): Path<i64>) -> ApiResult<Tracks> {
    let songs = s.library.lock().expect("library poisoned").album_songs(id)?;
    Ok(Json(Tracks::songs(songs)))
}

async fn history(State(s): State<Arc<NodeState>>, Query(p): Query<Limit>) -> ApiResult<History> {
    let limit = p.limit.unwrap_or(100).min(1000);
    let plays = s.library.lock().expect("library poisoned").history(limit)?;
    Ok(Json(History { plays }))
}

/// Clients that play by themselves (the native app's local zone, a browser)
/// report their plays here; node zones are recorded automatically.
async fn record_play(State(s): State<Arc<NodeState>>, Json(req): Json<RecordPlay>) -> ApiResult<History> {
    let lib = s.library.lock().expect("library poisoned");
    if lib.get_track(TrackId(req.track_id))?.is_none() {
        return Err(ApiError::not_found(format!("track {}", req.track_id)));
    }
    match req.played_at {
        Some(at) => {
            lib.record_play_at(TrackId(req.track_id), req.zone.as_deref(), at)?;
        }
        None => {
            lib.record_play(TrackId(req.track_id), req.zone.as_deref())?;
        }
    }
    let plays = lib.history(1)?;
    Ok(Json(History { plays }))
}

async fn home(State(s): State<Arc<NodeState>>, Query(p): Query<Limit>) -> ApiResult<HomePage> {
    let limit = p.limit.unwrap_or(12).min(100);
    let lib = s.library.lock().expect("library poisoned");
    let favs = bookmarks::collect(&lib)?;
    let bookmarked_albums: Vec<_> = favs.albums.into_iter().take(limit as usize).collect();
    // Forgotten bookmarks: bookmarked, then not played for two months
    let cutoff = pm_library::db::now_unix() - 60 * 86_400;
    let mut forgotten: Vec<(i64, pm_model::Track)> = favs
        .tracks
        .into_iter()
        .filter_map(|t| {
            let last =
                t.id.and_then(|id| lib.last_played(id).ok().flatten())
                    .unwrap_or(0);
            (last < cutoff).then_some((last, t))
        })
        .collect();
    forgotten.sort_by_key(|(last, _)| *last);
    let forgotten = forgotten
        .into_iter()
        .map(|(_, t)| t)
        .take(limit as usize)
        .collect();
    Ok(Json(HomePage {
        forgotten,
        favourite_albums: bookmarked_albums.clone(),
        bookmarked_albums,
        recently_added: lib.recent_albums(0, limit)?,
        recently_played: lib.recently_played_albums(limit)?,
        most_played_albums: lib.most_played_albums(limit)?,
        most_played: lib.most_played_tracks(limit)?,
        history: lib.history(limit)?,
        stats: lib.stats()?,
    }))
}

async fn folders(State(s): State<Arc<NodeState>>) -> ApiResult<Vec<(String, bool)>> {
    Ok(Json(folder_list(&s).await?))
}

fn settings_view(s: &NodeState) -> NodeSettingsView {
    let copy_rate = files::copy_rate(s);
    // One lock, taken and released here: a guard in a struct literal lives until the end of the
    // whole expression, so a second `lock()` in another field would wait for ever
    let (
        jukebox_zone,
        dev_setting,
        genre_tags,
        prefer_full,
        first_steps_hidden,
        sources_policy,
        api_token_exposed_at,
    ) = {
        let lib = s.library.lock().expect("library poisoned");
        let on = |k: &str| lib.get_setting(k).ok().flatten().is_some_and(|v| v == "1");
        (
            lib.get_setting(subsonic::JUKEBOX_ZONE_KEY)
                .ok()
                .flatten()
                .filter(|z| !z.is_empty()),
            on(DEV_OUTPUTS_KEY),
            tags::genre_tags_on(&lib),
            on(item::PREFER_FULL_KEY),
            on(first_steps::HIDDEN_KEY),
            accounts::source_policy(&lib),
            accounts::api_token_exposed_at(&lib, s.token().as_deref()),
        )
    };
    NodeSettingsView {
        contact: s.contact(),
        discovery: s.discovery().is_some(),
        jukebox_zone,
        copy_rate,
        dev_outputs: std::env::var("PM_DEV_OUTPUTS").is_ok_and(|v| v == "1") || dev_setting,
        genre_tags,
        prefer_full,
        first_steps_hidden,
        config_folders: config_folders(s),
        token_replaceable: s.token_file().is_some(),
        api_token_exposed_at,
        sources_policy,
    }
}

/// Show developer outputs (the Null output) in the zone lists (STEWARD-UX-INSTANCES item 5).
const DEV_OUTPUTS_KEY: &str = "ui.dev_outputs";

async fn node_settings(State(s): State<Arc<NodeState>>) -> ApiResult<NodeSettingsView> {
    Ok(Json(settings_view(&s)))
}

/// `POST /v1/node/settings/backup`: write the owner's settings to
/// `/priv/pubky.music/settings.json` now (also done after every sync).
async fn settings_backup_now(State(s): State<Arc<NodeState>>) -> ApiResult<serde_json::Value> {
    let sync = s.sync_handle().ok_or_else(|| {
        ApiError::new(
            StatusCode::PRECONDITION_FAILED,
            "no Pubky session: sign in with Pubky Ring first",
        )
    })?;
    sync.backup_settings(&s).await.map_err(sync_error)?;
    Ok(Json(serde_json::json!({"ok": true})))
}

/// `POST /v1/node/settings/restore`: bring the owner's settings from the
/// homeserver onto this node (missing values only; existing ones stay).
async fn settings_restore_now(State(s): State<Arc<NodeState>>) -> ApiResult<sync::SettingsRestored> {
    let sync = s.sync_handle().ok_or_else(|| {
        ApiError::new(
            StatusCode::PRECONDITION_FAILED,
            "no Pubky session: sign in with Pubky Ring first",
        )
    })?;
    let r = sync
        .restore_settings(&s, false)
        .await
        .map_err(sync_error)?
        .ok_or_else(|| ApiError::not_found("no settings backup on the homeserver yet"))?;
    s.notify_library();
    Ok(Json(r))
}

/// `PUT /v1/node/settings {contact}`: set the contact without touching the
/// config file; discovery comes alive at once.
async fn update_node_settings(
    State(s): State<Arc<NodeState>>,
    Json(req): Json<NodeSettingsUpdate>,
) -> ApiResult<NodeSettingsView> {
    if let Some(c) = req.contact {
        s.set_contact(&c).map_err(ApiError::bad_request)?;
    }
    if let Some(z) = req.jukebox_zone {
        if !z.is_empty() {
            zone_of(&s, &z)?;
        }
        s.library
            .lock()
            .expect("library poisoned")
            .set_setting(subsonic::JUKEBOX_ZONE_KEY, &z)?;
    }
    if let Some(f) = req.prefer_full {
        s.library
            .lock()
            .expect("library poisoned")
            .set_setting(item::PREFER_FULL_KEY, if f { "1" } else { "0" })?;
    }
    if let Some(g) = req.genre_tags {
        let lib = s.library.lock().expect("library poisoned");
        lib.set_setting(tags::GENRE_TAGS_KEY, if g { "1" } else { "0" })?;
        // Switched on: the tags now (they go out with the next sync); off keeps the ones made
        if g {
            let n = tags::publish_genres(&lib)?;
            tracing::info!(tags = n, "files' genres published as tags");
        }
    }
    if let Some(p) = req.sources_policy {
        s.library
            .lock()
            .expect("library poisoned")
            .set_setting(accounts::SOURCE_POLICY_KEY, p.as_str())?;
    }
    if let Some(h) = req.first_steps_hidden {
        s.library
            .lock()
            .expect("library poisoned")
            .set_setting(first_steps::HIDDEN_KEY, if h { "1" } else { "0" })?;
    }
    if let Some(d) = req.dev_outputs {
        s.library
            .lock()
            .expect("library poisoned")
            .set_setting(DEV_OUTPUTS_KEY, if d { "1" } else { "0" })?;
    }
    if let Some(r) = req.copy_rate {
        s.library
            .lock()
            .expect("library poisoned")
            .set_setting(files::COPY_RATE_KEY, &r.to_string())?;
    }
    Ok(Json(settings_view(&s)))
}

/// `[path, present]` for the config's folders, then the ones added through the API; present: the
/// folder is there now. Off the async threads: a stale network mount can hang the check.
async fn folder_list(s: &Arc<NodeState>) -> Result<Vec<(String, bool)>, ApiError> {
    let st = s.clone();
    tokio::task::spawn_blocking(move || folder_list_now(&st))
        .await
        .map_err(|e| ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
}

fn folder_list_now(s: &NodeState) -> Result<Vec<(String, bool)>, ApiError> {
    let added = s.library.lock().expect("library poisoned").list_folders()?;
    let mut out: Vec<(std::path::PathBuf, bool)> = s.scan_folders.iter().map(|p| (p.clone(), true)).collect();
    for (p, enabled) in added {
        if !out.iter().any(|(q, _)| *q == p) {
            out.push((p, enabled));
        }
    }
    Ok(out
        .into_iter()
        .map(|(p, enabled)| {
            let present = enabled && p.is_dir();
            (p.to_string_lossy().into_owned(), present)
        })
        .collect())
}

fn config_folders(s: &NodeState) -> Vec<String> {
    s.scan_folders
        .iter()
        .map(|p| p.to_string_lossy().into_owned())
        .collect()
}

/// `POST /v1/library/folders {path}`: add a folder, watch it, scan it.
async fn add_folder(
    State(s): State<Arc<NodeState>>,
    Json(req): Json<FolderRequest>,
) -> ApiResult<FoldersChanged> {
    let raw = req.path.trim();
    if raw.is_empty() {
        return Err(ApiError::bad_request("path is required"));
    }
    let path = std::path::PathBuf::from(raw);
    let path = tokio::fs::canonicalize(&path)
        .await
        .map_err(|e| ApiError::bad_request(format!("{}: {e}", path.display())))?;
    if !path.is_dir() {
        return Err(ApiError::bad_request(format!(
            "{} is not a directory",
            path.display()
        )));
    }
    s.library.lock().expect("library poisoned").add_folder(&path)?;
    s.watch_more(&path);
    let scan_started = matches!(s.start_scan(vec![path]), Ok(true));
    Ok(Json(FoldersChanged {
        folders: folder_list(&s).await?,
        config_folders: config_folders(&s),
        scan_started,
        removed_files: 0,
        removed_tracks: 0,
        moved_files: 0,
    }))
}

/// `DELETE /v1/library/folders?path=`: forget a folder and everything scanned from it.
async fn remove_folder(
    State(s): State<Arc<NodeState>>,
    Query(req): Query<FolderRequest>,
) -> ApiResult<FoldersChanged> {
    // The path as it was sent: a folder from the config is stored as it is written there, with a
    // slash at the end or without (`is_root` and `remove_folder` take either spelling)
    let raw = req.path.trim();
    if raw.is_empty() {
        return Err(ApiError::bad_request("path is required"));
    }
    let path = std::path::PathBuf::from(raw);
    if s.is_config_folder(&path) {
        return Err(ApiError::new(
            StatusCode::CONFLICT,
            format!(
                "{} is in the node's config ([library] folders, or PM_FOLDERS in Docker), which reads it at every start: take it out there and restart the node, then remove it here to forget its songs",
                path.display()
            ),
        ));
    }
    // Songs are forgotten by path, with their plays: only a folder the library was given (or
    // scanned as one) is taken, never a path above the folders or outside them
    if !s.library.lock().expect("library poisoned").is_root(&path)? {
        return Err(ApiError::not_found(format!(
            "{} is not one of the library's folders",
            path.display()
        )));
    }
    if s.is_scanning() {
        return Err(ApiError::new(
            StatusCode::CONFLICT,
            "a scan is running; try again when it is done",
        ));
    }
    s.unwatch(&path);
    let (removed_files, removed_tracks) = s.library.lock().expect("library poisoned").remove_folder(&path)?;
    s.notify_library();
    Ok(Json(FoldersChanged {
        folders: folder_list(&s).await?,
        config_folders: config_folders(&s),
        scan_started: false,
        removed_files,
        removed_tracks,
        moved_files: 0,
    }))
}

#[derive(Debug, serde::Deserialize)]
struct FolderMove {
    from: String,
    to: String,
}

/// `POST /v1/library/folders/move {from, to}`: the music under `from` now lives under `to` (a
/// restore on another machine, a new disk). Tracks keep their matches, playlists and history;
/// a rescan follows.
async fn move_folder(
    State(s): State<Arc<NodeState>>,
    Json(m): Json<FolderMove>,
) -> ApiResult<FoldersChanged> {
    if s.is_config_folder(std::path::Path::new(m.from.trim())) {
        return Err(ApiError::new(
            StatusCode::CONFLICT,
            format!(
                "{} is in the node's config ([library] folders, or PM_FOLDERS in Docker): put the new place there and restart the node",
                m.from.trim()
            ),
        ));
    }
    let to = tokio::fs::canonicalize(m.to.trim())
        .await
        .map_err(|e| ApiError::bad_request(format!("{}: {e}", m.to.trim())))?;
    if !to.is_dir() {
        return Err(ApiError::bad_request(format!("{} is not a folder", to.display())));
    }
    let Some(slot) = s.scan_slot() else {
        return Err(ApiError::new(
            StatusCode::CONFLICT,
            "a scan is running; try again when it is done",
        ));
    };
    // No match correction meanwhile (it would write a file under paths being moved)
    let held = s.no_corrections().await;
    let moved = s
        .library
        .lock()
        .expect("library poisoned")
        .move_folder(std::path::Path::new(m.from.trim()), &to);
    drop(held);
    drop(slot);
    let moved = moved?;
    s.unwatch(std::path::Path::new(m.from.trim()));
    s.watch_more(&to);
    let scan_started = matches!(s.start_scan(vec![to]), Ok(true));
    s.notify_library();
    Ok(Json(FoldersChanged {
        folders: folder_list(&s).await?,
        config_folders: config_folders(&s),
        scan_started,
        removed_files: 0,
        removed_tracks: 0,
        moved_files: moved,
    }))
}

/// Start a scan in the background on its own SQLite connection; one at a time.
async fn scan(State(s): State<Arc<NodeState>>, Json(body): Json<ScanRequest>) -> ApiResult<ScanStarted> {
    let folders: Vec<std::path::PathBuf> = if body.folders.is_empty() {
        s.all_folders()
    } else {
        body.folders.iter().map(std::path::PathBuf::from).collect()
    };
    match s.start_scan(folders.clone()) {
        Ok(true) => Ok(Json(ScanStarted {
            folders: folders.iter().map(|p| p.to_string_lossy().into_owned()).collect(),
        })),
        Ok(false) => Err(ApiError::new(StatusCode::CONFLICT, "a scan is already running")),
        Err(e) => Err(ApiError::bad_request(e)),
    }
}

/// `GET /v1/art/{album_id}?k=`: the cover for a renderer's screen (`NodeState::art_url`).
async fn keyed_cover(
    State(s): State<Arc<NodeState>>,
    Path(id): Path<i64>,
    Query(q): Query<std::collections::HashMap<String, String>>,
) -> Result<Response, ApiError> {
    if !q
        .get("k")
        .is_some_and(|k| s.stream_key_ok(&format!("art:{id}"), k))
    {
        return Err(ApiError::new(StatusCode::UNAUTHORIZED, "missing or wrong key"));
    }
    album_cover(State(s), Path(id)).await
}

/// The album's cover image (extracted at scan time or found in the folder).
async fn album_cover(State(s): State<Arc<NodeState>>, Path(id): Path<i64>) -> Result<Response, ApiError> {
    let known = s.library.lock().expect("library poisoned").album_art(id)?;
    let art = match known {
        Some(a) => a,
        None => art::fetch_missing_cover(&s, id)
            .await?
            .ok_or_else(|| ApiError::not_found(format!("no cover for album {id}")))?,
    };
    let bytes = tokio::fs::read(&art.file)
        .await
        .map_err(|e| ApiError::not_found(format!("cover file for album {id}: {e}")))?;
    // Its type by its bytes, also for covers stored before the scan checked them
    art::picture(bytes, "private, max-age=86400")
}

// ---- pubky ----

async fn pubky_status(State(s): State<Arc<NodeState>>) -> ApiResult<PubkyStatus> {
    let unsynced = s
        .library
        .lock()
        .expect("library poisoned")
        .unsynced_music_posts()?
        .len();
    let sync = s.sync_handle();
    let needs_resign = sync.as_ref().is_some_and(|h| h.needs_resign());
    Ok(Json(PubkyStatus {
        user: sync.as_ref().map(|h| h.z32().to_string()),
        unsynced,
        needs_resign,
        missing: sync
            .as_ref()
            .map(|h| h.missing_capabilities())
            .unwrap_or_default(),
        notice: needs_resign.then(|| sync::RESIGN_NOTICE.to_string()),
        sync: s.sync_activity.lock().expect("sync activity").clone(),
    }))
}

/// Start a Pubky Ring sign-in: returns the `pubkyauth://` URL to scan or
/// paste into Pubky Ring. Poll with GET until approved.
async fn pubky_login_start(State(s): State<Arc<NodeState>>) -> ApiResult<LoginStarted> {
    let hs = s
        .homeserver()
        .map_err(|e| ApiError::new(StatusCode::BAD_GATEWAY, e.to_string()))?;
    // The newest records first, so the grant exchange routes on them (the identity signed in
    // here before, when there is one)
    let identity = s
        .sync_handle()
        .and_then(|h| pubky::PublicKey::try_from(h.z32()).ok());
    let records = hs
        .fresh_before_sign_in_link(identity.as_ref(), None, s.bundled_homeserver().is_some())
        .await
        .map_err(|why| ApiError::new(StatusCode::SERVICE_UNAVAILABLE, why))?;
    let flow = hs
        .start_login("pubky-music-node")
        .map_err(|e| ApiError::new(StatusCode::BAD_GATEWAY, e.to_string()))?;
    let url = flow.url().to_string();
    *s.login.lock().await = Some(flow);
    let qr_svg = qr_svg(&url);
    Ok(Json(LoginStarted { records, url, qr_svg }))
}

const CONTROLLER_LOGIN_TTL: std::time::Duration = std::time::Duration::from_secs(10 * 60);

/// How often a controller log in may warm the homeserver records (see `controller_login_start`).
const CONTROLLER_WARM_EVERY: std::time::Duration = std::time::Duration::from_secs(30);

/// Run `work` unless it last succeeded less than `every` ago; one at a time (a caller that waited
/// for another's run uses its result). A failure is not remembered: the next caller tries again.
async fn now_and_then<F, Fut, E>(
    last: &tokio::sync::Mutex<Option<std::time::Instant>>,
    every: std::time::Duration,
    work: F,
) -> Result<(), E>
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = Result<(), E>>,
{
    let mut last = last.lock().await;
    if last.is_some_and(|at| at.elapsed() < every) {
        return Ok(());
    }
    work().await?;
    *last = Some(std::time::Instant::now());
    Ok(())
}
/// Sign-ins waiting at once, in all and from one address (security review 2026-10-30, #4).
const MAX_LOGINS: usize = 64;
const MAX_LOGINS_PER_ORIGIN: usize = 6;

/// Which waiting sign-in (`(id, started, origin)`) makes room for one more from `origin`: its own
/// oldest when it has its share, else, when every place is taken, the oldest of the biggest
/// bucket. Nobody is refused, so no client can lock everyone else out (review of #177); `None`
/// when there is room.
fn make_room(waiting: &[(String, std::time::Instant, String)], origin: &str) -> Option<String> {
    let oldest_of = |o: &str| {
        waiting
            .iter()
            .filter(|(_, _, w)| w == o)
            .min_by_key(|(_, at, _)| *at)
            .map(|(id, _, _)| id.clone())
    };
    let mine = waiting.iter().filter(|(_, _, w)| w == origin).count();
    if mine >= MAX_LOGINS_PER_ORIGIN {
        return oldest_of(origin);
    }
    if waiting.len() < MAX_LOGINS {
        return None;
    }
    // The biggest bucket; of equal ones, the one whose oldest is oldest
    let mut buckets: std::collections::HashMap<&str, (usize, std::time::Instant)> =
        std::collections::HashMap::new();
    for (_, at, w) in waiting {
        let b = buckets.entry(w.as_str()).or_insert((0, *at));
        b.0 += 1;
        b.1 = b.1.min(*at);
    }
    let biggest = buckets
        .into_iter()
        .max_by_key(|(_, (n, oldest))| (*n, std::cmp::Reverse(*oldest)))?
        .0;
    oldest_of(biggest)
}

/// An address as a sign-in bucket: IPv4 as it is, IPv6 by its /64 (one network hands out many).
fn login_bucket(ip: std::net::IpAddr) -> String {
    match ip {
        std::net::IpAddr::V6(v6) if v6.to_ipv4_mapped().is_none() => {
            let s = v6.segments();
            format!("{:x}:{:x}:{:x}:{:x}::/64", s[0], s[1], s[2], s[3])
        }
        std::net::IpAddr::V6(v6) => v6.to_ipv4_mapped().expect("mapped").to_string(),
        v4 => v4.to_string(),
    }
}

/// Who started a sign-in, for [`MAX_LOGINS_PER_ORIGIN`]: iroh for the remote tunnel; behind a
/// proxy or tunnel on this machine, the last address in `X-Forwarded-For` (the one that proxy
/// saw; anything before it the client may have written); else the connection's own. By
/// [`login_bucket`].
fn login_origin(req: &axum::extract::Request) -> String {
    if req.extensions().get::<peer::Remote>().is_some() {
        return "iroh".into();
    }
    let conn = req
        .extensions()
        .get::<axum::extract::ConnectInfo<std::net::SocketAddr>>()
        .map(|c| c.0.ip());
    // The forwarded address counts only from a proxy or tunnel on this machine (cloudflared, a
    // local reverse proxy): from anyone else it is whatever they wrote, a new bucket each time
    // (review of #177)
    if !conn.is_some_and(|ip| ip.is_loopback()) {
        return conn.map_or_else(|| "unknown".into(), login_bucket);
    }
    let forwarded = req
        .headers()
        .get_all("x-forwarded-for")
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(','))
        .filter_map(|a| a.trim().parse::<std::net::IpAddr>().ok())
        .next_back();
    forwarded.or(conn).map_or_else(|| "unknown".into(), login_bucket)
}

/// A QR code of `text` as an SVG string (dark modules on a light tile, a
/// quiet zone of four modules), for Pubky Ring sign-ins.
pub(crate) fn qr_svg(text: &str) -> Option<String> {
    let code = qrcode::QrCode::new(text.as_bytes()).ok()?;
    let w = code.width();
    let quiet = 4;
    let size = w + 2 * quiet;
    let mut path = String::new();
    for y in 0..w {
        for x in 0..w {
            if code[(x, y)] == qrcode::Color::Dark {
                path.push_str(&format!("M{} {}h1v1h-1z", x + quiet, y + quiet));
            }
        }
    }
    Some(format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 {size} {size}\" shape-rendering=\"crispEdges\"><rect width=\"{size}\" height=\"{size}\" fill=\"#fff\"/><path d=\"{path}\" fill=\"#111\"/></svg>"
    ))
}

/// How long a renewal code ([`login_renewal`]) stays good.
const RENEWAL_TTL: std::time::Duration = std::time::Duration::from_secs(15 * 60);

/// `POST /v1/auth/pubky/renewal` (owner): while the node needs new folders, a code for the next
/// Pubky Ring log ins from this page to ask for them, so only the owner's own page can start a log
/// in that asks anyone for write access (Kimi review of #209). The page gets it before logging out.
/// Good for 15 minutes, until a renewal succeeds: a wrong identity, Try again or an expired QR
/// code still renews.
async fn login_renewal(State(s): State<Arc<NodeState>>) -> ApiResult<LoginRenewal> {
    if !s.sync_handle().is_some_and(|h| h.needs_resign()) {
        return Ok(Json(LoginRenewal { renewal: None }));
    }
    let code = tidal::random_urlsafe(24);
    let mut all = s.renewals.lock().expect("renewals lock");
    all.retain(|_, at| at.elapsed() < RENEWAL_TTL);
    // Few at a time: the owner asks once per log out
    while all.len() >= 8 {
        let oldest = all.iter().min_by_key(|(_, at)| **at).map(|(c, _)| c.clone());
        let Some(oldest) = oldest else { break };
        all.remove(&oldest);
    }
    all.insert(code.clone(), std::time::Instant::now());
    Ok(Json(LoginRenewal { renewal: Some(code) }))
}

/// Whether `code` is a renewal code the node gave out, still good.
fn renewal_good(s: &NodeState, code: &str) -> bool {
    let mut all = s.renewals.lock().expect("renewals lock");
    all.retain(|_, at| at.elapsed() < RENEWAL_TTL);
    all.contains_key(code)
}

/// What a Pubky Ring log in asks for: the node's own folders only when the owner's page asked for
/// a renewal (with a code from [`login_renewal`]) and the node does lack them. Anyone may start a
/// log in, so a listener scanning the QR code is never asked to give the node write access to
/// their own homeserver.
fn login_capabilities(renew_asked: bool, needs_resign: bool) -> (bool, &'static str) {
    if renew_asked && needs_resign {
        (true, pm_pubky::homeserver::LOGIN_CAPABILITIES)
    } else {
        (false, pm_pubky::homeserver::CONTROLLER_CAPABILITIES)
    }
}

/// `POST /v1/auth/pubky/start [{renewal}]`: a controller or browser asks to log in with Pubky
/// Ring. The node tells the owner and listeners apart by who approves; a `renewal` code (the
/// owner's own page, when the node needs new folders) also asks for the node's folders. 412 until
/// the node has a Pubky session.
async fn controller_login_start(
    State(s): State<Arc<NodeState>>,
    req: axum::extract::Request,
) -> ApiResult<ControllerLoginStarted> {
    // Anyone may start one (it hands the token out), so few at a time: from one address, and in all
    let origin = login_origin(&req);
    // What is signing in, as it says, and whether from outside the home network: kept with the
    // sign-in for the owner's list (Settings), trusted for nothing
    let user_agent = req
        .headers()
        .get(header::USER_AGENT)
        .and_then(|v| v.to_str().ok())
        .map(str::to_string);
    let remote = peer::lan_peer(&req, s.is_behind_proxy()).is_none();
    let body = axum::body::to_bytes(req.into_body(), 1024)
        .await
        .map_err(|e| ApiError::new(StatusCode::BAD_REQUEST, e.to_string()))?;
    let asked = if body.iter().all(u8::is_ascii_whitespace) {
        ControllerLoginStart::default()
    } else {
        serde_json::from_slice::<ControllerLoginStart>(&body)
            .map_err(|e| ApiError::new(StatusCode::BAD_REQUEST, e.to_string()))?
    };
    let renew_asked = asked.renewal.is_some_and(|code| renewal_good(&s, &code));
    let device = accounts::NewSignIn::of(user_agent.as_deref(), asked.device, remote);
    // Before any relay wait: an approval kept at the relay is as old as the sign-in it answers
    let access = people::access_started(&s);
    // No owner, no sign-in: said before any homeserver lookup, so it doesn't hang on the network
    let no_owner = || {
        ApiError::new(
            StatusCode::PRECONDITION_FAILED,
            "the node has no Pubky session loaded: sign it in (Settings → Pubky, or `odeiond login`), or restart it once its homeserver answers",
        )
    };
    let Some(identity) = s.sync_handle().map(|h| h.z32().to_string()) else {
        return Err(no_owner());
    };
    let hs = s
        .homeserver()
        .map_err(|e| ApiError::new(StatusCode::BAD_GATEWAY, e.to_string()))?;
    // The newest records first, so the grant exchange routes on them: the owner's, to have them
    // ready. A listener on another homeserver may approve this log in too, so what is found of
    // the owner's homeserver never refuses it. Before the sign-ins' lock: these are network
    // lookups, and the polls of every waiting sign-in take that lock too. Anyone may ask (no
    // token), so the lookups run one at a time and at most every CONTROLLER_WARM_EVERY: what they
    // found stays in the client's cache meanwhile
    let identity = pubky::PublicKey::try_from(identity.as_str()).ok();
    now_and_then(&s.controller_warmed, CONTROLLER_WARM_EVERY, || async {
        hs.warm_for_sign_in(identity.as_ref(), s.bundled_homeserver().is_some())
            .await
            .map(drop)
    })
    .await
    .map_err(|why| ApiError::new(StatusCode::SERVICE_UNAVAILABLE, why))?;
    // Held from the check to the insert: sign-ins started at once can't pass the caps together
    let mut logins = s.controller_logins.lock().await;
    logins.retain(|_, (_, at, _, _, _)| at.elapsed() < CONTROLLER_LOGIN_TTL);
    let Some(owner) = s.sync_handle().map(|h| h.z32().to_string()) else {
        return Err(no_owner());
    };
    // The node's own session lacks folders and the owner asked: this log in asks for them too, so
    // the owner's approval renews it (no separate "sign in again")
    let (renews, caps) = login_capabilities(renew_asked, s.sync_handle().is_some_and(|h| h.needs_resign()));
    let flow = hs
        .start_login_with("pubky-music-controller", caps)
        .map_err(|e| ApiError::new(StatusCode::BAD_GATEWAY, e.to_string()))?;
    let url = flow.url().to_string();
    let id = tidal::random_urlsafe(24);
    loop {
        let waiting: Vec<(String, std::time::Instant, String)> = logins
            .iter()
            .map(|(id, (_, at, o, _, _))| (id.clone(), *at, o.clone()))
            .collect();
        match make_room(&waiting, &origin) {
            Some(old) => {
                logins.remove(&old);
            }
            None => break,
        }
    }
    logins.insert(
        id.clone(),
        (flow, std::time::Instant::now(), origin, device, access),
    );
    drop(logins);
    let qr_svg = qr_svg(&url);
    let owner_face = profiles::login_face(&s, &owner, profiles::FaceFrom::Owner).await;
    Ok(Json(ControllerLoginStarted {
        id,
        url,
        owner,
        owner_face,
        node_name: s.name.clone(),
        renews,
        qr_svg,
    }))
}

/// [`mint_listener_sign_in_as`] for a sign-in that says nothing about itself.
#[cfg(test)]
pub(crate) async fn mint_listener_sign_in(
    s: &NodeState,
    who: &str,
    started: u64,
) -> Result<Option<String>, ApiError> {
    mint_listener_sign_in_as(s, who, started, &accounts::NewSignIn::default()).await
}

/// A listener's Ring approval becomes a personal token only while the music access it started
/// under still stands: `started` is the access revision taken when the sign-in started, before
/// any relay wait. The final check and the synchronous mint run under the listener's membership
/// lock and the roster guard (the order Remove takes them), so Remove with a re-add, Music off or
/// a restore cannot cross them. Neither lock is held over network I/O. `None`: no music access.
/// `what` is what the sign-in said it is when it started, kept with it.
pub(crate) async fn mint_listener_sign_in_as(
    s: &NodeState,
    who: &str,
    started: u64,
    what: &accounts::NewSignIn,
) -> Result<Option<String>, ApiError> {
    let key = who.trim_start_matches("pubky");
    let membership = personal::membership_lock(s, key);
    let _membership = membership.lock().await;
    let _roster = s.roster.lock().await;
    people::access_current(s, key, started)?;
    if !people::may_use_music(s, key) {
        return Ok(None);
    }
    accounts::mint_as(s, key, what).map(Some)
}

/// `GET /v1/auth/pubky/poll?id=`: `pending` until Ring approves; `ok` with a browser session
/// of its own when the approver is the owner (renewing the node's session when this log in asked
/// for its folders); `denied` otherwise, with who approved and whose node it is.
async fn controller_login_poll(
    State(s): State<Arc<NodeState>>,
    Query(q): Query<std::collections::HashMap<String, String>>,
) -> ApiResult<ControllerLoginStatus> {
    let id = q.get("id").cloned().unwrap_or_default();
    let mut logins = s.controller_logins.lock().await;
    let Some((flow, at, _, device, access)) = logins.get(&id) else {
        return Ok(Json(ControllerLoginStatus::of("expired")));
    };
    let (device, access) = (device.clone(), *access);
    if at.elapsed() > CONTROLLER_LOGIN_TTL {
        logins.remove(&id);
        return Ok(Json(ControllerLoginStatus::of("expired")));
    }
    match flow.poll().await {
        Ok(None) => Ok(Json(ControllerLoginStatus::of("pending"))),
        Ok(Some(session)) => {
            logins.remove(&id);
            drop(logins);
            let who = session.public_key().to_string();
            let owner = s.sync_handle().map(|h| h.z32().to_string()).unwrap_or_default();
            if who != owner {
                // A listener in People with music access gets a personal token, unless their
                // access changed after this sign-in started (409: start a new one)
                if let Some(token) = mint_listener_sign_in_as(&s, &who, access, &device).await? {
                    tracing::info!(%who, "listener signed in with Pubky Ring");
                    return Ok(Json(ControllerLoginStatus {
                        token: Some(token),
                        user: Some(who),
                        role: Some("listener".into()),
                        ..ControllerLoginStatus::of("ok")
                    }));
                }
                let key = who.trim_start_matches("pubky");
                let known = people::load(&s).people.iter().any(|p| p.key == key);
                tracing::warn!(%who, %owner, known, "controller sign-in by a different identity refused");
                // The approver may be anyone: their profile from the cache or Nexus only, never
                // fetched from a homeserver they name
                let (approver, owner_face) = tokio::join!(
                    profiles::login_face(&s, &who, profiles::FaceFrom::Nexus),
                    profiles::login_face(&s, &owner, profiles::FaceFrom::Owner)
                );
                return Ok(Json(ControllerLoginStatus {
                    user: Some(who),
                    reason: Some(if known { "no_access" } else { "not_owner" }.into()),
                    approver: Some(approver),
                    owner_face: Some(owner_face),
                    ..ControllerLoginStatus::of("denied")
                }));
            }
            // A log in that asked for the node's folders, approved by the owner: the node's
            // session is renewed with it
            let granted = pm_pubky::homeserver::session_capabilities(&session);
            let renew = s.sync_handle().is_some_and(|h| h.needs_resign())
                && pm_pubky::homeserver::missing_capabilities(
                    &granted,
                    pm_pubky::homeserver::LOGIN_CAPABILITIES,
                )
                .is_empty();
            if renew {
                adopt_session(&s, session).await?;
                // Renewed: the codes for it are used up
                s.renewals.lock().expect("renewals lock").clear();
            }
            let token = accounts::mint_owner_as(&s, &who, &device)?;
            tracing::info!(%who, renew, device = device.device, "controller signed in with Pubky Ring");
            Ok(Json(ControllerLoginStatus {
                token: Some(token),
                user: Some(who),
                role: Some("owner".into()),
                ..ControllerLoginStatus::of("ok")
            }))
        }
        Err(e) => {
            logins.remove(&id);
            Err(ApiError::new(StatusCode::BAD_GATEWAY, e.to_string()))
        }
    }
}

/// The node signs in to Pubky with `session`: its secret goes to the configured file, sync
/// starts with it, and the node record, settings and the first sync follow in the background.
/// Returns the user's z32 key.
async fn adopt_session(s: &Arc<NodeState>, session: pubky::PubkySession) -> Result<String, ApiError> {
    let secret = pm_pubky::homeserver::export_session_secret(&session).await;
    if let (Some(path), Some(secret)) = (&s.session_secret_path, &secret) {
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        // Created 0600, never written first and made private after
        if let Err(e) = imports::write_private(path, secret.as_bytes()) {
            tracing::warn!(path = %path.display(), "cannot store the session secret: {e}");
        }
    }
    let hs = s
        .homeserver()
        .map_err(|e| ApiError::new(StatusCode::BAD_GATEWAY, e.to_string()))?;
    let sync = Arc::new(sync::PubkySync::from_session(hs, session));
    let user = sync.z32().to_string();
    s.set_sync(Some(sync.clone()));
    tracing::info!(user = %user, "pubky sign-in: session ready");
    // "Signed in" now; the node record, settings and the first sync follow in the background
    // (they took minutes, one request at a time, before the answer)
    sync::first_sync_in_background(s, sync.clone());
    Ok(user)
}

/// Poll the sign-in. When approved: the session is kept, its secret written
/// to the configured file, the node record published, playlists synced.
async fn pubky_login_poll(State(s): State<Arc<NodeState>>) -> ApiResult<LoginStatus> {
    if let Some(sync) = s.sync_handle() {
        if s.login.lock().await.is_none() {
            return Ok(Json(LoginStatus {
                status: "signed_in".into(),
                user: Some(sync.z32().to_string()),
                url: None,
            }));
        }
    }
    let mut guard = s.login.lock().await;
    let Some(flow) = guard.as_ref() else {
        return Ok(Json(LoginStatus {
            status: "none".into(),
            user: None,
            url: None,
        }));
    };
    let url = flow.url().to_string();
    match flow.poll().await {
        Ok(None) => Ok(Json(LoginStatus {
            status: "pending".into(),
            user: None,
            url: Some(url),
        })),
        Ok(Some(session)) => {
            *guard = None;
            drop(guard);
            let user = adopt_session(&s, session).await?;
            Ok(Json(LoginStatus {
                status: "signed_in".into(),
                user: Some(user),
                url: None,
            }))
        }
        Err(e) => {
            *guard = None;
            Err(ApiError::new(StatusCode::BAD_GATEWAY, e.to_string()))
        }
    }
}

/// Push local changes to the private homeserver folder and pull what is missing.
/// A sync error as an answer: a session another sign-in replaced meanwhile is a conflict (409),
/// not a homeserver failure.
fn sync_error(e: sync::SyncError) -> ApiError {
    match e {
        sync::SyncError::Replaced => ApiError::new(StatusCode::CONFLICT, e.to_string()),
        e => ApiError::new(StatusCode::BAD_GATEWAY, e.to_string()),
    }
}

async fn pubky_sync(State(s): State<Arc<NodeState>>) -> ApiResult<SyncReport> {
    let Some(sync) = s.sync_handle() else {
        return Err(ApiError::new(
            StatusCode::PRECONDITION_FAILED,
            "no Pubky session: set `pubky.session_secret_file` in the node config",
        ));
    };
    let report = sync.sync(&s.library).await.map_err(sync_error)?;
    let _ = sync.backup_settings(&s).await;
    if report.pulled > 0 {
        s.notify_library();
    }
    sync.carry_on(&s, &report);
    Ok(Json(report))
}

#[cfg(test)]
mod l16_tests {
    use super::L16;

    #[test]
    fn pcm_becomes_big_endian_lpcm_across_any_chunking() {
        let pcm = [0x01u8, 0x02, 0x03, 0x04, 0x05, 0x06];
        let want = vec![0x02, 0x01, 0x04, 0x03, 0x06, 0x05];
        for cut in 0..pcm.len() {
            for cut2 in cut..pcm.len() {
                let mut c = L16::default();
                let mut got = c.convert(&pcm[..cut]);
                got.extend(c.convert(&pcm[cut..cut2]));
                got.extend(c.convert(&pcm[cut2..]));
                assert_eq!(got, want, "cuts at {cut}, {cut2}");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request;
    use http_body_util::BodyExt;
    use pm_audio::PipelineConfig;
    use pm_library::Library;
    use pm_zones::outputs::NullOutput;
    use pm_zones::OutputRegistry;
    use tower::ServiceExt;

    fn write_wav(path: &std::path::Path, frames: u32) {
        let spec = hound::WavSpec {
            channels: 2,
            sample_rate: 44_100,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut w = hound::WavWriter::create(path, spec).unwrap();
        for i in 0..frames {
            w.write_sample((i % 100) as i16).unwrap();
            w.write_sample(-((i % 100) as i16)).unwrap();
        }
        w.finalize().unwrap();
    }

    /// The node's router as the node serves it: the API with the web UI's pages next to it
    /// (pm-web), so the tests below see a page path and an API path side by side.
    fn router(state: Arc<NodeState>) -> Router {
        router_with_pages(state, pm_web::router())
    }

    pub(crate) async fn state(dir: &std::path::Path, token: Option<&str>) -> Arc<NodeState> {
        state_with(dir, token, Default::default()).await
    }

    async fn state_with(
        dir: &std::path::Path,
        token: Option<&str>,
        pubky: pm_pubky::config::PmPubkyConfig,
    ) -> Arc<NodeState> {
        let db = dir.join("lib.sqlite");
        let lib = Library::open(&db).unwrap();
        let outputs = OutputRegistry::new();
        outputs.register(Arc::new(NullOutput::new()));
        let s = NodeState::new(
            NodeSettings {
                name: "test".into(),
                version: "0".into(),
                db_path: db,
                scan_folders: vec![dir.join("music")],
                art_dir: dir.join("art"),
                mirror_dir: dir.join("homeserver"),
                pipeline: PipelineConfig::default(),
                token: token.map(str::to_string),
                subsonic_user: "pubky".into(),
                pubky,
                public_url: "http://127.0.0.1:7788".into(),
                upnp: false,
                cast: false,
                slimproto_port: 0,
                session_secret_path: None,
                tidal: Default::default(),
                spotify: Default::default(),
                acoustid_key: String::new(),
                jamendo: Default::default(),
            },
            lib,
            outputs,
        );
        // Never reach the real Deezer API from tests; a closed port fails fast.
        *s.deezer_base.write().unwrap() = "http://127.0.0.1:9".into();
        // As the node does at start (main.rs)
        accounts::clean_saved_queues(&s).unwrap();
        s.zones_for_all_outputs();
        s
    }

    async fn call(
        app: &Router,
        method: &str,
        uri: &str,
        body: Option<serde_json::Value>,
    ) -> (StatusCode, serde_json::Value) {
        let req = Request::builder()
            .method(method)
            .uri(uri)
            .header("content-type", "application/json")
            .body(match body {
                Some(b) => Body::from(b.to_string()),
                None => Body::empty(),
            })
            .unwrap();
        let resp = app.clone().oneshot(req).await.unwrap();
        let status = resp.status();
        let bytes = resp.into_body().collect().await.unwrap().to_bytes();
        let json = if bytes.is_empty() {
            serde_json::Value::Null
        } else {
            serde_json::from_slice(&bytes).unwrap_or_else(|e| {
                panic!(
                    "{method} {uri}: {status}: not JSON ({e}): {}",
                    String::from_utf8_lossy(&bytes)
                )
            })
        };
        (status, json)
    }

    /// One mock for Google (OAuth, Drive API) and Microsoft (OAuth, Graph, pre-authenticated
    /// downloads). Files: Music/01 Song.flac and Music/Sub/02 Two.flac.
    async fn mock_drives(flac: Vec<u8>, refreshes: Arc<std::sync::atomic::AtomicU32>) -> String {
        use axum::extract::{Path as P, Query as Q};
        use std::collections::HashMap as M;
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());
        fn ranged(h: &axum::http::HeaderMap, body: &[u8]) -> axum::response::Response {
            use axum::response::IntoResponse;
            match h
                .get("range")
                .and_then(|v| v.to_str().ok())
                .and_then(|r| r.strip_prefix("bytes="))
            {
                Some(r) => {
                    let (a, b) = r.split_once('-').unwrap();
                    let a: usize = a.parse().unwrap();
                    let b: usize = b.parse::<usize>().unwrap_or(body.len() - 1).min(body.len() - 1);
                    (
                        StatusCode::PARTIAL_CONTENT,
                        [(header::CONTENT_TYPE, "application/octet-stream")],
                        body[a..=b].to_vec(),
                    )
                        .into_response()
                }
                None => (
                    [(header::CONTENT_TYPE, "application/octet-stream")],
                    body.to_vec(),
                )
                    .into_response(),
            }
        }
        let token = move |body: String, prefix: &'static str, r: Arc<std::sync::atomic::AtomicU32>| {
            let f: M<String, String> = url::form_urlencoded::parse(body.as_bytes())
                .into_owned()
                .collect();
            assert!(
                !f.contains_key("client_secret"),
                "a public client sends no secret"
            );
            assert_eq!(f["client_id"], format!("{prefix}-client"));
            match f["grant_type"].as_str() {
                "authorization_code" => {
                    assert_eq!(f["code"], "C0DE");
                    assert!(f["code_verifier"].len() >= 43, "PKCE");
                    // Expires at once, so the next call refreshes
                    Json(
                        serde_json::json!({"access_token": format!("{prefix}-A1"), "refresh_token": format!("{prefix}-R"), "expires_in": 1}),
                    )
                }
                "refresh_token" => {
                    assert_eq!(f["refresh_token"], format!("{prefix}-R"));
                    r.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    Json(serde_json::json!({"access_token": format!("{prefix}-A2"), "expires_in": 3600}))
                }
                g => panic!("grant {g}"),
            }
        };
        let auth = |h: &axum::http::HeaderMap, prefix: &str| {
            let a = h.get("authorization").and_then(|v| v.to_str().ok()).unwrap_or("");
            assert!(a.starts_with(&format!("Bearer {prefix}-A")), "{a}");
        };
        let (r1, r2) = (refreshes.clone(), refreshes);
        let (f1, f2, f3) = (flac.clone(), flac.clone(), flac);
        let b2 = base.clone();
        let app = Router::new()
            .route("/google/token", post(move |body: String| { let r = r1.clone(); async move { token(body, "g", r) } }))
            .route("/ms/token", post(move |body: String| { let r = r2.clone(); async move { token(body, "m", r) } }))
            .route("/gapi/drive/v3/about", get(move |h: axum::http::HeaderMap| async move { auth(&h, "g"); Json(serde_json::json!({"user": {"emailAddress": "me@gmail.example"}})) }))
            .route("/gapi/drive/v3/files", get(move |h: axum::http::HeaderMap, Q(q): Q<M<String, String>>| {
                let n = f1.len();
                async move {
                    auth(&h, "g");
                    let folder = q["q"].split('\'').nth(1).unwrap().to_string();
                    let file = |id: &str, name: &str| serde_json::json!({"id": id, "name": name, "mimeType": "audio/flac", "size": n.to_string(), "modifiedTime": "2026-10-20T10:00:00Z"});
                    let dir = |id: &str, name: &str| serde_json::json!({"id": id, "name": name, "mimeType": "application/vnd.google-apps.folder"});
                    Json(serde_json::json!({"files": match folder.as_str() {
                        "root" => vec![dir("F1", "Music")],
                        "F1" => vec![file("G1", "01 Song.flac"), dir("F2", "Sub")],
                        "F2" => vec![file("G2", "02 Two.flac")],
                        _ => vec![],
                    }}))
                }
            }))
            .route("/gapi/drive/v3/files/{id}", get(move |h: axum::http::HeaderMap, Q(q): Q<M<String, String>>| {
                let f = f2.clone();
                async move { auth(&h, "g"); assert_eq!(q["alt"], "media"); ranged(&h, &f) }
            }))
            .route("/graph/me", get(move |h: axum::http::HeaderMap| async move { auth(&h, "m"); Json(serde_json::json!({"userPrincipalName": "me@outlook.example"})) }))
            .route("/graph/me/drive/items/{id}/children", get(move |h: axum::http::HeaderMap, P(id): P<String>, Q(q): Q<M<String, String>>| {
                let b = b2.clone();
                async move {
                    auth(&h, "m");
                    let file = |id: &str, name: &str| serde_json::json!({"id": id, "name": name, "size": 47782, "file": {}, "lastModifiedDateTime": "2026-10-20T10:00:00Z"});
                    let dir = |id: &str, name: &str| serde_json::json!({"id": id, "name": name, "folder": {"childCount": 1}});
                    Json(match (id.as_str(), q.contains_key("page2")) {
                        ("root", _) => serde_json::json!({"value": [dir("M1", "Music")]}),
                        // Two pages: the second through @odata.nextLink
                        ("M1", false) => serde_json::json!({"value": [file("O1", "01 Song.flac")], "@odata.nextLink": format!("{b}/graph/me/drive/items/M1/children?page2=1")}),
                        ("M1", true) => serde_json::json!({"value": [dir("M2", "Sub")]}),
                        ("M2", _) => serde_json::json!({"value": [file("O2", "02 Two.flac")]}),
                        _ => serde_json::json!({"value": []}),
                    })
                }
            }))
            .route("/graph/me/drive/items/{id}/content", get(move |h: axum::http::HeaderMap, P(id): P<String>| async move {
                auth(&h, "m");
                // OneDrive answers with a pre-authenticated download address
                axum::response::Redirect::temporary(&format!("/dl/{id}"))
            }))
            .route("/dl/{id}", get(move |h: axum::http::HeaderMap| { let f = f3.clone(); async move { ranged(&h, &f) } }));
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        base
    }

    fn tone_wav(path: &std::path::Path, secs: u32) {
        let spec = hound::WavSpec {
            channels: 2,
            sample_rate: 44_100,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut w = hound::WavWriter::create(path, spec).unwrap();
        for i in 0..(secs * 44_100) {
            let t = i as f64 / 44_100.0;
            let f = [261.6, 329.6, 392.0, 440.0][((t * 2.0) as usize) % 4];
            let v = ((t * f * std::f64::consts::TAU).sin() * 12_000.0) as i16;
            w.write_sample(v).unwrap();
            w.write_sample(v).unwrap();
        }
        w.finalize().unwrap();
    }

    fn untagged_track(path: std::path::PathBuf, title: &str, artist: &str) -> pm_model::Track {
        pm_model::Track {
            id: None,
            title: title.into(),
            artists: vec![pm_model::Artist {
                id: None,
                name: artist.into(),
                mbid: None,
            }],
            album: None,
            track_no: Some(11),
            disc_no: Some(1),
            duration_ms: Some(10_000),
            file: pm_model::AudioFile {
                path,
                size_bytes: 1,
                mtime_unix: 0,
                container: "wav".into(),
                format: pm_model::AudioFormat::pcm(44_100, 16, 2),
                duration_ms: Some(10_000),
            },
            recording_mbid: None,
            isrc: None,
            acoustid: None,
            canonical: None,
            replaygain_track_gain_db: None,
            replaygain_album_gain_db: None,
            genres: vec![],
        }
    }

    /// A node at `public_url`, which the binary guessed (none configured) or the owner set.
    async fn state_at(dir: &std::path::Path, public_url: &str, guessed: bool) -> Arc<NodeState> {
        let db = dir.join("lib.sqlite");
        let s = NodeState::new(
            NodeSettings {
                name: "test".into(),
                version: "0".into(),
                db_path: db.clone(),
                scan_folders: vec![],
                art_dir: dir.join("art"),
                mirror_dir: dir.join("homeserver"),
                pipeline: PipelineConfig::default(),
                token: None,
                subsonic_user: "pubky".into(),
                pubky: Default::default(),
                public_url: public_url.into(),
                upnp: false,
                cast: false,
                slimproto_port: 0,
                session_secret_path: None,
                tidal: Default::default(),
                spotify: Default::default(),
                acoustid_key: String::new(),
                jamendo: Default::default(),
            },
            Library::open(&db).unwrap(),
            OutputRegistry::new(),
        );
        s.set_public_url_guessed(guessed);
        s.tidal.write().unwrap().creds = tidal::TidalCredentials {
            client_id: "cid".into(),
            client_secret: "sec".into(),
            scopes: String::new(),
        };
        s
    }

    async fn tidal_redirect(s: Arc<NodeState>, host: &str) -> String {
        // A node reached by another name than localhost has a token
        *s.token.write().unwrap() = Some("s3cret".into());
        let req = Request::builder()
            .uri("/v1/sources/tidal/status")
            .header("host", host)
            .header("authorization", "Bearer s3cret")
            .body(Body::empty())
            .unwrap();
        let resp = router(s).oneshot(req).await.unwrap();
        let bytes = resp.into_body().collect().await.unwrap().to_bytes();
        let v: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        v["redirect_uri"].as_str().unwrap().to_string()
    }

    /// One matching pass at a time runs in the node, so tests that run one take turns.
    static MATCH_PASSES: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

    /// A TIDAL mock that plays one track and records the bearer token of every manifest request.
    async fn tidal_mock_recording_tokens(flac: Vec<u8>) -> (String, Arc<std::sync::Mutex<Vec<String>>>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());
        let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
        let manifest_base = base.clone();
        let seen2 = seen.clone();
        let mock = Router::new()
            .route(
                "/trackManifests/{id}",
                get(move |Path(id): Path<String>, headers: HeaderMap| {
                    let manifest_base = manifest_base.clone();
                    let seen = seen2.clone();
                    async move {
                        let auth = headers.get("authorization").unwrap().to_str().unwrap().to_string();
                        seen.lock().unwrap().push(auth);
                        Json(serde_json::json!({"data": {"id": id, "type": "trackManifests", "attributes": {
                            "uri": format!("{manifest_base}/hls/master.m3u8"), "formats": ["FLAC"], "trackPresentation": "FULL", "hash": "h"}}}))
                    }
                }),
            )
            .route("/hls/master.m3u8", get(|| async { "#EXTM3U\n#EXT-X-STREAM-INF:BANDWIDTH=1000000\nmedia.m3u8\n" }))
            .route("/hls/media.m3u8", get(|| async { "#EXTM3U\n#EXT-X-PLAYLIST-TYPE:VOD\n#EXTINF:1.0,\nseg/0\n#EXT-X-ENDLIST\n" }))
            .route(
                "/hls/seg/{n}",
                get(move |Path(_n): Path<usize>| {
                    let flac = flac.clone();
                    async move { ([(header::CONTENT_TYPE, "application/octet-stream")], flac) }
                }),
            );
        tokio::spawn(async move { axum::serve(listener, mock).await.unwrap() });
        (base, seen)
    }

    /// A node with TIDAL against `base`, the owner signed in as `owner-token`, and listeners
    /// Anna (signed in as `anna-token`) and Ben (no TIDAL).
    async fn two_people_node(
        dir: &std::path::Path,
        base: &str,
    ) -> (Arc<NodeState>, &'static str, &'static str) {
        let s = state(dir, Some("s3cret")).await;
        {
            let mut ep = s.tidal.write().unwrap();
            ep.api_base = base.to_string();
            ep.auth_base = base.to_string();
            ep.creds = tidal::TidalCredentials {
                client_id: "cid".into(),
                client_secret: "sec".into(),
                scopes: String::new(),
            };
        }
        let session = |token: &str| {
            serde_json::json!({"access_token": token, "refresh_token": "r", "expires_at": pm_library::db::now_unix() + 3600, "user_id": "1", "scope": "playback"}).to_string()
        };
        let (anna, ben) = (
            "tbepfp8fisyiqpskbmxsmpub47gxb7xdsh5x9ut9qrb97gcq9phy",
            "o4dksfbqk85ogzdb5osziw6befigbuxmuxkuxq8434q89uj56uyy",
        );
        let lib = s.library.lock().unwrap();
        lib.set_setting("tidal.user", &session("owner-token")).unwrap();
        lib.set_setting(&format!("person.{anna}.tidal.user"), &session("anna-token"))
            .unwrap();
        lib.set_setting(
            "people",
            &serde_json::json!({"people": [
            {"key": anna, "access": "music", "music_enabled": true, "added_at": 1},
            {"key": ben, "access": "music", "music_enabled": true, "added_at": 1}]})
            .to_string(),
        )
        .unwrap();
        drop(lib);
        (s, anna, ben)
    }

    /// A request with a node token, and its status, JSON body and headers.
    async fn send_as(
        app: &Router,
        method: &str,
        uri: &str,
        token: Option<&str>,
        extra: &[(&str, &str)],
        body: Option<serde_json::Value>,
    ) -> (StatusCode, serde_json::Value, HeaderMap) {
        let mut req = Request::builder()
            .method(method)
            .uri(uri)
            .header("content-type", "application/json");
        if let Some(t) = token {
            req = req.header("authorization", format!("Bearer {t}"));
        }
        for (k, v) in extra {
            req = req.header(*k, *v);
        }
        let r = app
            .clone()
            .oneshot(
                req.body(
                    body.map(|b| Body::from(b.to_string()))
                        .unwrap_or_else(Body::empty),
                )
                .unwrap(),
            )
            .await
            .unwrap();
        let (st, h) = (r.status(), r.headers().clone());
        let b = r.into_body().collect().await.unwrap().to_bytes();
        (st, serde_json::from_slice(&b).unwrap_or_default(), h)
    }

    /// A TIDAL API mock: the token endpoint (`token` for any grant) and `/searchResults`,
    /// recording each search's bearer.
    async fn tidal_auth_and_search_mock(token: &'static str) -> (String, Arc<std::sync::Mutex<Vec<String>>>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());
        let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
        let seen2 = seen.clone();
        let mock = Router::new()
            .route(
                "/v1/oauth2/token",
                post(move || async move {
                    Json(serde_json::json!({"access_token": token, "refresh_token": "r", "expires_in": 3600, "user_id": 77}))
                }),
            )
            .route(
                "/searchResults",
                get(move |headers: HeaderMap| {
                    let seen = seen2.clone();
                    async move {
                        let b = headers.get("authorization").unwrap().to_str().unwrap().to_string();
                        seen.lock().unwrap().push(b);
                        Json(serde_json::json!({"data": [], "included": []}))
                    }
                }),
            );
        tokio::spawn(async move { axum::serve(listener, mock).await.unwrap() });
        (base, seen)
    }

    /// A homeserver for the copy jobs' tests: records uploads and deletes; uploads fail while
    /// `fails` is set, keep `short` bytes less than sent, and use up `disk`'s free space.
    #[derive(Default)]
    struct FakeHomeserver {
        uploads: std::sync::Mutex<Vec<String>>,
        deletes: std::sync::Mutex<Vec<String>>,
        sizes: std::sync::Mutex<std::collections::HashMap<String, u64>>,
        fails: std::sync::atomic::AtomicBool,
        short: std::sync::atomic::AtomicU64,
        disk: Option<Arc<FakeDisk>>,
        /// What else each upload uses on the disk (the homeserver's database and logs).
        extra: u64,
        /// What the homeserver lists before the copy.
        listed: std::sync::Mutex<Vec<pm_pubky::homeserver::RemoteFile>>,
        /// Uploads after this many fail, as while `fails` is set.
        fail_after: Option<usize>,
        /// Checking a file's size fails (the homeserver went away after an upload).
        stat_fails: std::sync::atomic::AtomicBool,
        /// `stat_fails` is set by the first upload (checks before it go through).
        stat_fails_after_upload: bool,
        /// Files whose size the homeserver doesn't say (no Content-Length).
        no_size: std::sync::Mutex<std::collections::HashSet<String>>,
        /// How long each upload takes.
        delay: Option<std::time::Duration>,
        /// How long a size check takes.
        stat_delay: Option<std::time::Duration>,
        /// Uploads (`up:`) and deletes (`del:`) in the order they happened.
        ops: std::sync::Mutex<Vec<String>>,
        /// Whose homeserver it is when set (else `fake-account`): a sign-in as someone else.
        who: std::sync::Mutex<Option<String>>,
    }

    #[async_trait::async_trait]
    impl copy_guard::Target for FakeHomeserver {
        async fn list(&self) -> Result<Vec<pm_pubky::homeserver::RemoteFile>, String> {
            Ok(self.listed.lock().unwrap().clone())
        }
        async fn upload(
            &self,
            remote: &str,
            bytes: Vec<u8>,
            _rate: Option<u64>,
        ) -> Result<(), copy_guard::UploadError> {
            if let Some(d) = self.delay {
                tokio::time::sleep(d).await;
            }
            self.ops.lock().unwrap().push(format!("up:{remote}"));
            if self.stat_fails_after_upload {
                self.stat_fails.store(true, std::sync::atomic::Ordering::SeqCst);
            }
            let n = {
                let mut u = self.uploads.lock().unwrap();
                u.push(remote.to_string());
                u.len()
            };
            if self.fails.load(std::sync::atomic::Ordering::SeqCst) || self.fail_after.is_some_and(|k| n > k)
            {
                // As a homeserver failing mid-write: half the file left there
                self.sizes
                    .lock()
                    .unwrap()
                    .insert(remote.to_string(), bytes.len() as u64 / 2);
                return Err(copy_guard::UploadError::Other("500 Internal Server Error".into()));
            }
            let kept =
                (bytes.len() as u64).saturating_sub(self.short.load(std::sync::atomic::Ordering::SeqCst));
            self.sizes.lock().unwrap().insert(remote.to_string(), kept);
            if let Some(d) = &self.disk {
                let mut sp = d.0.lock().unwrap();
                sp.free = sp.free.saturating_sub(kept + self.extra);
            }
            Ok(())
        }
        async fn stat(&self, remote: &str) -> Result<Option<Option<u64>>, String> {
            if let Some(d) = self.stat_delay {
                tokio::time::sleep(d).await;
            }
            if self.stat_fails.load(std::sync::atomic::Ordering::SeqCst) {
                return Err("connection reset".into());
            }
            let no_size = self.no_size.lock().unwrap().contains(remote);
            Ok(self
                .sizes
                .lock()
                .unwrap()
                .get(remote)
                .map(|n| (!no_size).then_some(*n)))
        }
        async fn delete(&self, remote: &str) -> Result<(), String> {
            self.ops.lock().unwrap().push(format!("del:{remote}"));
            self.deletes.lock().unwrap().push(remote.to_string());
            Ok(())
        }
        fn account(&self) -> String {
            self.who
                .lock()
                .unwrap()
                .clone()
                .unwrap_or_else(|| "fake-account".into())
        }
    }

    /// The bundled homeserver's disk, as the copy jobs see it.
    struct FakeDisk(std::sync::Mutex<copy_guard::Space>);

    impl copy_guard::Room for FakeDisk {
        fn space(&self) -> Option<copy_guard::Space> {
            Some(*self.0.lock().unwrap())
        }
    }

    const GB: u64 = 1 << 30;

    /// A node whose copy jobs go to `hs` on `disk`, and a folder of `n` short songs.
    async fn copy_setup(
        hs: Arc<FakeHomeserver>,
        disk: Arc<FakeDisk>,
        n: u32,
    ) -> (tempfile::TempDir, Arc<NodeState>, Router, std::path::PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let s = state(dir.path(), None).await;
        *s.copy_seams.lock().unwrap() = (Some(hs), Some(disk));
        let src = dir.path().join("music");
        std::fs::create_dir_all(&src).unwrap();
        for i in 1..=n {
            tone_wav(&src.join(format!("{i:02} song.wav")), 1);
        }
        let app = router(s.clone());
        (dir, s, app, src)
    }

    async fn copy_finished(app: &Router, id: &str) -> serde_json::Value {
        for _ in 0..200 {
            let (_, v) = call(app, "GET", &format!("/v1/pubky/files/upload/{id}"), None).await;
            if v["finished"] == true {
                return v;
            }
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        }
        panic!("the copy never finished");
    }

    /// The homeserver name a library file is copied to, and its size (its hash remembered as a
    /// copy before would have).
    fn copied_name(s: &NodeState, path: &std::path::Path) -> (String, u64) {
        let bytes = std::fs::read(path).unwrap();
        let hash = blake3::hash(&bytes).to_hex().to_string();
        let m = std::fs::metadata(path).unwrap();
        let mtime = m
            .modified()
            .unwrap()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;
        s.library
            .lock()
            .unwrap()
            .store_file_hash(path, m.len(), mtime, &hash)
            .unwrap();
        (
            format!(
                "{}{}",
                pm_pubky::homeserver::PRIVATE_FILES_DIR,
                files::remote_name_for(&hash, &files::safe_file_name(path))
            ),
            bytes.len() as u64,
        )
    }

    // Identification review, completion and reuse (their own file: fewer shared lines here)
    mod identify_review;

    /// Library songs at these paths (made as short tones), their sizes as on disk: what the
    /// copy-library tests pick from.
    fn songs_at(s: &NodeState, paths: &[std::path::PathBuf]) {
        for (i, p) in paths.iter().enumerate() {
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            tone_wav(p, 1 + i as u32 % 3);
            let mut t = untagged_track(p.clone(), &format!("Song {i}"), "Band");
            t.file.size_bytes = std::fs::metadata(p).unwrap().len();
            s.library.lock().unwrap().upsert_track(&t).unwrap();
        }
    }

    async fn copy_ended(app: &Router, id: &str) -> serde_json::Value {
        for _ in 0..400 {
            let (_, v) = call(app, "GET", &format!("/v1/pubky/files/upload/{id}"), None).await;
            if v["finished"] == true {
                return v;
            }
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        }
        panic!("the copy never ended");
    }

    fn roomy() -> Arc<FakeDisk> {
        Arc::new(FakeDisk(std::sync::Mutex::new(copy_guard::Space {
            free: 500 * GB,
            total: 1000 * GB,
        })))
    }

    /// A copy job kept for the next start: `id`, running, of the folder `A` under `src`, for
    /// `account` (none: kept by a node that didn't note it).
    fn kept_copy(id: &str, src: &std::path::Path, account: Option<&str>) -> String {
        let progress = types::UploadProgress {
            id: id.into(),
            kind: "copy".into(),
            state: "running".into(),
            total: 3,
            started_at: Some(1),
            ..Default::default()
        };
        let mut current = serde_json::json!({"progress": progress, "selection": {"paths": [src.join("A")]}});
        if let Some(a) = account {
            current["account"] = a.into();
        }
        serde_json::json!({"current": current, "queued": [], "last": null}).to_string()
    }

    // ---- TIDAL full tracks: Sign in with TIDAL (browser, PKCE), what TIDAL answered (2026-11-04) ----

    /// Log lines written while a test runs, for checking that no secret is in them.
    #[derive(Clone, Default)]
    struct LogBuf(Arc<std::sync::Mutex<Vec<u8>>>);

    impl std::io::Write for LogBuf {
        fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(b);
            Ok(b.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for LogBuf {
        type Writer = LogBuf;
        fn make_writer(&'a self) -> Self::Writer {
            self.clone()
        }
    }

    impl LogBuf {
        fn text(&self) -> String {
            String::from_utf8_lossy(&self.0.lock().unwrap()).into_owned()
        }
    }

    /// Everything the node logs (all levels of its own crates and the HTTP stack's), into a buffer.
    fn log_capture() -> (LogBuf, tracing::Dispatch) {
        let buf = LogBuf::default();
        let sub = tracing_subscriber::fmt()
            .with_writer(buf.clone())
            .with_ansi(false)
            .with_env_filter(tracing_subscriber::EnvFilter::new("trace"))
            .finish();
        (buf, tracing::Dispatch::new(sub))
    }

    /// What the mock TIDAL was sent, by endpoint.
    #[derive(Default)]
    struct TidalSeen {
        tokens: Vec<String>,
        asked: Vec<(String, String)>,
        agents: Vec<String>,
    }

    const FAKE_BROWSER_ID: &str = "fake-browser-client-id";
    const FAKE_BROWSER_SECRET: &str = "fake-browser-client-secret";

    /// A mock of TIDAL's auth, sessions and private playback for the full-tracks tests. Track ids:
    /// `1` BTS FLAC 16/44.1; `2` DASH FLAC 24/96 (hi-res); `3` DASH AAC for HI_RES_LOSSLESS, BTS
    /// FLAC for LOSSLESS; `4` DASH AAC at any quality. Every CDN address carries `SECRET-CDN`.
    async fn full_tracks_mock(flac: Vec<u8>) -> (String, Arc<std::sync::Mutex<TidalSeen>>) {
        use base64::Engine;
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());
        let seen = Arc::new(std::sync::Mutex::new(TidalSeen::default()));
        let b64 = |v: &str| base64::engine::general_purpose::STANDARD.encode(v);
        let bts_flac = b64(&format!(
            r#"{{"mimeType":"audio/flac","codecs":"flac","encryptionType":"NONE","urls":["{base}/file.flac?token=SECRET-CDN"]}}"#
        ));
        let dash = |codecs: &str, rate: u32, id: &str| {
            b64(&format!(
                r#"<?xml version='1.0' encoding='UTF-8'?><MPD xmlns="urn:mpeg:dash:schema:mpd:2011" profiles="urn:mpeg:dash:profile:isoff-main:2011" type="static" minBufferTime="PT3.993S" mediaPresentationDuration="PT0M8.0S"><Period id="0"><AdaptationSet id="0" contentType="audio" mimeType="audio/mp4" segmentAlignment="true"><Representation id="{id}" codecs="{codecs}" bandwidth="2770880" audioSamplingRate="{rate}"><SegmentTemplate timescale="{rate}" initialization="{base}/seg/init?token=SECRET-CDN" media="{base}/seg/$Number$?token=SECRET-CDN" startNumber="1"><SegmentTimeline><S d="383936" r="1"/><S d="214080"/></SegmentTimeline></SegmentTemplate></Representation></AdaptationSet></Period></MPD>"#
            ))
        };
        let dash_flac = dash("flac", 96_000, "FLAC,96000,24");
        let dash_aac = dash("mp4a.40.2", 44_100, "AAC,320000");
        let mock = Router::new()
            .route(
                "/v1/oauth2/token",
                post({
                    let seen = seen.clone();
                    move |body: String| {
                        let seen = seen.clone();
                        async move {
                            seen.lock().unwrap().tokens.push(body.clone());
                            let q: std::collections::HashMap<String, String> =
                                url::form_urlencoded::parse(body.as_bytes()).into_owned().collect();
                            let get = |k: &str| q.get(k).map(String::as_str).unwrap_or("");
                            match get("grant_type") {
                                "authorization_code" if get("code") == "good-code" || get("code") == "anna-code" => (
                                    StatusCode::OK,
                                    Json(serde_json::json!({"access_token": format!("pk-at-{}", get("code")), "refresh_token": "pk-rt", "expires_in": 3600, "token_type": "Bearer", "user_id": 77, "user": {"userId": 77, "countryCode": "CH"}})),
                                ),
                                "authorization_code" => (
                                    StatusCode::BAD_REQUEST,
                                    Json(serde_json::json!({"error": "invalid_grant", "error_description": "Authorization code is invalid"})),
                                ),
                                "refresh_token" if get("client_id") == FAKE_BROWSER_ID && get("client_secret") == FAKE_BROWSER_SECRET => (
                                    StatusCode::OK,
                                    Json(serde_json::json!({"access_token": "pk-at-renewed", "expires_in": 3600})),
                                ),
                                "refresh_token" if get("client_id") == "dev-id" && get("client_secret") == "dev-secret" => (
                                    StatusCode::OK,
                                    Json(serde_json::json!({"access_token": "dev-at-renewed", "refresh_token": "dev-rt-2", "expires_in": 3600})),
                                ),
                                _ => (
                                    StatusCode::UNAUTHORIZED,
                                    Json(serde_json::json!({"error": "invalid_client"})),
                                ),
                            }
                        }
                    }
                }),
            )
            .route(
                "/v1/sessions",
                get(|| async { Json(serde_json::json!({"userId": 77, "countryCode": "CH", "sessionId": "sid"})) }),
            )
            .route(
                "/v1/tracks/{id}/playbackinfopostpaywall",
                get({
                    let seen = seen.clone();
                    move |Path(id): Path<String>, headers: HeaderMap, q: axum::extract::Query<std::collections::HashMap<String, String>>| {
                        let seen = seen.clone();
                        let (bts_flac, dash_flac, dash_aac) = (bts_flac.clone(), dash_flac.clone(), dash_aac.clone());
                        async move {
                            let asked = q.get("audioquality").cloned().unwrap_or_default();
                            {
                                let mut seen = seen.lock().unwrap();
                                seen.asked.push((id.clone(), asked.clone()));
                                seen.agents.push(
                                    headers.get("user-agent").and_then(|v| v.to_str().ok()).unwrap_or("").to_string(),
                                );
                            }
                            let bts = || serde_json::json!({"trackId": id, "assetPresentation": "FULL", "audioMode": "STEREO", "audioQuality": "LOSSLESS", "manifestMimeType": "application/vnd.tidal.bts", "manifest": bts_flac, "bitDepth": 16, "sampleRate": 44100});
                            let aac = || serde_json::json!({"trackId": id, "assetPresentation": "FULL", "audioMode": "STEREO", "audioQuality": "HIGH", "manifestMimeType": "application/dash+xml", "manifest": dash_aac});
                            let v = match (id.as_str(), asked.as_str()) {
                                ("1", _) => bts(),
                                ("2", _) => serde_json::json!({"trackId": 2, "assetPresentation": "FULL", "audioMode": "STEREO", "audioQuality": "HI_RES_LOSSLESS", "manifestMimeType": "application/dash+xml", "manifest": dash_flac, "bitDepth": 24, "sampleRate": 96000}),
                                ("3", "HI_RES_LOSSLESS") => aac(),
                                ("3", _) => bts(),
                                ("4", _) => aac(),
                                _ => return (StatusCode::NOT_FOUND, Json(serde_json::json!({"userMessage": "no such track"}))),
                            };
                            (StatusCode::OK, Json(v))
                        }
                    }
                }),
            )
            .route("/file.flac", get(move || { let f = flac.clone(); async move { f } }))
            .route("/seg/{n}", get(|Path(n): Path<String>| async move { format!("[{n}]") }));
        tokio::spawn(async move { axum::serve(listener, mock).await.unwrap() });
        (base, seen)
    }

    fn point_tidal_at(s: &NodeState, base: &str, browser: bool) {
        let mut ep = s.tidal.write().unwrap();
        ep.auth_base = base.to_string();
        ep.login_base = base.to_string();
        ep.v1_base = format!("{base}/v1");
        ep.browser_client = browser.then(|| tidal_unofficial::BrowserClient {
            id: FAKE_BROWSER_ID.into(),
            secret: FAKE_BROWSER_SECRET.into(),
        });
    }

    /// Opens a TIDAL track the full-tracks way on the audio thread's path and records the health,
    /// with the test's log capture; the extension and the bytes read, or the error.
    async fn open_full(
        s: &Arc<NodeState>,
        who: accounts::Who,
        id: &'static str,
        logs: &tracing::Dispatch,
    ) -> Result<Option<(&'static str, bool, Vec<u8>)>, String> {
        let s = s.clone();
        let logs = logs.clone();
        tokio::task::spawn_blocking(move || {
            tracing::dispatcher::with_default(&logs, || {
                use std::io::Read;
                let ep = s.tidal.read().unwrap().clone();
                let r = crate::tidal_unofficial::open_blocking(&s.library, &ep, id, &who);
                crate::tidal_unofficial::record(&s.library, &who, &r);
                match r {
                    Ok(Some(mut f)) => {
                        let mut b = Vec::new();
                        f.reader.read_to_end(&mut b).unwrap();
                        Ok(Some((f.ext, f.lossy, b)))
                    }
                    Ok(None) => Ok(None),
                    Err(crate::tidal_unofficial::FullError::Mode(e)) => Err(format!("mode: {e}")),
                    Err(crate::tidal_unofficial::FullError::Track(e)) => Err(format!("track: {e}")),
                }
            })
        })
        .await
        .unwrap()
    }

    async fn call_auth(
        app: &Router,
        method: &str,
        uri: &str,
        token: &str,
    ) -> (StatusCode, serde_json::Value) {
        let req = Request::builder()
            .method(method)
            .uri(uri)
            .header("authorization", format!("Bearer {token}"))
            .body(Body::empty())
            .unwrap();
        let r = app.clone().oneshot(req).await.unwrap();
        let st = r.status();
        let b = r.into_body().collect().await.unwrap().to_bytes();
        (st, serde_json::from_slice(&b).unwrap_or_default())
    }

    /// A stand-in for the bundled homeserver's admin API: `GET /signup_tokens` answers `used`, and
    /// the first such call waits (after waking `held`) until `release`, so a test picks which
    /// roster change stores last. `/info` is down.
    async fn held_admin(
        used: serde_json::Value,
    ) -> (
        bundled::AdminEndpoint,
        Arc<tokio::sync::Notify>,
        Arc<tokio::sync::Notify>,
    ) {
        let held = Arc::new(tokio::sync::Notify::new());
        let release = Arc::new(tokio::sync::Notify::new());
        let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let tokens = {
            let (held, release) = (held.clone(), release.clone());
            move || {
                let (held, release, calls, used) =
                    (held.clone(), release.clone(), calls.clone(), used.clone());
                async move {
                    if calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst) == 0 {
                        held.notify_one();
                        release.notified().await;
                    }
                    Json(used)
                }
            }
        };
        let app = Router::new()
            .route("/signup_tokens", get(tokens))
            .route("/info", get(|| async { StatusCode::SERVICE_UNAVAILABLE }));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        (
            bundled::AdminEndpoint::new(&url, "test-admin-password").unwrap(),
            held,
            release,
        )
    }

    async fn as_who(
        app: &Router,
        method: &str,
        uri: String,
        token: &str,
        body: Option<serde_json::Value>,
    ) -> (StatusCode, serde_json::Value) {
        let req = Request::builder()
            .method(method)
            .uri(uri)
            .header("authorization", format!("Bearer {token}"))
            .header("content-type", "application/json")
            .body(
                body.map(|b| Body::from(b.to_string()))
                    .unwrap_or_else(Body::empty),
            )
            .unwrap();
        let resp = app.clone().oneshot(req).await.unwrap();
        let st = resp.status();
        let bytes = resp.into_body().collect().await.unwrap().to_bytes();
        (
            st,
            serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null),
        )
    }

    const ROSTER_KEY: &str = "tbepfp8fisyiqpskbmxsmpub47gxb7xdsh5x9ut9qrb97gcq9phy";
    const INVITE_TOKEN: &str = "FIXT-URE0-TOKN";

    /// A node behind a token with one listener (music on) and one unused homeserver invite.
    async fn roster_node(
        dir: &std::path::Path,
        used: serde_json::Value,
    ) -> (
        Arc<NodeState>,
        Router,
        Arc<tokio::sync::Notify>,
        Arc<tokio::sync::Notify>,
    ) {
        let s = state(dir, Some("s3cret")).await;
        let (ep, held, release) = held_admin(used).await;
        s.set_bundled_homeserver(ep);
        s.library
            .lock()
            .unwrap()
            .set_setting(
                "people",
                &serde_json::json!({
                    "people": [{"key": ROSTER_KEY, "access": "homeserver_music", "music_enabled": true, "added_at": 1}],
                    "invites": [{"id": "i1", "access": "homeserver_music", "token": INVITE_TOKEN, "created_at": 1}],
                })
                .to_string(),
            )
            .unwrap();
        let app = router(s.clone());
        (s, app, held, release)
    }

    /// Artist release sections (their own file, so parallel work doesn't meet at one anchor).
    mod release_sections;

    /// Links on the node in a queue never carry a sign-in (their own file).
    mod queue_links;

    /// A renderer's description (Linn-like, `udn`), served on 127.0.0.1 and counting its fetches;
    /// 503 until `ready` (a device not answering yet, on a port held all along).
    async fn fake_renderer_at(
        listener: tokio::net::TcpListener,
        udn: &'static str,
        name: &'static str,
        hits: Arc<std::sync::atomic::AtomicUsize>,
        ready: Arc<std::sync::atomic::AtomicBool>,
    ) {
        let desc = format!(
            r#"<?xml version="1.0"?>
<root xmlns="urn:schemas-upnp-org:device-1-0">
  <device>
    <deviceType>urn:schemas-upnp-org:device:MediaRenderer:1</deviceType>
    <friendlyName>{name}</friendlyName>
    <manufacturer>Linn Products Ltd.</manufacturer>
    <UDN>{udn}</UDN>
    <serviceList>
      <service>
        <serviceType>urn:schemas-upnp-org:service:AVTransport:1</serviceType>
        <serviceId>urn:upnp-org:serviceId:AVTransport</serviceId>
        <controlURL>/AVTransport/control</controlURL>
      </service>
    </serviceList>
  </device>
</root>"#
        );
        let mock = Router::new().route(
            "/desc.xml",
            get(move || {
                let desc = desc.clone();
                let ok = ready.load(std::sync::atomic::Ordering::SeqCst);
                if ok {
                    hits.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                }
                async move {
                    if ok {
                        ([(header::CONTENT_TYPE, "text/xml")], desc).into_response()
                    } else {
                        StatusCode::SERVICE_UNAVAILABLE.into_response()
                    }
                }
            }),
        );
        tokio::spawn(async move { axum::serve(listener, mock).await.unwrap() });
    }

    /// [`fake_renderer_at`] that says when a request has come (`arrived`) and answers it only once
    /// let go (`release`): a description provably on its way while something else happens.
    async fn fake_renderer_gated(
        listener: tokio::net::TcpListener,
        udn: &'static str,
        name: &'static str,
        arrived: Arc<tokio::sync::Notify>,
        release: Arc<tokio::sync::Notify>,
    ) {
        let desc = format!(
            r#"<?xml version="1.0"?>
<root xmlns="urn:schemas-upnp-org:device-1-0">
  <device>
    <deviceType>urn:schemas-upnp-org:device:MediaRenderer:1</deviceType>
    <friendlyName>{name}</friendlyName>
    <UDN>{udn}</UDN>
    <serviceList>
      <service>
        <serviceType>urn:schemas-upnp-org:service:AVTransport:1</serviceType>
        <serviceId>urn:upnp-org:serviceId:AVTransport</serviceId>
        <controlURL>/AVTransport/control</controlURL>
      </service>
    </serviceList>
  </device>
</root>"#
        );
        let mock = Router::new().route(
            "/desc.xml",
            get(move || {
                let desc = desc.clone();
                let arrived = arrived.clone();
                let release = release.clone();
                async move {
                    arrived.notify_one();
                    release.notified().await;
                    ([(header::CONTENT_TYPE, "text/xml")], desc)
                }
            }),
        );
        tokio::spawn(async move { axum::serve(listener, mock).await.unwrap() });
    }

    /// A zone's output that notes every volume request it is given.
    struct NotedVolume {
        inner: NullOutput,
        asked: std::sync::Mutex<Vec<pm_model::Volume>>,
    }

    #[async_trait::async_trait]
    impl pm_zones::Output for NotedVolume {
        fn id(&self) -> OutputId {
            self.inner.id()
        }
        fn name(&self) -> String {
            self.inner.name()
        }
        fn caps(&self) -> &pm_model::OutputCaps {
            self.inner.caps()
        }
        async fn open(
            &self,
            fmt: pm_model::AudioFormat,
        ) -> Result<Box<dyn pm_zones::AudioSink>, pm_zones::SinkError> {
            self.inner.open(fmt).await
        }
        async fn set_volume(&self, v: pm_model::Volume) -> Result<(), pm_zones::SinkError> {
            self.asked.lock().unwrap().push(v);
            self.inner.set_volume(v).await
        }
        async fn volume(&self) -> Result<Option<pm_model::ZoneVolume>, pm_zones::SinkError> {
            self.inner.volume().await
        }
    }

    /// A fake web player for the Qobuz tests below: the bundle carries a wrong secret first, then
    /// each of `secrets`.
    fn qobuz_mock_bundle(secrets: &[&str]) -> String {
        use base64::Engine;
        let piece = |secret: &str, zone: &str| {
            let padded = format!(
                "{}{}",
                base64::engine::general_purpose::STANDARD.encode(secret),
                "A".repeat(44)
            );
            let (seed, rest) = padded.split_at(8);
            let (info, extras) = rest.split_at(8);
            format!("a.initialSeed(\"{seed}\",window.utimezone.{});name:\"Region/{zone}\",info:\"{info}\",extras:\"{extras}\";", zone.to_ascii_lowercase())
        };
        let zones = ["First", "Second", "Third"];
        format!(
            "production:{{api:{{appId:\"100000000\",appSecret:\"{}\"}}}};{}",
            "a".repeat(32),
            std::iter::once("wrongSecret")
                .chain(secrets.iter().copied())
                .zip(zones)
                .map(|(secret, zone)| piece(secret, zone))
                .collect::<String>()
        )
    }

    // Automatic matching, automatic settling and "Wrong match?" (their own file)
    mod identify_auto;

    /// An 8-item queue playing on the null output with repeat one, `index` current; the zone.
    async fn looping_queue(
        n: usize,
        index: usize,
    ) -> (tempfile::TempDir, Arc<NodeState>, axum::Router, String) {
        let dir = tempfile::tempdir().unwrap();
        let wav = dir.path().join("t.wav");
        write_wav(&wav, 441);
        let s = state(dir.path(), None).await;
        let app = router(s.clone());
        let (_, zones) = call(&app, "GET", "/v1/zones", None).await;
        let zid = zones[0]["id"].as_str().unwrap().to_string();
        let items: Vec<_> = (0..n)
            .map(|i| serde_json::json!({"uri": wav.to_string_lossy(), "title": format!("t{i}")}))
            .collect();
        let (st, _) = call(
            &app,
            "POST",
            &format!("/v1/zones/{zid}/queue"),
            Some(serde_json::json!({"items": items, "replace": true})),
        )
        .await;
        assert_eq!(st, StatusCode::OK);
        call(
            &app,
            "PUT",
            &format!("/v1/zones/{zid}/repeat"),
            Some(serde_json::json!({"repeat": "one"})),
        )
        .await;
        call(
            &app,
            "POST",
            &format!("/v1/zones/{zid}/transport"),
            Some(serde_json::json!({"action": "play", "index": index})),
        )
        .await;
        let zone = s.zones.get(&ZoneId::new(&zid)).unwrap();
        for _ in 0..500 {
            if zone.state().now_playing.is_some_and(|n| n.index == index) {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        assert_eq!(zone.state().now_playing.map(|n| n.index), Some(index));
        (dir, s, app, zid)
    }

    fn queue_titles(z: &serde_json::Value) -> Vec<String> {
        z["queue"]
            .as_array()
            .unwrap()
            .iter()
            .map(|i| i["title"].as_str().unwrap_or_default().to_string())
            .collect()
    }

    /// A ListenBrainz mock whose submit answers only once let through (`gate`), takes nothing
    /// whose payload holds a listen at `listened_at == 0`, and keeps what it took.
    async fn gated_listenbrainz(
        s: &NodeState,
    ) -> (
        Arc<tokio::sync::Semaphore>,
        Arc<std::sync::Mutex<Vec<serde_json::Value>>>,
        Arc<tokio::sync::Semaphore>,
    ) {
        let gate = Arc::new(tokio::sync::Semaphore::new(0));
        let entered = Arc::new(tokio::sync::Semaphore::new(0));
        let got: Arc<std::sync::Mutex<Vec<serde_json::Value>>> = Default::default();
        // The account the senders send for (a sender checks it is still the one connected)
        s.library
            .lock()
            .unwrap()
            .set_setting(
                "listenbrainz.account",
                &serde_json::json!({"token": "LBT", "user": "owner", "submit": true}).to_string(),
            )
            .unwrap();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        *s.listenbrainz_base.write().unwrap() =
            format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());
        let mock = Router::new()
            .route(
                "/1/submit-listens",
                post({
                    let (gate, got, entered) = (gate.clone(), got.clone(), entered.clone());
                    move |h: axum::http::HeaderMap, Json(b): Json<serde_json::Value>| {
                        let (gate, got, entered) = (gate.clone(), got.clone(), entered.clone());
                        async move {
                            entered.add_permits(1);
                            let Ok(permit) = gate.acquire().await else {
                                return StatusCode::SERVICE_UNAVAILABLE;
                            };
                            permit.forget();
                            let payload = b["payload"].as_array().unwrap().clone();
                            if payload.iter().any(|l| l["listened_at"] == 0) {
                                return StatusCode::SERVICE_UNAVAILABLE;
                            }
                            // Which account it went to, with each listen
                            let token = h
                                .get("authorization")
                                .and_then(|v| v.to_str().ok())
                                .unwrap_or("")
                                .to_string();
                            got.lock().unwrap().extend(payload.into_iter().map(|mut l| {
                                l["_to"] = serde_json::json!(token);
                                l
                            }));
                            StatusCode::OK
                        }
                    }
                }),
            )
            .route(
                "/1/validate-token",
                get(|| async { Json(serde_json::json!({"valid": true, "user_name": "fixture"})) }),
            );
        tokio::spawn(async move { axum::serve(listener, mock).await.unwrap() });
        (gate, got, entered)
    }

    fn a_listen(n: u64) -> serde_json::Value {
        serde_json::json!({"listened_at": n, "track_metadata": {"artist_name": "A", "track_name": format!("T{n}")}})
    }

    fn waiting_listens(s: &NodeState) -> Vec<u64> {
        let v: Vec<serde_json::Value> = s
            .library
            .lock()
            .unwrap()
            .get_setting("listenbrainz.pending")
            .unwrap()
            .map(|j| serde_json::from_str(&j).unwrap())
            .unwrap_or_default();
        v.iter().map(|l| l["listened_at"].as_u64().unwrap()).collect()
    }

    /// The library refuses `key`'s writes until the returned triggers are dropped.
    fn refuse_writes_of(dir: &std::path::Path, key: &str) -> rusqlite::Connection {
        let db = rusqlite::Connection::open(dir.join("lib.sqlite")).unwrap();
        db.execute_batch(&format!(
            "CREATE TRIGGER no_pending_i BEFORE INSERT ON settings WHEN NEW.key = '{key}'
             BEGIN SELECT RAISE(ABORT, 'disk I/O error'); END;
             CREATE TRIGGER no_pending_u BEFORE UPDATE ON settings WHEN NEW.key = '{key}'
             BEGIN SELECT RAISE(ABORT, 'disk I/O error'); END;"
        ))
        .unwrap();
        db
    }

    fn writes_work_again(db: &rusqlite::Connection) {
        db.execute_batch("DROP TRIGGER no_pending_i; DROP TRIGGER no_pending_u;")
            .unwrap();
    }

    /// A Last.fm mock for this node: each `track.scrobble` answers once let through (`gate`),
    /// refused while `refuse` is set; the track names it took are in `got`; `entered` counts
    /// requests that arrived.
    struct LastfmMock {
        gate: Arc<tokio::sync::Semaphore>,
        got: Arc<std::sync::Mutex<Vec<String>>>,
        refuse: Arc<std::sync::atomic::AtomicBool>,
        entered: Arc<tokio::sync::Semaphore>,
    }

    async fn gated_lastfm(s: &NodeState) -> LastfmMock {
        let gate = Arc::new(tokio::sync::Semaphore::new(0));
        let entered = Arc::new(tokio::sync::Semaphore::new(0));
        let got: Arc<std::sync::Mutex<Vec<String>>> = Default::default();
        let refuse = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());
        let mock = Router::new().route(
            "/",
            post({
                let (gate, got, refuse, entered) =
                    (gate.clone(), got.clone(), refuse.clone(), entered.clone());
                move |body: String| {
                    let (gate, got, refuse, entered) =
                        (gate.clone(), got.clone(), refuse.clone(), entered.clone());
                    async move {
                        entered.add_permits(1);
                        gate.acquire().await.unwrap().forget();
                        if refuse.load(std::sync::atomic::Ordering::SeqCst) {
                            return Json(serde_json::json!({"error": 11, "message": "Service Offline"}));
                        }
                        let tracks: Vec<String> = url::form_urlencoded::parse(body.as_bytes())
                            .filter(|(k, _)| k.starts_with("track["))
                            .map(|(_, v)| v.into_owned())
                            .collect();
                        let n = tracks.len();
                        got.lock().unwrap().extend(tracks);
                        Json(serde_json::json!({"scrobbles": {"@attr": {"accepted": n, "ignored": 0}}}))
                    }
                }
            }),
        );
        tokio::spawn(async move { axum::serve(listener, mock).await.unwrap() });
        lastfm::set_api_base(&format!("{base}/"));
        let app = serde_json::json!({
            "api_key": "0123456789abcdef0123456789abcdef",
            "api_secret": "fedcba9876543210fedcba9876543210",
            "from": "settings"
        })
        .to_string();
        let lib = s.library.lock().unwrap();
        lib.set_setting(lastfm::APP_KEY, &app).unwrap();
        lib.set_setting(
            "lastfm.session",
            &serde_json::json!({"key": "sess-key-xyz", "user": "listener", "submit": true}).to_string(),
        )
        .unwrap();
        LastfmMock {
            gate,
            got,
            refuse,
            entered,
        }
    }

    fn a_scrobble(n: u64) -> serde_json::Value {
        serde_json::json!({"artist": "A", "track": format!("T{n}"), "timestamp": n.to_string()})
    }

    fn waiting_scrobbles(s: &NodeState) -> Vec<String> {
        let v: Vec<serde_json::Value> = s
            .library
            .lock()
            .unwrap()
            .get_setting("lastfm.pending")
            .unwrap()
            .map(|j| serde_json::from_str(&j).unwrap())
            .unwrap_or_default();
        v.iter()
            .map(|l| l["track"].as_str().unwrap().to_string())
            .collect()
    }

    fn lastfm_session(key: &str) -> lastfm::Session {
        lastfm::Session {
            key: key.into(),
            user: "listener".into(),
            submit: true,
            problem: None,
            from: None,
        }
    }

    /// Radio goes on when the queue runs out, not after a Stop (its own file).
    mod radio_stop;

    /// People, sign-ins, zones and the source policy as a 0.2.4 node stored them still work
    /// after the update: nobody is signed out, nothing stored is rewritten by reading it, and what
    /// is written keeps the fields the older version reads.
    #[tokio::test]
    async fn access_data_from_0_2_4_still_works() {
        const ANN: &str = "o3ftkuxjd9k6zgnc4hhbhbg5jusubr4zqyb1dtzi8hhm3hf5wbmy";
        const ANNS_TOKEN: &str = "pml_fixture-old-listener-token";
        let fixture: serde_json::Map<String, serde_json::Value> =
            serde_json::from_str(include_str!("../tests/fixtures/access-0.2.4-settings.json")).unwrap();
        let dir = tempfile::tempdir().unwrap();
        let s = state(dir.path(), Some("s3cret")).await;
        {
            let lib = s.library.lock().unwrap();
            for (key, value) in fixture.iter().filter(|(k, _)| !k.starts_with('_')) {
                lib.set_setting(key, value.as_str().unwrap()).unwrap();
            }
        }
        accounts::sign_ins_changed();
        let stored = |key: &str| s.library.lock().unwrap().get_setting(key).unwrap();
        let app = router(s.clone());

        // Ann's sign-in from before still signs her in, on the zones the owner opened
        let (st, me) = call_auth(&app, "GET", "/v1/me", ANNS_TOKEN).await;
        assert_eq!(st, StatusCode::OK, "{me}");
        assert_eq!(
            (me["role"].as_str(), me["key"].as_str()),
            (Some("listener"), Some(ANN)),
            "{me}"
        );
        assert_eq!(me["zones"], serde_json::json!(["null"]), "{me}");
        let can = |me: &serde_json::Value, cap: &str| me["can"].as_array().unwrap().iter().any(|c| c == cap);
        assert!(
            can(&me, "library") && can(&me, "own") && can(&me, "own_sign_in"),
            "{me}"
        );
        assert!(
            !can(&me, "people") && !can(&me, "tokens") && !can(&me, "party_guest"),
            "{me}"
        );
        // The API token is the owner, with everything
        let (st, me) = call_auth(&app, "GET", "/v1/me", "s3cret").await;
        assert_eq!((st, me["role"].as_str()), (StatusCode::OK, Some("owner")), "{me}");
        // (all but a party guest's own calls, which answer only a guest)
        assert_eq!(
            me["can"].as_array().unwrap().len(),
            access::Capability::ALL.len() - 1,
            "{me}"
        );
        assert!(!can(&me, "party_guest") && can(&me, "people"), "{me}");

        // People reads the old list whole
        let (st, all) = call_auth(&app, "GET", "/v1/people", "s3cret").await;
        assert_eq!(st, StatusCode::OK, "{all}");
        assert_eq!(all["people"].as_array().unwrap().len(), 3, "{all}");
        assert_eq!(all["invites"].as_array().unwrap().len(), 1, "{all}");
        let ann = &all["people"][0];
        assert_eq!(
            (ann["name"].as_str(), ann["homeserver_enabled"].as_bool()),
            (Some("Ann"), Some(true))
        );
        assert_eq!(ann["sign_ins"].as_array().unwrap().len(), 1, "{all}");
        assert_eq!(ann["sign_ins"][0]["created_at"], 1761481000_i64, "{all}");
        assert!(
            ann["sign_ins"][0]["id"].as_str().is_some_and(|id| id.len() == 16),
            "{all}"
        );
        // The owner's browser and app from before are listed, the newer first
        let (st, mine) = call_auth(&app, "GET", "/v1/me/sign-ins", "s3cret").await;
        assert_eq!(st, StatusCode::OK, "{mine}");
        let kinds: Vec<&str> = mine
            .as_array()
            .unwrap()
            .iter()
            .map(|x| x["kind"].as_str().unwrap())
            .collect();
        assert_eq!(kinds, ["app", "browser"], "{mine}");

        // Reading rewrote nothing
        for key in [
            "people",
            "accounts.owner_sessions",
            "accounts.device_sessions",
            "accounts.listener_zones",
            "sources.policy",
        ] {
            assert_eq!(stored(key).as_deref(), fixture[key].as_str(), "{key}");
        }
        // Ann's sign-in gained the day it was used; the fields 0.2.4 reads are as they were
        #[derive(serde::Deserialize)]
        struct AsBefore {
            hash: String,
            key: String,
            created_at: i64,
        }
        let now = stored("accounts.tokens").unwrap();
        let old: Vec<AsBefore> = serde_json::from_str(&now).unwrap();
        assert_eq!(old.len(), 1, "{now}");
        assert_eq!(
            (old[0].key.as_str(), old[0].created_at),
            (ANN, 1761481000),
            "{now}"
        );
        assert_eq!(
            old[0].hash,
            blake3::hash(ANNS_TOKEN.as_bytes()).to_hex().to_string()
        );
        assert!(now.contains("\"last_used\":"), "{now}");

        // A change to the people list keeps what the old rows held
        let (st, v) = as_who(
            &app,
            "PATCH",
            format!("/v1/people/{ANN}"),
            "s3cret",
            Some(serde_json::json!({"name": "Anna"})),
        )
        .await;
        assert_eq!(st, StatusCode::OK, "{v}");
        let roster: serde_json::Value = serde_json::from_str(&stored("people").unwrap()).unwrap();
        assert_eq!(roster["people"].as_array().unwrap().len(), 3, "{roster}");
        assert_eq!(roster["people"][0]["name"], "Anna");
        assert_eq!(roster["people"][0]["music_enabled"], true);
        assert_eq!(roster["people"][1]["reports"], "0123456789abcdef0123456789abcdef");
        assert_eq!(roster["people"][2]["access"], "homeserver");
        assert_eq!(roster["invites"][0]["token"], "AAAA-BBBB-CCCC");
        // And Ann is still signed in
        let (st, _) = call_auth(&app, "GET", "/v1/me", ANNS_TOKEN).await;
        assert_eq!(st, StatusCode::OK);
    }

    // The tests by area, one file each (tests/<area>.rs). A new test goes at the end of
    // its area's file; the helpers above are shared by all of them.
    mod accounts_people;
    mod discover_search;
    mod homeserver_copies;
    mod identify_match;
    mod library_core;
    mod node_ops;
    mod outputs_renderers;
    mod qobuz_flows;
    mod scrobblers;
    mod sign_in_ways;
    mod streaming_services;
    mod tidal_flows;
    mod zones_queue;
}
