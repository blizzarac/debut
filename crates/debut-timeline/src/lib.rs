//! Timeline logic (TL-01 .. TL-16). Produces `Command`s from edit intents; the
//! primitives themselves live in `debut-command` so every edit is undoable.
//! Must stay under 16 ms per interaction with 1,000+ clips (NFR-02).

pub mod edit; // TL-03 three- and four-point editing
pub mod trim; // TL-04 ripple / roll / slip / slide

pub mod evaluate; // sequence -> frame composition plan (pending; see debut-render::compose)
pub mod select; // TL-05 track targeting, patching, linked selection (pending)
pub mod shortcuts; // TL-12 keymaps with NLE presets (pending)
pub mod snap; // TL-06 snap targets, gap removal
pub mod snapshot; // TL-14 versions and compare (pending)
pub mod speed; // TL-09 constant, reverse, freeze, ramps

pub use edit::{three_point, EditMode, EditPoints};
pub use snap::{close_gaps, snap_targets, SnapKind, SnapTarget};
pub use speed::{set_ramp, set_speed};
pub use trim::{ripple_head, ripple_tail, roll, slide, slip};
