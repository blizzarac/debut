//! Scenarist SCC captions (GFX-05): CEA-608 pop-on captions on channel CC1 at
//! 29.97 fps drop-frame timecode.
//!
//! Writing: each cue is loaded into non-displayed memory (RCL, ENM, a
//! preamble per row, the text) timed so its End Of Caption lands on the cue
//! start; Erase Displayed Memory clears it at the end unless the next cue
//! replaces it first. Control codes are doubled, as 608 decoders expect.
//! Rows hold 32 characters; text wraps on words into at most 4 rows at the
//! bottom of the screen.
//!
//! Reading runs a small 608 decoder over CC1: pop-on captions become cues;
//! roll-up and paint-on files are refused rather than half-read.

use debut_core::{Error, FrameRate, Rational, Result, Timecode};

/// One cue: start, end, text (rows joined with `\n`).
pub type Cue = (Rational, Rational, String);

const RATE: FrameRate = FrameRate::FPS_29_97;
const COLS: usize = 32;
const MAX_ROWS: usize = 4;

const RCL: [u8; 2] = [0x14, 0x20];
const ENM: [u8; 2] = [0x14, 0x2e];
const EDM: [u8; 2] = [0x14, 0x2c];
const EOC: [u8; 2] = [0x14, 0x2f];

fn parity(b: u8) -> u8 {
    let b = b & 0x7f;
    if b.count_ones().is_multiple_of(2) {
        b | 0x80
    } else {
        b
    }
}

fn word(pair: [u8; 2]) -> String {
    format!("{:02x}{:02x}", parity(pair[0]), parity(pair[1]))
}

/// Preamble address code for `row` (1..=15) at column 0, white.
fn pac(row: usize) -> [u8; 2] {
    const FIRST: [(u8, u8); 15] = [
        (0x11, 0x40),
        (0x11, 0x60),
        (0x12, 0x40),
        (0x12, 0x60),
        (0x15, 0x40),
        (0x15, 0x60),
        (0x16, 0x40),
        (0x16, 0x60),
        (0x17, 0x40),
        (0x17, 0x60),
        (0x10, 0x40),
        (0x13, 0x40),
        (0x13, 0x60),
        (0x14, 0x40),
        (0x14, 0x60),
    ];
    let (a, b) = FIRST[row.clamp(1, 15) - 1];
    [a, b + 0x10]
}

/// Row of a preamble address code, if `pair` is one on CC1.
fn pac_row(pair: [u8; 2]) -> Option<usize> {
    let (a, b) = (pair[0], pair[1]);
    if !(0x40..=0x7f).contains(&b) {
        return None;
    }
    let low = b < 0x60;
    Some(match (a, low) {
        (0x11, true) => 1,
        (0x11, false) => 2,
        (0x12, true) => 3,
        (0x12, false) => 4,
        (0x15, true) => 5,
        (0x15, false) => 6,
        (0x16, true) => 7,
        (0x16, false) => 8,
        (0x17, true) => 9,
        (0x17, false) => 10,
        (0x10, true) => 11,
        (0x13, true) => 12,
        (0x13, false) => 13,
        (0x14, true) => 14,
        (0x14, false) => 15,
        _ => return None,
    })
}

/// 608 byte for a character; the basic set is ASCII with a few accented
/// letters in place of rarely used symbols. Anything else becomes '?'.
fn encode_char(c: char) -> u8 {
    match c {
        'á' => 0x2a,
        'é' => 0x5c,
        'í' => 0x5e,
        'ó' => 0x5f,
        'ú' => 0x60,
        'ç' => 0x7b,
        '÷' => 0x7c,
        'Ñ' => 0x7d,
        'ñ' => 0x7e,
        '*' | '\\' | '^' | '_' | '`' | '{' | '|' | '}' | '~' => b'?',
        c if (' '..='\u{7e}').contains(&c) => c as u8,
        _ => b'?',
    }
}

fn decode_char(b: u8) -> char {
    match b {
        0x2a => 'á',
        0x5c => 'é',
        0x5e => 'í',
        0x5f => 'ó',
        0x60 => 'ú',
        0x7b => 'ç',
        0x7c => '÷',
        0x7d => 'Ñ',
        0x7e => 'ñ',
        0x7f => '█',
        b => b as char,
    }
}

/// Wrap `text` into at most `MAX_ROWS` rows of `COLS`, keeping its own line
/// breaks where they fit.
fn wrap(text: &str) -> Vec<String> {
    let mut rows = Vec::new();
    for line in text.lines() {
        let mut row = String::new();
        for w in line.split_whitespace() {
            let w: String = w.chars().take(COLS).collect();
            if !row.is_empty() && row.chars().count() + 1 + w.chars().count() > COLS {
                rows.push(std::mem::take(&mut row));
            }
            if !row.is_empty() {
                row.push(' ');
            }
            row.push_str(&w);
        }
        if !row.is_empty() {
            rows.push(row);
        }
    }
    rows.truncate(MAX_ROWS);
    rows
}

