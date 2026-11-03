use std::collections::BTreeMap;

use rand::{Rng, SeedableRng};
use serde::{Deserialize, Serialize};

use crate::gen::{self, Event, VoiceKind, ARP_MAX};
use crate::notation::{self, NoteEv};

/// A composed score. Tracks declare voices; notes are mini-notation strings
/// (see docs/score.md), either directly on the track or in named sections
/// arranged by `structure = "A A B"`. Mood recipes are scores too, usually
/// with several note variants per track and a `vary` table.
#[derive(Debug, Clone, Deserialize)]
pub struct Score {
    pub tempo: f64,
    /// 0..=0.4: delay offbeat eighths by this fraction of a beat
    #[serde(default)]
    pub swing: f64,
    /// 0..=0.05 beats of random timing jitter
    #[serde(default)]
    pub humanize: f64,
    #[serde(default)]
    pub fx: FxCfg,
    #[serde(default)]
    pub vary: Vary,
    #[serde(default)]
    pub tracks: Vec<Track>,
    pub structure: Option<String>,
    #[serde(default)]
    pub sections: BTreeMap<String, BTreeMap<String, String>>,
}

/// Per-score effect settings; the daemon resets to defaults for mood chirps.
#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(default)]
pub struct FxCfg {
    /// echo delay in seconds
    pub echo: f64,
    /// echo feedback 0..0.9
    pub feedback: f64,
    /// reverb wet mix 0..0.6
    pub verb: f64,
}

impl Default for FxCfg {
    fn default() -> Self {
        FxCfg {
            echo: 0.19,
            feedback: 0.335,
            verb: 0.18,
        }
    }
}

/// How a performance may differ from the written motif. All optional; a
/// score file without `[vary]` plays exactly as written.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Vary {
    /// semitone offsets, one picked per play (repeat 0 to weight it)
    pub transpose: Vec<i32>,
    /// tempo jitter, 0..=0.2 as a fraction either way
    pub tempo: f64,
    /// 0..=1 chance of a chromatic grace note into one lead note
    pub grace: f64,
}

/// A track's notes: one string, or variants of which one plays per
/// performance (the same index across all tracks, wrapping if shorter).
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum Notes {
    One(String),
    Many(Vec<String>),
}

impl Notes {
    fn len(&self) -> usize {
        match self {
            Notes::One(_) => 1,
            Notes::Many(v) => v.len(),
        }
    }

