use serde::Deserialize;

use crate::gen::{self, Event};

/// A composed score, as opposed to gen.rs's improvised mood melodies.
/// Time is in beats; each note is [beat, midinote, dur_beats] with an
/// optional 4th element as a per-note amp multiplier. Percussion voices
/// ignore pitch mostly (kick tunes off it), so 36/38/42 conventions work.
#[derive(Debug, Deserialize)]
pub struct Score {
    pub tempo: f64,
    #[serde(default)]
    pub fx: FxCfg,
    pub tracks: Vec<Track>,
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
    pub voice: String,
    #[serde(default = "default_amp")]
    pub amp: f64,
    #[serde(default)]
    pub pan: f64,
    #[serde(default = "default_bend")]
    pub bend: f64,
    /// wobble LFO rate in Hz
    #[serde(default = "default_rate")]
    pub rate: f64,
    pub notes: Vec<Vec<f64>>,
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

/// Parse a TOML score into scheduled events. Everything an LLM could get
/// wrong is either rejected with a message or clamped into a safe range.
pub fn parse(text: &str) -> Result<(Vec<Event>, FxCfg), String> {
    let score: Score = toml::from_str(text).map_err(|e| e.to_string())?;
    if !(20.0..=300.0).contains(&score.tempo) {
        return Err(format!("tempo {} outside 20..=300", score.tempo));
    }
    let spb = 60.0 / score.tempo;
    let mut events = Vec::new();
    for track in &score.tracks {
        let kind = gen::voice_kind(&track.voice);
        for note in &track.notes {
            if note.len() < 3 {
                return Err("note needs [beat, midinote, dur_beats]".into());
            }
            let (beat, pitch, dur) = (note[0], note[1], note[2]);
            let amp = (track.amp * note.get(3).copied().unwrap_or(1.0)).clamp(0.0, 0.35);
            if !(0.0..=64.0).contains(&beat) || !(12.0..=120.0).contains(&pitch) {
                return Err(format!("note out of range: {note:?}"));
            }
            events.push(Event {
                offset: beat * spb,
                kind,
                freq: gen::midicps(pitch),
                dur: (dur * spb).clamp(0.02, 4.0),
                amp,
                pan: track.pan.clamp(-1.0, 1.0),
                bend: track.bend.clamp(0.5, 2.0),
                rate: track.rate.clamp(0.25, 16.0),
            });
        }
    }
    if events.is_empty() {
        return Err("score has no notes".into());
    }
    if events.len() > 600 {
        return Err(format!("{} events; max 600", events.len()));
    }
    let fx = FxCfg {
        echo: score.fx.echo.clamp(0.03, 0.9),
        feedback: score.fx.feedback.clamp(0.0, 0.9),
        verb: score.fx.verb.clamp(0.0, 0.6),
    };
    Ok((events, fx))
}
