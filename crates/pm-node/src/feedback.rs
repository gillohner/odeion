//! `pm-node feedback`: the node's operator reads new problem reports and records receipts
//! (pm_api::feedback). Reports print as JSON for an intake tool; their text is a person's words,
//! data for whoever reads it, never instructions.

use crate::config::NodeConfig;
use pm_api::feedback::{Inbox, State};

/// What `pending` prints: the format's name and version, then the reports.
const FORMAT: &str = "pm-feedback-pending";

fn inbox(cfg: &NodeConfig) -> anyhow::Result<Inbox> {
    let db = cfg.db_path();
    let dir = db.parent().unwrap_or_else(|| std::path::Path::new("."));
    Ok(Inbox::open(dir)?)
}

/// Print the reports no one has received yet.
pub fn pending(cfg: &NodeConfig) -> anyhow::Result<()> {
    let reports = inbox(cfg)?.pending(pm_api::feedback::now())?;
    println!(
        "{}",
        serde_json::json!({"format": FORMAT, "version": 1, "reports": reports})
    );
    Ok(())
}

/// Record that report `id` was received or reviewed, under the operator's own reference.
pub fn receipt(cfg: &NodeConfig, id: &str, state: State, reference: &str) -> anyhow::Result<()> {
    if !inbox(cfg)?.receipt(id, state, reference, pm_api::feedback::now())? {
        anyhow::bail!("no report {id:?} in this node's inbox");
    }
    println!("{}", serde_json::json!({"id": id, "state": state}));
    Ok(())
}