    fn get(&self, i: usize) -> &str {
        match self {
            Notes::One(s) => s,
            Notes::Many(v) => &v[i % v.len()],
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct Track {
    /// referenced from sections; defaults to the voice name
    pub name: Option<String>,
    pub voice: String,
    #[serde(default = "default_amp")]
    pub amp: f64,
    #[serde(default)]
    pub pan: f64,
    /// pitch glide multiplier, applied only to notes marked `~`
    #[serde(default = "default_bend")]
    pub bend: f64,
    /// wobble LFO rate in Hz
    #[serde(default = "default_rate")]
    pub rate: f64,
    /// starting pulse width for pulse/arp
    pub duty: Option<f64>,
    /// mini-notation; only when the score has no `structure`
    pub notes: Option<Notes>,
}

fn default_amp() -> f64 {
    0.2
}
fn default_bend() -> f64 {
    1.0
}
fn default_rate() -> f64 {
    6.0
}

fn perc_pitch(voice: &str) -> i32 {
    match voice {
        "kick" => 36,
        "snare" => 38,
        "hat" => 42,
        "noise" => 72,
        "metal" => 84,
        _ => 60,
    }
}

/// What an agent iterates against: stats + advisory warnings.
#[derive(Debug, Serialize)]
pub struct Report {
    pub events: usize,
    pub seconds: f64,
    pub tempo: f64,
    pub variants: usize,
    pub tracks: Vec<TrackStat>,
    pub polyphony_peak: usize,
    pub level_peak: f64,
    pub warnings: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct TrackStat {
    pub name: String,
    pub voice: String,
    pub notes: usize,
    pub range: Option<[String; 2]>,
    pub notes_per_sec: f64,
}

/// Parse a score file into scheduled events (variant 0, as written), FX
/// settings, and a report. Every variant is compiled so a bad one fails here.
pub fn parse(text: &str) -> Result<(Vec<Event>, FxCfg, Report), String> {
    let score = load(text)?;
    let mut rng = rand::rngs::SmallRng::seed_from_u64(text.len() as u64);
    for v in 1..score.variants() {
        compile(&score, v, &mut rng).map_err(|e| format!("variant {}: {e}", v + 1))?;
    }
    compile(&score, 0, &mut rng)
}

/// TOML text to a Score, absorbing the structure-placement footgun.
pub fn load(text: &str) -> Result<Score, String> {
    let mut score: Score = toml::from_str(text).map_err(|e| e.to_string())?;
    // `structure = "A B"` written below any [table] header lands inside that
    // table, not at top level. Models naturally state the arrangement after
    // [fx] or the sections, so recover it from the text.
    if score.structure.is_none() {
        for line in text.lines() {
            if let Some(v) = line
                .trim()
                .strip_prefix("structure")
                .and_then(|r| r.trim_start().strip_prefix('='))
            {
                let v = v.trim().trim_matches(|c| c == '"' || c == '\'');
                if !v.is_empty() {
                    score.structure = Some(v.to_string());
                    break;
                }
            }
        }
    }
    // and drop stray copies swallowed into sections so they don't read as tracks
    for sec in score.sections.values_mut() {
        sec.remove("structure");
    }
    Ok(score)
}

impl Score {
    /// How many note variants this score has (the longest track's list).
    pub fn variants(&self) -> usize {
        self.tracks
            .iter()
            .filter_map(|t| t.notes.as_ref().map(Notes::len))
            .max()
            .unwrap_or(1)
            .max(1)
    }
}

/// Compile one performance: `variant` picks the note strings, `rng` drives
/// humanize and the `vary` table. Structural mistakes are errors; musical
/// smells become report warnings.
pub fn compile(
    score: &Score,
    variant: usize,
    rng: &mut impl Rng,
) -> Result<(Vec<Event>, FxCfg, Report), String> {
    if !(20.0..=480.0).contains(&score.tempo) {
        return Err(format!("tempo {} outside 20..=480", score.tempo));
    }
    let mut tracks = score.tracks.clone();
    // No [[tracks]] but sections keyed by voice names: infer default tracks.
    if tracks.is_empty() {
        let mut seen: Vec<String> = Vec::new();
        for sec in score.sections.values() {
            for k in sec.keys() {
                if !seen.contains(k) {
                    seen.push(k.clone());
                }
            }
        }
        if seen.is_empty() {
            return Err("score has no tracks".into());
        }
        for k in seen {
            if !gen::VOICE_NAMES.contains(&k.as_str()) {
                return Err(format!(
                    "no [[tracks]] defined and section key {k:?} is not a voice name"
                ));
            }
            tracks.push(Track {
                name: Some(k.clone()),
                voice: k,
                amp: default_amp(),
                pan: 0.0,
                bend: default_bend(),
                rate: default_rate(),
                duty: None,
                notes: None,
            });
        }
    }
    let kinds: Vec<VoiceKind> = tracks
        .iter()
        .map(|t| {
            gen::voice_kind(&t.voice).ok_or_else(|| {
                format!(
                    "unknown voice {:?} (want one of: {})",
                    t.voice,
                    gen::VOICE_NAMES.join(", ")
                )
            })
        })
        .collect::<Result<_, _>>()?;
    let mut warnings = Vec::new();
    let names: Vec<String> = tracks
        .iter()
        .map(|t| t.name.clone().unwrap_or_else(|| t.voice.clone()))
        .collect();

    // Per-track note lists, in absolute beats.
    let mut per_track: Vec<Vec<NoteEv>> = vec![Vec::new(); tracks.len()];
    match &score.structure {
        Some(structure) => {
            if tracks.iter().any(|t| t.notes.is_some()) {
                return Err("with `structure`, put notes in [sections.X], not on tracks".into());
            }
            let mut offset = 0.0;
            for sec_name in structure.split_whitespace() {
                let Some(section) = score.sections.get(sec_name) else {
                    return Err(format!("structure names undefined section {sec_name:?}"));
                };
                let mut sec_len: f64 = 0.0;
                for (track_name, src) in section {
                    let Some(ti) = names.iter().position(|n| n == track_name) else {
                        return Err(format!(
                            "section {sec_name:?} references unknown track {track_name:?}"
                        ));
                    };
                    let (notes, len) = notation::parse(src, perc_pitch(&tracks[ti].voice))
                        .map_err(|e| format!("section {sec_name}, track {track_name}: {e}"))?;
                    sec_len = sec_len.max(len);
                    per_track[ti].extend(notes.into_iter().map(|mut n| {
                        n.beat += offset;
                        n
                    }));
                }
                offset += sec_len;
            }
            for name in score.sections.keys() {
                if !structure.split_whitespace().any(|s| s == name) {
                    warnings.push(format!("section {name:?} defined but not in structure"));
                }
            }
        }
        None => {
            for (ti, track) in tracks.iter().enumerate() {
                let Some(src) = &track.notes else {
                    warnings.push(format!("track {:?} has no notes", names[ti]));
                    continue;
                };
                let (notes, _) = notation::parse(src.get(variant), perc_pitch(&track.voice))
                    .map_err(|e| format!("track {}: {e}", names[ti]))?;
                per_track[ti] = notes;
            }
        }
    }
    for (ti, notes) in per_track.iter().enumerate() {
        if notes.is_empty() && score.structure.is_some() {
            warnings.push(format!("track {:?} never used by any section", names[ti]));
        }
    }

    // The `vary` table: transpose, tempo jitter, a grace note into the lead.
    let vary = &score.vary;
    let transpose = if vary.transpose.is_empty() {
        0
    } else {
        vary.transpose[rng.gen_range(0..vary.transpose.len())]
    };
    let jitter = vary.tempo.clamp(0.0, 0.2);
    let tempo = if jitter > 0.0 {
        score.tempo * (1.0 + rng.gen_range(-jitter..=jitter))
    } else {
        score.tempo
    };
    if vary.grace > 0.0 && rng.gen_bool(vary.grace.clamp(0.0, 1.0)) {
        if let Some(lead) = per_track.iter_mut().zip(&tracks).find_map(|(n, t)| {
            (!gen::is_perc(&t.voice) && t.voice != "arp" && n.len() > 1).then_some(n)
        }) {
            // a note with room before it for a 1/8-beat approach from below
            let room: Vec<usize> = (1..lead.len())
                .filter(|&i| lead[i].beat - lead[i - 1].beat >= 0.25)
                .collect();
            if !room.is_empty() {
                let n = lead[room[rng.gen_range(0..room.len())]].clone();
                lead.push(NoteEv {
                    beat: n.beat - 0.125,
                    pitch: n.pitch - 1,
                    dur: 0.125,
                    amp_mul: n.amp_mul * 0.7,
                    bend: false,
                });
            }
        }
    }

    // Expand to events with swing + humanize.
    let spb = 60.0 / tempo;
    let swing = score.swing.clamp(0.0, 0.4);
    let humanize = score.humanize.clamp(0.0, 0.05);
    let mut events = Vec::new();
    let mut tagged: Vec<(usize, f64, f64, f64)> = Vec::new(); // (track, start_s, dur_s, amp)
    for (ti, notes) in per_track.iter().enumerate() {
        let track = &tracks[ti];
        let shift = if gen::is_perc(&track.voice) {
            0
        } else {
            transpose
        };
        for group in group_chords(notes, kinds[ti] == VoiceKind::Arp) {
            let n = &group[0];
            let pitch = n.pitch + shift;
            if !(0.0..=256.0).contains(&n.beat) || !(12..=120).contains(&pitch) {
                return Err(format!(
                    "note out of range in track {:?}: beat {} pitch {}",
                    names[ti], n.beat, pitch
                ));
            }
            let mut beat = n.beat;
            // offbeat eighth (x.5): push it late for swing feel
            if (beat.rem_euclid(1.0) - 0.5).abs() < 0.026 {
                beat += swing * 0.5;
            }
            if humanize > 0.0 {
                beat = (beat + rng.gen_range(-humanize..=humanize)).max(0.0);
            }
            let amp = (track.amp * n.amp_mul).clamp(0.0, 0.35);
            let dur = (n.dur * spb).clamp(0.02, 4.0);
            let mut arp = [0.0f32; ARP_MAX];
            for (slot, m) in arp.iter_mut().zip(&group) {
                *slot = (m.pitch - n.pitch) as f32;
            }
            let default_duty = if kinds[ti] == VoiceKind::Arp {
                0.25
            } else {
                0.125
            };
            events.push(Event {
                offset: beat * spb,
                kind: kinds[ti],
                freq: gen::midicps(pitch as f64),
                dur,
                amp,
                pan: track.pan.clamp(-1.0, 1.0),
                bend: if n.bend {
                    track.bend.clamp(0.5, 2.0)
                } else {
                    1.0
                },
                rate: track.rate.clamp(0.25, 16.0),
                duty: track.duty.unwrap_or(default_duty).clamp(0.05, 0.5),
                arp,
                arp_n: group.len() as u8,
            });
            tagged.push((ti, beat * spb, dur, amp));
        }
    }
    if events.is_empty() {
        return Err("score has no notes".into());
    }
    if events.len() > 1500 {
        return Err(format!("{} events; max 1500", events.len()));
    }

    let report = analyze(score, tempo, &tracks, &names, &per_track, &tagged, warnings);
    Ok((events, fx_clamped(score.fx), report))
}

/// Notes as they become voices. Most voices take each note alone; an arp
/// voice takes every note sharing a start beat as one chord, lowest first.
fn group_chords(notes: &[NoteEv], arp: bool) -> Vec<Vec<NoteEv>> {
    if !arp {
        return notes.iter().map(|n| vec![n.clone()]).collect();
    }
    let mut groups: Vec<Vec<NoteEv>> = Vec::new();
    for n in notes {
        match groups
            .iter_mut()
            .find(|g| (g[0].beat - n.beat).abs() < 1e-9)
        {
            Some(g) if g.len() < ARP_MAX => g.push(n.clone()),
            Some(_) => {}
            None => groups.push(vec![n.clone()]),
        }
    }
    for g in &mut groups {
        g.sort_by_key(|n| n.pitch);
    }
    groups
}

fn fx_clamped(fx: FxCfg) -> FxCfg {
    FxCfg {
        echo: fx.echo.clamp(0.03, 0.9),
        feedback: fx.feedback.clamp(0.0, 0.9),
        verb: fx.verb.clamp(0.0, 0.6),
    }
}

fn analyze(
    score: &Score,
    tempo: f64,
    tracks: &[Track],
    names: &[String],
    per_track: &[Vec<NoteEv>],
    tagged: &[(usize, f64, f64, f64)],
    mut warnings: Vec<String>,
) -> Report {
    let seconds = tagged
        .iter()
        .map(|(_, s, d, _)| s + d)
        .fold(0.0_f64, f64::max);

    let mut stats = Vec::new();
    for (ti, notes) in per_track.iter().enumerate() {
        if notes.is_empty() {
            continue;
        }
        let span: f64 = tagged
            .iter()
            .filter(|(t, ..)| *t == ti)
            .map(|(_, s, d, _)| s + d)
            .fold(0.0_f64, f64::max);
        let density = notes.len() as f64 / span.max(0.1);
        let (lo, hi) = notes.iter().fold((i32::MAX, i32::MIN), |(lo, hi), n| {
            (lo.min(n.pitch), hi.max(n.pitch))
        });
        if density > 20.0 {
            warnings.push(format!(
                "track {:?}: {:.0} notes/s is machine-gun territory",
                names[ti], density
            ));
        }
        if notes.len() > 12 && notes.iter().all(|n| n.amp_mul == notes[0].amp_mul) {
            warnings.push(format!(
                "track {:?}: {} notes, all identical velocity (consider ! and ?)",
                names[ti],
                notes.len()
            ));
        }
        stats.push(TrackStat {
            name: names[ti].clone(),
            voice: tracks[ti].voice.clone(),
            notes: notes.len(),
            range: (!gen::is_perc(&tracks[ti].voice))
                .then(|| [notation::name_of(lo), notation::name_of(hi)]),
            notes_per_sec: (density * 10.0).round() / 10.0,
        });
    }

    // Overlap sweep at each note start: polyphony and a rough summed-amp level.
    let (mut poly_peak, mut level_peak, mut peak_at) = (0usize, 0.0_f64, 0.0_f64);
    for (_, start, _, _) in tagged {
        let (mut poly, mut level) = (0, 0.0);
        for (_, s, d, a) in tagged {
            if *s <= *start && start < &(s + d) {
                poly += 1;
                level += a;
            }
        }
        poly_peak = poly_peak.max(poly);
        if level > level_peak {
            level_peak = level;
            peak_at = *start;
        }
    }
    if level_peak > 1.5 {
        warnings.push(format!(
            "summed level peaks at {level_peak:.2} around {peak_at:.1}s: may clip or squash"
        ));
    }
    if seconds > 20.0 {
        warnings.push(format!("{seconds:.1}s is long for a jingle"));
    }

    Report {
        events: tagged.len(),
        seconds: (seconds * 10.0).round() / 10.0,
        tempo: (tempo * 10.0).round() / 10.0,
        variants: score.variants(),
        tracks: stats,
        polyphony_peak: poly_peak,
        level_peak: (level_peak * 100.0).round() / 100.0,
        warnings,
    }
}

impl Report {
    pub fn render(&self) -> String {
        let variants = if self.variants > 1 {
            format!(" (variant 1 of {})", self.variants)
        } else {
            String::new()
        };
        let mut s = format!(
            "score ok: {} events, {:.1}s @ {:.0}bpm{variants}\n",
            self.events, self.seconds, self.tempo
        );
        for t in &self.tracks {
            let range = t
                .range
                .as_ref()
                .map(|[lo, hi]| format!(", {lo}\u{2013}{hi}"))
                .unwrap_or_default();
            s += &format!(
                "  {} ({}): {} notes{}, {:.1}/s\n",
                t.name, t.voice, t.notes, range, t.notes_per_sec
            );
        }
        s += &format!(
            "polyphony peak {}, est. level peak {:.2}\n",
            self.polyphony_peak, self.level_peak
        );
        if !self.warnings.is_empty() {
            s += "warnings:\n";
            for w in &self.warnings {
                s += &format!("  - {w}\n");
            }
        }
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rng() -> rand::rngs::SmallRng {
        rand::rngs::SmallRng::seed_from_u64(7)
    }

    #[test]
    fn arp_chords_become_one_voice() {
        let s = load("tempo = 120\n[[tracks]]\nvoice = \"arp\"\nnotes = \"g4+c4+e4:1 d4+f4:1 a4\"")
            .unwrap();
        let (ev, _, _) = compile(&s, 0, &mut rng()).unwrap();
        assert_eq!(ev.len(), 3);
        assert_eq!(ev[0].arp_n, 3);
        assert_eq!(&ev[0].arp[..3], &[0.0, 4.0, 7.0]); // sorted from c4
        assert!((ev[0].freq - gen::midicps(60.0)).abs() < 1e-9);
        assert_eq!(ev[2].arp_n, 1);
    }

    #[test]
    fn variants_wrap_and_transpose_skips_perc() {
        let s = load(
            "tempo = 120\nvary = { transpose = [5] }\ntracks = [\
             { voice = \"pulse\", notes = [\"c4\", \"d4\"] },\
             { voice = \"noise\", notes = \"x\" }]",
        )
        .unwrap();
        assert_eq!(s.variants(), 2);
        let (ev, _, _) = compile(&s, 1, &mut rng()).unwrap();
        assert!((ev[0].freq - gen::midicps(67.0)).abs() < 1e-9); // d4 + 5
        assert!((ev[1].freq - gen::midicps(72.0)).abs() < 1e-9); // noise home, untouched
    }

    #[test]
    fn grace_note_lands_before_a_lead_note() {
        let s = load(
            "tempo = 120\nvary = { grace = 1.0 }\n[[tracks]]\nvoice = \"pulse\"\nnotes = \"c5:1 e5:1\"",
        )
        .unwrap();
        let (ev, _, _) = compile(&s, 0, &mut rng()).unwrap();
        assert_eq!(ev.len(), 3);
        let g = &ev[2];
        assert!((g.offset - 0.4375).abs() < 1e-9); // 0.875 beats at 0.5 s/beat
        assert!((g.freq - gen::midicps(75.0)).abs() < 1e-9); // e5 (76) - 1
    }

    #[test]
    fn unknown_voice_is_an_error() {
        let s = load("tempo = 120\n[[tracks]]\nvoice = \"kazoo\"\nnotes = \"c4\"").unwrap();
        assert!(compile(&s, 0, &mut rng())
            .unwrap_err()
            .contains("unknown voice"));
    }

    #[test]
    fn parse_checks_every_variant() {
        let e = parse("tempo = 120\n[[tracks]]\nvoice = \"pulse\"\nnotes = [\"c4\", \"q9\"]")
            .unwrap_err();
        assert!(e.starts_with("variant 2"), "{e}");
    }
}