fn frame_of(t: Rational) -> i64 {
    (t * RATE.0).round().max(0)
}

fn timecode(frame: i64) -> String {
    Timecode::from_frames(frame, RATE, true).to_string()
}

/// Format cues as SCC.
pub fn format_scc<'a>(cues: impl IntoIterator<Item = (Rational, Rational, &'a str)>) -> String {
    let mut rows: Vec<(Rational, Rational, &str)> = cues.into_iter().collect();
    rows.sort_by_key(|c| c.0);
    let mut out = String::from("Scenarist_SCC V1.0\n\n");
    // First frame not yet used by an emitted line.
    let mut free = 0i64;
    for (i, (start, end, text)) in rows.iter().enumerate() {
        let lines = wrap(text);
        if lines.is_empty() {
            continue;
        }
        let mut words = vec![word(RCL), word(RCL), word(ENM), word(ENM)];
        let first_row = 16 - lines.len();
        for (r, line) in lines.iter().enumerate() {
            let p = word(pac(first_row + r));
            words.push(p.clone());
            words.push(p);
            let bytes: Vec<u8> = line.chars().map(encode_char).collect();
            for pair in bytes.chunks(2) {
                let b = if pair.len() == 2 { pair[1] } else { 0x00 };
                words.push(format!("{:02x}{:02x}", parity(pair[0]), parity(b)));
            }
        }
        words.push(word(EOC));
        words.push(word(EOC));
        // One word per frame; the last EOC lands on the cue start.
        let start_f = frame_of(*start);
        let load = (start_f - (words.len() as i64 - 1)).max(free);
        out.push_str(&format!("{}\t{}\n\n", timecode(load), words.join(" ")));
        let shown = load + words.len() as i64;
        free = shown;
        // Clear at the end unless the next cue takes over by then.
        let end_f = frame_of(*end).max(shown);
        let next_on = rows.get(i + 1).map(|n| frame_of(n.0));
        if next_on.is_none_or(|n| n > end_f) {
            out.push_str(&format!(
                "{}\t{} {}\n\n",
                timecode(end_f),
                word(EDM),
                word(EDM)
            ));
            free = end_f + 2;
        }
    }
    out
}

/// Parse SCC text (CC1, pop-on).
pub fn parse_scc(text: &str) -> Result<Vec<Cue>> {
    let bad = |m: String| Error::InvalidArgument(format!("SCC: {m}"));
    let mut lines = text.lines().map(str::trim).filter(|l| !l.is_empty());
    match lines.next() {
        Some(h) if h.starts_with("Scenarist_SCC") => {}
        _ => return Err(bad("missing the Scenarist_SCC header".into())),
    }
    let mut cues = Vec::new();
    let mut loading: Vec<(usize, String)> = Vec::new();
    let mut shown: Vec<(usize, String)> = Vec::new();
    let mut shown_at = 0i64;
    let mut row = 15usize;
    let mut last: Option<[u8; 2]> = None;
    let time = |f: i64| RATE.frame_to_time(f);
    let flush = |rows: &[(usize, String)]| -> String {
        let mut rows = rows.to_vec();
        rows.sort_by_key(|r| r.0);
        rows.iter()
            .map(|r| r.1.trim_end().to_string())
            .filter(|r| !r.is_empty())
            .collect::<Vec<_>>()
            .join("\n")
    };
    for line in lines {
        let (tc, data) = line
            .split_once(char::is_whitespace)
            .ok_or_else(|| bad(format!("bad line {line:?}")))?;
        let tc = Timecode::parse(tc).ok_or_else(|| bad(format!("bad timecode {tc:?}")))?;
        let base = tc.to_frames(RATE);
        for (i, w) in data.split_whitespace().enumerate() {
            let frame = base + i as i64;
            let v = u16::from_str_radix(w, 16).map_err(|_| bad(format!("bad word {w:?}")))?;
            let pair = [((v >> 8) as u8) & 0x7f, (v as u8) & 0x7f];
            let is_control = (0x10..=0x1f).contains(&pair[0]);
            if is_control {
                // Control codes are sent twice; act once.
                if last == Some(pair) {
                    last = None;
                    continue;
                }
                last = Some(pair);
            } else {
                last = None;
            }
            match pair {
                [0, 0] => {}
                // Channel 2 (and its field) is not ours.
                [a, _] if (0x18..=0x1f).contains(&a) => {}
                RCL => {}
                ENM => loading.clear(),
                EDM => {
                    let text = flush(&shown);
                    if !text.is_empty() {
                        cues.push((time(shown_at), time(frame), text));
                    }
                    shown.clear();
                }
                EOC => {
                    let text = flush(&shown);
                    if !text.is_empty() {
                        cues.push((time(shown_at), time(frame), text));
                    }
                    std::mem::swap(&mut shown, &mut loading);
                    loading.clear();
                    shown_at = frame;
                }
                [0x14, 0x25..=0x27] | [0x14, 0x29] => {
                    return Err(bad("roll-up and paint-on captions are not supported".into()))
                }
                // Backspace.
                [0x14, 0x21] => {
                    if let Some(r) = loading.iter_mut().find(|r| r.0 == row) {
                        r.1.pop();
                    }
                }
                // Tab offsets 1..3 columns.
                [0x17, b @ 0x21..=0x23] => {
                    let n = (b - 0x20) as usize;
                    text_row(&mut loading, row).push_str(&" ".repeat(n));
                }
                // Special characters: ♪ and a few others; the rest as '?'.
                [0x11, b @ 0x30..=0x3f] => {
                    let c = match b {
                        0x37 => '♪',
                        0x30 => '®',
                        0x31 => '°',
                        0x32 => '½',
                        0x33 => '¿',
                        0x34 => '™',
                        0x35 => '¢',
                        0x36 => '£',
                        0x38 => 'à',
                        0x39 => ' ',
                        0x3a => 'è',
                        0x3b => 'â',
                        0x3c => 'ê',
                        0x3d => 'î',
                        0x3e => 'ô',
                        _ => 'û',
                    };
                    text_row(&mut loading, row).push(c);
                }
                p if pac_row(p).is_some() => {
                    row = pac_row(p).unwrap();
                    text_row(&mut loading, row);
                }
                // Other controls (mid-row styles, extended sets): no text.
                _ if is_control => {}
                [a, b] => {
                    let s = text_row(&mut loading, row);
                    for c in [a, b] {
                        if c >= 0x20 {
                            s.push(decode_char(c));
                        }
                    }
                }
            }
        }
    }
    Ok(cues)
}

