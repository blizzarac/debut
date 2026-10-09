//! Titles, graphics, captions and AI assistance (GFX-01 .. GFX-11).
//! AI runs on-device by default; cloud is explicit opt-in per project (GFX-11).

pub mod text;        // GFX-01 text layout, styling, keyframed animation
pub mod templates;   // GFX-02 title / lower-third templates
pub mod shapes;      // GFX-03
pub mod transcribe;  // GFX-04 speech-to-text, speaker labels (feeds TL-13, MED-08)
pub mod captions;    // GFX-05, GFX-06 edit, style, burn-in, SRT/VTT/SCC export
pub mod ai;          // GFX-07 .. GFX-10, FX-14, AUD-07 model host and opt-in policy
