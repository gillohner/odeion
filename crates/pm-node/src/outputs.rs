//! Output discovery for this node.

use crate::config::OutputsConfig;
use pm_zones::{outputs::NullOutput, OutputRegistry};
use std::sync::Arc;

/// Local outputs only: the null output always, ALSA `hw:` devices when
/// built with the `alsa` feature.
pub fn discover() -> OutputRegistry {
    let registry = OutputRegistry::new();
    registry.register(Arc::new(NullOutput::from_env()));
    // Developer outputs with a pretend volume (dB and percent), for the pages' volume checks;
    // hidden with the Null output unless developer outputs are switched on
    registry.register(Arc::new(NullOutput::with_volume(pm_model::VolumeKind::Db {
        min: -60.0,
        max: 0.0,
        step: 1.0,
    })));
    registry.register(Arc::new(NullOutput::with_volume(pm_model::VolumeKind::Percent)));
    #[cfg(feature = "alsa")]
    for out in pm_out_local::alsa::AlsaOutput::enumerate() {
        registry.register(Arc::new(out));
    }
    registry
}

/// Local outputs. Endpoints (the desktop app's player) are never discovered or configured: each
/// answers only the node holding its key, so the app registers it (`POST /v1/outputs/endpoints`).
pub fn discover_all(cfg: &OutputsConfig) -> OutputRegistry {
    if !cfg.endpoints.is_empty() {
        tracing::warn!(
            "[outputs] endpoints is no longer used: the Odeion app registers its player itself; \
             speakers on a Raspberry Pi run squeezelite"
        );
    }
    discover()
}
