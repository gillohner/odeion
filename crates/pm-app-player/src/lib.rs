//! The Odeion app's own player: what the page plays in the app ("This phone"), decoded and played
//! here through this device's output, as the phone's own zone would play it, instead of through
//! the web view's `<audio>` (which always goes through Android's mixer).
//!
//! * [`range`]: the page's URL as a seekable byte stream (HTTP ranges).
//! * [`source`]: those streams decoded with pm-audio and planned like the node plans a zone.
//! * [`engine`]: a `pm_zones::Zone` on the device's output behind the page's `<audio>`-like
//!   surface, the output taken from (and given back to) the node's session on it.

pub mod engine;
pub mod range;
pub mod source;

pub use engine::{AppPlayer, Notify, Playing, Status, TAKEN_BY_NODE};
pub use pm_zones::{OutputId, Transport};
