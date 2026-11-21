//! `GET /v1/search?q=&in=`: the node's search answer from the phone's sources, one NDJSON line per
//! source as it answers (`pm_sources::search::SourceAnswer`), with `x-pm-sources` naming the ones
//! that will.

use super::{qobuz, tidal, Phone};
use axum::body::Body;
use axum::extract::{Query, State};
use axum::http::header;
use axum::response::{IntoResponse, Response};
use pm_sources::search::{hit_of, SourceAnswer};
use pm_sources::types::SourceHit;
use serde::Deserialize;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// How long one source may take before it is reported as not answering (as on the node).
const SOURCE_TIMEOUT: Duration = Duration::from_secs(8);

#[derive(Deserialize)]
pub(crate) struct Params {
    #[serde(default)]
    q: String,
    #[serde(default, rename = "in")]
    scope: Option<String>,
    #[serde(default)]
    limit: Option<u32>,
}

/// The phone's sources that answer a search in `scope` now.
pub(crate) fn sources_for(p: &Phone, scope: &str) -> Vec<&'static str> {
    [("tidal", tidal::streams(p)), ("qobuz", qobuz::streams(p))]
        .into_iter()
        .filter(|(id, on)| *on && (scope == "everywhere" || scope == *id))
        .map(|(id, _)| id)
        .collect()
}

async fn one(p: &Arc<Phone>, source: &str, q: &str, limit: u32) -> Result<Vec<SourceHit>, String> {
    match source {
        "tidal" => {
            let sess = tidal::session(p).await.map_err(|e| e.message)?;
            pm_sources::tidal::catalog::search(&p.http, &p.tidal.v1_base, &sess, q, limit)
                .await
                .map_err(|e| e.message)
        }
        "qobuz" => {
            let (app, sess) = qobuz::ready(p).map_err(|e| e.message)?;
            pm_sources::qobuz::search(&p.http, &app, &sess.token, q, limit)
                .await
                .map_err(|e| qobuz::failed(p, e).message)
        }
        _ => Err("not on this phone".into()),
    }
}

/// One source's line.
async fn answer(p: Arc<Phone>, source: &'static str, q: String, limit: u32) -> SourceAnswer {
    let started = Instant::now();
    let r = tokio::time::timeout(SOURCE_TIMEOUT, one(&p, source, &q, limit)).await;
    let ms = started.elapsed().as_millis() as u64;
    let (status, error, hits) = match r {
        Ok(Ok(hits)) => (
            "ok",
            None,
            hits.iter()
                .filter_map(|h| hit_of(h, pm_sources::pages::page_id))
                .collect(),
        ),
        Ok(Err(e)) => ("error", Some(e), Vec::new()),
        Err(_) => ("timeout", Some(format!("{source} took too long")), Vec::new()),
    };
    SourceAnswer {
        source: source.into(),
        status: status.into(),
        ms,
        error,
        hits,
        counts: Vec::new(),
        cached: false,
    }
}

pub(crate) async fn search(State(p): State<Arc<Phone>>, Query(q): Query<Params>) -> Response {
    let scope = q.scope.unwrap_or_else(|| "everywhere".into());
    let term = q.q.trim().to_string();
    let sources = if term.is_empty() {
        Vec::new()
    } else {
        sources_for(&p, &scope)
    };
    let limit = q.limit.unwrap_or(20).clamp(1, 50);
    let announced = sources.join(",");
    let (tx, rx) = tokio::sync::mpsc::channel::<String>(4);
    for source in sources {
        let (p, tx, term) = (p.clone(), tx.clone(), term.clone());
        tokio::spawn(async move {
            let a = answer(p, source, term, limit).await;
            let mut line = serde_json::to_string(&a).unwrap_or_default();
            line.push('\n');
            let _ = tx.send(line).await;
        });
    }
    drop(tx);
    let lines = futures_util::stream::unfold(rx, |mut rx| async move {
        rx.recv()
            .await
            .map(|l| (Ok::<_, std::convert::Infallible>(l), rx))
    });
    (
        [
            (header::CONTENT_TYPE, "application/x-ndjson".to_string()),
            (header::CACHE_CONTROL, "no-cache".to_string()),
            (header::HeaderName::from_static("x-pm-sources"), announced),
        ],
        Body::from_stream(lines),
    )
        .into_response()
}
