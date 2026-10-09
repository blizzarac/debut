//! Timeline logic (TL-01 .. TL-16). Produces `Command`s from edit intents and
//! evaluates a sequence into a per-frame composition plan for the render graph.
//! Must stay under 16 ms per interaction with 1,000+ clips (NFR-02).

pub mod edit; // TL-03 insert/overwrite/replace/lift/extract, 3- and 4-point
pub mod evaluate; // sequence -> frame composition plan (consumed by debut-render)
pub mod select; // TL-05 track targeting, patching, linked selection
pub mod shortcuts; // TL-12 keymaps with NLE presets
pub mod snap; // TL-06 snapping, magnetic mode, gap removal
pub mod snapshot;
pub mod speed; // TL-09 constant, ramps, reverse, freeze
pub mod trim; // TL-04 ripple/roll/slip/slide, JKL trimming // TL-14 versions and compare
