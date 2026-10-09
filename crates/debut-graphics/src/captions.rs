//! SubRip (.srt) import and export for captions (GFX-06). Times are kept as
//! exact millisecond rationals; blank lines separate cues, numbering is
//! regenerated on output.

use debut_core::{Error, Rational, Result};

/// One parsed cue: start, end, text (lines joined with `\n`).
pub type Cue = (Rational, Rational, String);

fn parse_time(s: &str) -> Option<Rational> {
    // HH:MM:SS,mmm (also accepts '.' and a missing millisecond part)
    let s = s.trim();
    let (hms, ms) = match s.split_once([',', '.']) {
        Some((a, b)) => (a, b),
        None => (s, "0"),
    };
    let mut parts = hms.split(':').rev();
    let sec: i64 = parts.next()?.trim().parse().ok()?;
    let min: i64 = parts.next().map_or(Some(0), |p| p.trim().parse().ok())?;
    let hour: i64 = parts.next().map_or(Some(0), |p| p.trim().parse().ok())?;
    let ms: i64 = format!("{:0<3}", ms.trim()).get(..3)?.parse().ok()?;
    Some(Rational::new(
        ((hour * 60 + min) * 60 + sec) * 1000 + ms,
        1000,
    ))
}

fn format_time_sep(t: Rational, sep: char) -> String {
    let ms = (t * Rational::from_int(1000)).round().max(0);
    format!(
        "{:02}:{:02}:{:02}{sep}{:03}",
        ms / 3_600_000,
        ms / 60_000 % 60,
        ms / 1000 % 60,
        ms % 1000
    )
}

fn format_time(t: Rational) -> String {
    format_time_sep(t, ',')
}

/// Format cues as WebVTT (GFX-06).
pub fn format_vtt<'a>(cues: impl IntoIterator<Item = (Rational, Rational, &'a str)>) -> String {
    let mut rows: Vec<(Rational, Rational, &str)> = cues.into_iter().collect();
    rows.sort_by_key(|c| c.0);
    let mut out = String::from("WEBVTT\n\n");
    for (start, end, text) in rows {
        out.push_str(&format!(
            "{} --> {}\n{}\n\n",
            format_time_sep(start, '.'),
            format_time_sep(end, '.'),
            text.trim()
        ));
    }
    out
}

/// Parse SRT text. Cue numbers are optional; malformed blocks are errors so a
/// bad file never half-imports.
pub fn parse_srt(text: &str) -> Result<Vec<Cue>> {
    let text = text.trim_start_matches('\u{feff}').replace("\r\n", "\n");
    let mut cues = Vec::new();
    for block in text.split("\n\n").map(str::trim).filter(|b| !b.is_empty()) {
        // WebVTT header and comment blocks carry no cue.
        if block.starts_with("WEBVTT") || block.starts_with("NOTE") || block.starts_with("STYLE") {
            continue;
        }
        let mut lines = block.lines();
        let mut first = lines.next().unwrap_or("");
        // Optional index line.
        if first.trim().parse::<u64>().is_ok() {
            first = lines.next().unwrap_or("");
        }
        let (a, b) = first
            .split_once("-->")
            .ok_or_else(|| Error::InvalidArgument(format!("bad SRT cue: {first:?}")))?;
        let start =
            parse_time(a).ok_or_else(|| Error::InvalidArgument(format!("bad SRT time: {a:?}")))?;
        // Positioning hints after the end time are ignored.
        let b = b.split_whitespace().next().unwrap_or("");
        let end =
            parse_time(b).ok_or_else(|| Error::InvalidArgument(format!("bad SRT time: {b:?}")))?;
        if end <= start {
            return Err(Error::InvalidArgument(format!(
                "SRT cue ends before it starts: {first:?}"
            )));
        }
        let body: Vec<&str> = lines.map(str::trim_end).collect();
        cues.push((start, end, body.join("\n")));
    }
    cues.sort_by_key(|c| c.0);
    Ok(cues)
}

/// Format cues as SRT, numbered from 1 in time order.
pub fn format_srt<'a>(cues: impl IntoIterator<Item = (Rational, Rational, &'a str)>) -> String {
    let mut rows: Vec<(Rational, Rational, &str)> = cues.into_iter().collect();
    rows.sort_by_key(|c| c.0);
    let mut out = String::new();
    for (i, (start, end, text)) in rows.iter().enumerate() {
        out.push_str(&format!(
            "{}\n{} --> {}\n{}\n\n",
            i + 1,
            format_time(*start),
            format_time(*end),
            text.trim()
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn srt_round_trips_and_tolerates_loose_input() {
        let text = "\u{feff}1\r\n00:00:01,500 --> 00:00:03,000\r\nHello\r\nworld\r\n\r\n\n2\n00:01:00.25 --> 00:01:02 X1:0\nSecond\n";
        let cues = parse_srt(text).unwrap();
        assert_eq!(cues.len(), 2);
        assert_eq!(
            cues[0],
            (
                Rational::new(3, 2),
                Rational::from_int(3),
                "Hello\nworld".into()
            )
        );
        assert_eq!(cues[1].0, Rational::new(60_250, 1000));
        assert_eq!(cues[1].1, Rational::from_int(62));
        let out = format_srt(cues.iter().map(|c| (c.0, c.1, c.2.as_str())));
        assert_eq!(
            out,
            "1\n00:00:01,500 --> 00:00:03,000\nHello\nworld\n\n2\n00:01:00,250 --> 00:01:02,000\nSecond\n\n"
        );
        assert_eq!(parse_srt(&out).unwrap(), cues);
        assert!(parse_srt("00:00:02,000 --> 00:00:01,000\nbackwards").is_err());
        assert!(parse_srt("not a cue").is_err());
        // VTT round trip through the same parser.
        let vtt = format_vtt(cues.iter().map(|c| (c.0, c.1, c.2.as_str())));
        assert!(vtt.starts_with("WEBVTT\n\n00:00:01.500 --> 00:00:03.000\nHello\nworld\n\n"));
        assert_eq!(parse_srt(&vtt).unwrap(), cues);
    }
}
