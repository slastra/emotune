//! Mini-notation for score tracks: whitespace-separated tokens, e.g.
//! `a4 c5?:0.5 e5 f5:2 | (c5+e5 a4):1 x! r g#5~`
//!
//! - pitch names `c4`=60 (`#`/`b` accidentals), `x` = percussion home pitch
//! - `:dur` in beats, sticky until changed (initial 1); `|` barlines ignored
//! - `r` rest, `+` chord, `!` accent (amp x1.3), `?` ghost (x0.6),
//!   `~` apply the track's bend to this note, `(...)xN` repeat group

/// One parsed note, in beats relative to the string's start.
#[derive(Debug, Clone, PartialEq)]
pub struct NoteEv {
    pub beat: f64,
    pub pitch: i32,
    pub dur: f64,
    pub amp_mul: f64,
    pub bend: bool,
}

struct Parser<'a> {
    toks: Vec<&'a str>,
    i: usize,
    dur: f64,
    perc_pitch: i32,
}

/// Parse a track's notation. Returns the notes and the total length in beats.
pub fn parse(src: &str, perc_pitch: i32) -> Result<(Vec<NoteEv>, f64), String> {
    // pad group delimiters so "(c4" and "e4)x2" tokenize cleanly
    let spaced = src.replace('(', " ( ").replace(')', " )");
    let toks: Vec<&str> = spaced.split_whitespace().filter(|t| *t != "|").collect();
    let mut p = Parser {
        toks,
        i: 0,
        dur: 1.0,
        perc_pitch,
    };
    let mut out = Vec::new();
    let end = p.seq(0, 0.0, &mut out)?;
    if p.i < p.toks.len() {
        return Err(format!("unmatched ')' at token {}", p.i + 1));
    }
    Ok((out, end))
}

impl<'a> Parser<'a> {
    /// Parse tokens until end of input or a ')' closer; returns the end beat.
    fn seq(&mut self, depth: usize, start: f64, out: &mut Vec<NoteEv>) -> Result<f64, String> {
        if depth > 4 {
            return Err("repeat groups nested deeper than 4".into());
        }
        let mut cursor = start;
        while self.i < self.toks.len() {
            let tok = self.toks[self.i];
            if let Some(rest) = tok.strip_prefix(')') {
                if depth == 0 {
                    return Err(format!("')' without '(' at token {}", self.i + 1));
                }
                self.i += 1;
                // `):d` time-squashes the group to d beats (models write
                // `(c4+e4+g4):0.5` expecting parens + duration to combine)
                if let Some(d) = rest.strip_prefix(':') {
                    let d: f64 = d.parse().map_err(|_| format!("bad duration {rest:?}"))?;
                    if !(0.05..=16.0).contains(&d) {
                        return Err(format!("duration {d} outside 0.05..=16"));
                    }
                    let span = cursor - start;
                    if span <= 0.0 {
                        return Err("empty '():dur' group".into());
                    }
                    let k = d / span;
                    for n in out.iter_mut().filter(|n| n.beat >= start - 1e-9) {
                        n.beat = start + (n.beat - start) * k;
                        n.dur *= k;
                    }
                    self.dur = d;
                    return Ok(start + d);
                }
                let count: usize = match rest.strip_prefix('x') {
                    Some(n) => n
                        .parse()
                        .map_err(|_| format!("bad repeat count {rest:?}"))?,
                    None => 1,
                };
                if !(1..=32).contains(&count) {
                    return Err(format!("repeat count {count} outside 1..=32"));
                }
                return Ok(repeat_tail(out, start, cursor, count));
            }
            self.i += 1;
            if tok == "(" {
                let mark = out.len();
                let gstart = cursor;
                let tok_at = self.i;
                let gend = self.seq(depth + 1, gstart, out)?;
                if gend == gstart && out.len() == mark {
                    return Err(format!("empty group at token {tok_at}"));
                }
                cursor = gend;
                continue;
            }
            cursor = self.token(tok, cursor, out)?;
        }
        if depth > 0 {
            return Err("unclosed '('".into());
        }
        Ok(cursor)
    }

    /// Parse one note/rest token starting at `beat`; returns the new cursor.
    fn token(&mut self, tok: &str, beat: f64, out: &mut Vec<NoteEv>) -> Result<f64, String> {
        // modifiers may sit before or after :dur (`x?:0.5` and `x:0.5?` both occur)
        let (mut amp_mul, mut bend) = (1.0, false);
        let mut body = strip_mods(tok, &mut amp_mul, &mut bend);
        // optional :dur (sticky)
        if let Some((left, d)) = body.split_once(':') {
            let d: f64 = d.parse().map_err(|_| format!("bad duration in {tok:?}"))?;
            if !(0.05..=16.0).contains(&d) {
                return Err(format!("duration {d} outside 0.05..=16 in {tok:?}"));
            }
            self.dur = d;
            body = left;
        }
        let dur = self.dur;
        body = strip_mods(body, &mut amp_mul, &mut bend);
        if body == "r" {
            return Ok(beat + dur);
        }
        for part in body.split('+') {
            let (mut amp_mul, mut bend) = (amp_mul, bend);
            let part = strip_mods(part, &mut amp_mul, &mut bend);
            let pitch = if part == "x" {
                self.perc_pitch
            } else {
                pitch_of(part).ok_or_else(|| {
                    format!("bad pitch {part:?} in {tok:?} (want e.g. c4, f#3, bb2, x, r)")
                })?
            };
            out.push(NoteEv {
                beat,
                pitch,
                dur,
                amp_mul,
                bend,
            });
        }
        Ok(beat + dur)
    }
}

