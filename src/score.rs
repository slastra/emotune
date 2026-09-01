use std::collections::BTreeMap;

use rand::{Rng, SeedableRng};
use serde::{Deserialize, Serialize};

use crate::gen::{self, Event};
use crate::notation::{self, NoteEv};

/// A composed score. Tracks declare voices; notes are mini-notation strings
/// (see docs/score.md), either directly on the track or in named sections
/// arranged by `structure = "A A B"`.
#[derive(Debug, Deserialize)]
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
        FxCfg { echo: 0.19, feedback: 0.335, verb: 0.18 }
    }
}

#[derive(Debug, Deserialize)]
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
    /// mini-notation; only when the score has no `structure`
    pub notes: Option<String>,
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
        _ => 60,
    }
}

/// What an agent iterates against: stats + advisory warnings.
#[derive(Debug, Serialize)]
pub struct Report {
    pub events: usize,
    pub seconds: f64,
    pub tempo: f64,
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

/// Parse a v2 score into scheduled events, FX settings, and a report.
/// Structural mistakes are errors; musical smells become report warnings.
pub fn parse(text: &str) -> Result<(Vec<Event>, FxCfg, Report), String> {
    let score: Score = toml::from_str(text).map_err(|e| e.to_string())?;
    if !(20.0..=300.0).contains(&score.tempo) {
        return Err(format!("tempo {} outside 20..=300", score.tempo));
    }
    if score.tracks.is_empty() {
        return Err("score has no tracks".into());
    }
    let mut warnings = Vec::new();
    let names: Vec<String> = score
        .tracks
        .iter()
        .map(|t| t.name.clone().unwrap_or_else(|| t.voice.clone()))
        .collect();

    // Per-track note lists, in absolute beats.
    let mut per_track: Vec<Vec<NoteEv>> = vec![Vec::new(); score.tracks.len()];
    match &score.structure {
        Some(structure) => {
            if score.tracks.iter().any(|t| t.notes.is_some()) {
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
                    let (notes, len) = notation::parse(src, perc_pitch(&score.tracks[ti].voice))
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
            for (ti, track) in score.tracks.iter().enumerate() {
                let Some(src) = &track.notes else {
                    warnings.push(format!("track {:?} has no notes", names[ti]));
                    continue;
                };
                let (notes, _) = notation::parse(src, perc_pitch(&track.voice))
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

    // Expand to events with swing + humanize.
    let spb = 60.0 / score.tempo;
    let swing = score.swing.clamp(0.0, 0.4);
    let humanize = score.humanize.clamp(0.0, 0.05);
    let mut rng = rand::rngs::SmallRng::seed_from_u64(text.len() as u64);
    let mut events = Vec::new();
    let mut tagged: Vec<(usize, f64, f64, f64)> = Vec::new(); // (track, start_s, dur_s, amp)
    for (ti, notes) in per_track.iter().enumerate() {
        let track = &score.tracks[ti];
        for n in notes {
            if !(0.0..=256.0).contains(&n.beat) || !(12..=120).contains(&n.pitch) {
                return Err(format!(
                    "note out of range in track {:?}: beat {} pitch {}",
                    names[ti], n.beat, n.pitch
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
            events.push(Event {
                offset: beat * spb,
                kind: gen::voice_kind(&track.voice),
                freq: gen::midicps(n.pitch as f64),
                dur,
                amp,
                pan: track.pan.clamp(-1.0, 1.0),
                bend: if n.bend { track.bend.clamp(0.5, 2.0) } else { 1.0 },
                rate: track.rate.clamp(0.25, 16.0),
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

    let report = analyze(&score, &names, &per_track, &tagged, warnings);
    Ok((events, fx_clamped(score.fx), report))
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
    names: &[String],
    per_track: &[Vec<NoteEv>],
    tagged: &[(usize, f64, f64, f64)],
    mut warnings: Vec<String>,
) -> Report {
    let seconds = tagged
        .iter()
        .map(|(_, s, d, _)| s + d)
        .fold(0.0_f64, f64::max);

    let mut tracks = Vec::new();
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
        let is_perc = matches!(score.tracks[ti].voice.as_str(), "kick" | "snare" | "hat");
        let (lo, hi) = notes
            .iter()
            .fold((i32::MAX, i32::MIN), |(lo, hi), n| (lo.min(n.pitch), hi.max(n.pitch)));
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
        tracks.push(TrackStat {
            name: names[ti].clone(),
            voice: score.tracks[ti].voice.clone(),
            notes: notes.len(),
            range: (!is_perc).then(|| [notation::name_of(lo), notation::name_of(hi)]),
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
        tempo: score.tempo,
        tracks,
        polyphony_peak: poly_peak,
        level_peak: (level_peak * 100.0).round() / 100.0,
        warnings,
    }
}

impl Report {
    pub fn render(&self) -> String {
        let mut s = format!(
            "score ok: {} events, {:.1}s @ {:.0}bpm\n",
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
