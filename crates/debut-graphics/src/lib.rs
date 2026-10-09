//! Titles, graphics, captions and AI assistance (GFX-01 .. GFX-11).
//! AI runs on-device by default; cloud is explicit opt-in per project (GFX-11).

pub mod ai;
pub mod captions; // GFX-05, GFX-06 SRT/VTT import and export
pub mod scc; // GFX-05 Scenarist SCC (CEA-608 pop-on) import and export
pub mod shapes; // GFX-03
pub mod templates; // GFX-02 title / lower-third templates
pub mod text; // GFX-01 text layout and rasterization
pub mod transcribe; // GFX-04 speech-to-text, speaker labels (feeds TL-13, MED-08) // GFX-07 .. GFX-10, FX-14, AUD-07 model host and opt-in policy

pub use captions::{format_srt, format_vtt, parse_srt, Cue};
pub use scc::{format_scc, parse_scc};
pub use templates::{build as build_title_template, Built as BuiltTitle, Template, TEMPLATES};
pub use text::{render as render_title, Font, Raster};