/// Peel `!` / `?` / `~` off the end of a token piece, recording their effect.
fn strip_mods<'a>(mut body: &'a str, amp_mul: &mut f64, bend: &mut bool) -> &'a str {
    while let Some(b) = body.strip_suffix(['!', '?', '~']) {
        match body.as_bytes()[body.len() - 1] {
            b'!' => *amp_mul = 1.3,
            b'?' => *amp_mul = 0.6,
            _ => *bend = true,
        }
        body = b;
    }
    body
}

/// Duplicate the group's events `count-1` more times; returns the final end beat.
fn repeat_tail(out: &mut Vec<NoteEv>, gstart: f64, gend: f64, count: usize) -> f64 {
    let span = gend - gstart;
    let group: Vec<NoteEv> = out
        .iter()
        .filter(|n| n.beat >= gstart - 1e-9)
        .cloned()
        .collect();
    for k in 1..count {
        out.extend(group.iter().map(|n| NoteEv {
            beat: n.beat + span * k as f64,
            ..n.clone()
        }));
    }
    gstart + span * count as f64
}

/// `c4` = 60. Letter, optional single `#`/`b`, octave 0..=9.
fn pitch_of(s: &str) -> Option<i32> {
    let bytes = s.as_bytes();
    let sem = match bytes.first()?.to_ascii_lowercase() {
        b'c' => 0,
        b'd' => 2,
        b'e' => 4,
        b'f' => 5,
        b'g' => 7,
        b'a' => 9,
        b'b' => 11,
        _ => return None,
    };
    let (acc, rest) = match bytes.get(1)? {
        b'#' => (1, &s[2..]),
        b'b' => (-1, &s[2..]),
        _ => (0, &s[1..]),
    };
    let oct: i32 = rest.parse().ok()?;
    if !(0..=9).contains(&oct) {
        return None;
    }
    Some(12 * (oct + 1) + sem + acc)
}

/// Midi note back to a display name (flats never emitted).
pub fn name_of(midi: i32) -> String {
    const NAMES: [&str; 12] = [
        "c", "c#", "d", "d#", "e", "f", "f#", "g", "g#", "a", "a#", "b",
    ];
    format!(
        "{}{}",
        NAMES[midi.rem_euclid(12) as usize],
        midi.div_euclid(12) - 1
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pitches() {
        assert_eq!(pitch_of("c4"), Some(60));
        assert_eq!(pitch_of("a4"), Some(69));
        assert_eq!(pitch_of("f#3"), Some(54));
        assert_eq!(pitch_of("bb2"), Some(46));
        assert_eq!(pitch_of("h4"), None);
        assert_eq!(name_of(69), "a4");
    }

    #[test]
    fn sticky_dur_and_rest() {
        let (n, end) = parse("c4:0.5 d4 r e4:1", 36).unwrap();
        assert_eq!(n.len(), 3);
        assert_eq!(n[1].beat, 0.5);
        assert_eq!(n[2].beat, 1.5); // rest consumed 0.5
        assert_eq!(n[2].dur, 1.0);
        assert_eq!(end, 2.5);
    }

    #[test]
    fn chords_and_mods() {
        let (n, _) = parse("c4+e4+g4:2! a4?~", 36).unwrap();
        assert_eq!(n.len(), 4);
        assert!(n[..3]
            .iter()
            .all(|x| x.beat == 0.0 && x.dur == 2.0 && x.amp_mul == 1.3));
        assert_eq!(n[3].amp_mul, 0.6);
        assert!(n[3].bend);
    }

    #[test]
    fn repeats_nested() {
        let (n, end) = parse("((x:0.5 r)x2 x:1)x2", 42).unwrap();
        assert_eq!(end, 6.0);
        assert_eq!(n.len(), 6);
        let beats: Vec<f64> = n.iter().map(|x| x.beat).collect();
        assert_eq!(beats, vec![0.0, 1.0, 2.0, 3.0, 4.0, 5.0]);
    }

    #[test]
    fn percussion_and_barlines() {
        let (n, _) = parse("x:0.5 | x! r x?", 38).unwrap();
        assert_eq!(n.len(), 3);
        assert!(n.iter().all(|x| x.pitch == 38));
    }

    #[test]
    fn group_duration() {
        // parens + :dur = squash the group into that many beats
        let (n, end) = parse("(c4+e4+g4):0.5 c4", 36).unwrap();
        assert_eq!(end, 1.0);
        assert!(n[..3].iter().all(|x| x.beat == 0.0 && x.dur == 0.5));
        assert_eq!(n[3].beat, 0.5); // sticky dur updated by the group
        let (n2, end2) = parse("(c4 d4):1", 36).unwrap();
        assert_eq!(end2, 1.0);
        assert_eq!(n2[1].beat, 0.5);
    }

    #[test]
    fn mods_before_dur() {
        let (n, _) = parse("x?:0.5 c4!:2 e4~+g4", 38).unwrap();
        assert_eq!(n[0].amp_mul, 0.6);
        assert_eq!(n[0].dur, 0.5);
        assert_eq!(n[1].amp_mul, 1.3);
        assert!(n[2].bend && !n[3].bend);
    }

    #[test]
    fn errors() {
        assert!(parse("(c4", 36).is_err());
        assert!(parse("c4)x2", 36).is_err());
        assert!(parse("q9", 36).unwrap_err().contains("bad pitch"));
        assert!(parse("c4:99", 36).is_err());
    }
}
