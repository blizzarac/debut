//! Titles, graphics and captions (GFX-01 .. GFX-06). Transcription runs in
//! the platform's on-device AI host (`debut_platform::ai`, GFX-04).

pub mod captions; // GFX-05, GFX-06 SRT/VTT import and export
pub mod scc; // GFX-05 Scenarist SCC (CEA-608 pop-on) import and export
pub mod shapes; // GFX-03
pub mod templates; // GFX-02 title / lower-third templates
pub mod text; // GFX-01 text layout and rasterization

pub use captions::{format_srt, format_vtt, parse_srt, Cue};
pub use scc::{format_scc, parse_scc};
pub use templates::{build as build_title_template, Built as BuiltTitle, Template, TEMPLATES};
pub use text::{render as render_title, Font, Raster};