fn text_row(rows: &mut Vec<(usize, String)>, row: usize) -> &mut String {
    let i = match rows.iter().position(|r| r.0 == row) {
        Some(i) => i,
        None => {
            rows.push((row, String::new()));
            rows.len() - 1
        }
    };
    &mut rows[i].1
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(x: f64) -> Rational {
        RATE.frame_to_time((x * RATE.0.as_f64()).round() as i64)
    }

    #[test]
    fn control_words_have_odd_parity() {
        assert_eq!(word(RCL), "9420");
        assert_eq!(word(ENM), "94ae");
        assert_eq!(word(EDM), "942c");
        assert_eq!(word(EOC), "942f");
        assert_eq!(word(pac(15)), "9470");
        assert_eq!(word(pac(14)), "94d0");
        assert_eq!(pac_row(pac(13)), Some(13));
    }

    #[test]
    fn round_trips_pop_on_cues() {
        let cues = [
            (s(1.0), s(3.0), "Hello, world"),
            (
                s(3.0),
                s(5.0),
                "Second caption with enough words to wrap onto two rows",
            ),
            (s(7.0), s(8.0), "Señor ñ"),
        ];
        let text = format_scc(cues.iter().copied());
        assert!(text.starts_with("Scenarist_SCC V1.0\n\n"));
        // The first load is timed so EOC lands on 1 s (frame 30).
        assert!(text.contains("\t9420 9420 94ae 94ae 9470 9470"), "{text}");
        let back = parse_scc(&text).unwrap();
        assert_eq!(back.len(), 3, "{back:?}");
        let frame = RATE.frame_duration();
        for ((a0, a1, at), (b0, b1, bt)) in cues.iter().zip(&back) {
            assert!((*a0 - *b0).as_f64().abs() <= frame.as_f64(), "{a0} vs {b0}");
            assert!(
                (*a1 - *b1).as_f64().abs() <= 3.0 * frame.as_f64(),
                "{a1} vs {b1}"
            );
            assert_eq!(
                at.split_whitespace().collect::<Vec<_>>(),
                bt.split_whitespace().collect::<Vec<_>>()
            );
        }
        assert!(back[1].2.contains('\n'), "wrapped to two rows");
    }

    #[test]
    fn reads_a_hand_written_file_and_refuses_roll_up() {
        // "HI" on row 15 shown at 00:00:01;00, cleared at 00:00:02;00.
        let scc = "Scenarist_SCC V1.0\n\n\
            00:00:00;29\t9420 9420 94ae 94ae 9470 9470 c849 942f 942f\n\n\
            00:00:02;00\t942c 942c\n";
        let cues = parse_scc(scc).unwrap();
        assert_eq!(cues.len(), 1);
        assert_eq!(cues[0].2, "HI");
        assert_eq!(cues[0].0, RATE.frame_to_time(29 + 7));
        assert_eq!(cues[0].1, RATE.frame_to_time(60));
        assert!(parse_scc("Scenarist_SCC V1.0\n\n00:00:00;00\t9425 9425\n").is_err());
        assert!(parse_scc("WEBVTT\n").is_err());
    }
}
