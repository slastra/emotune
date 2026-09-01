use rand::Rng;

use crate::recipes::Recipe;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum VoiceKind {
    Tone,
    Womp,
    Soft,
    Kick,
    Snare,
    Hat,
    Bell,
    Wobble,
}

/// One scheduled note: when it starts (seconds from now) and how it sounds.
#[derive(Debug, Clone)]
pub struct Event {
    pub offset: f64,
    pub kind: VoiceKind,
    pub freq: f64,
    pub dur: f64,
    pub amp: f64,
    pub pan: f64,
    pub bend: f64,
    /// modulation rate in Hz (wobble LFO); unused by other voices
    pub rate: f64,
}

pub fn midicps(m: f64) -> f64 {
    440.0 * 2f64.powf((m - 69.0) / 12.0)
}

/// SC's performDegreeToKey: scale degree → semitones, wrapping into octaves.
fn degree_to_key(scale: &[i32], degree: i32) -> i32 {
    let len = scale.len() as i32;
    let octave = degree.div_euclid(len);
    scale[degree.rem_euclid(len) as usize] + 12 * octave
}

pub fn voice_kind(name: &str) -> VoiceKind {
    match name {
        "womp" => VoiceKind::Womp,
        "soft" => VoiceKind::Soft,
        "kick" => VoiceKind::Kick,
        "snare" => VoiceKind::Snare,
        "hat" => VoiceKind::Hat,
        "bell" => VoiceKind::Bell,
        "wobble" => VoiceKind::Wobble,
        _ => VoiceKind::Tone,
    }
}

/// Improvise a melody from a recipe: random walk over scale degrees,
/// drifting in `dir` with occasional back-steps; 40ms timing grid;
/// last note rings long, carries the bend, or explodes into an arp finale.
pub fn melody(r: &Recipe, rng: &mut impl Rng) -> Vec<Event> {
    let kind = voice_kind(&r.synth);
    let mut deg: i32 = 0;
    let n = rng.gen_range(r.notes[0]..=r.notes[1]);
    let steps: [i32; 4] = if r.dir == 0 { [-2, -1, 1, 2] } else { [r.dir, r.dir, 2 * r.dir, -r.dir] };
    let mut semis: Vec<i32> = (0..n)
        .map(|_| {
            let s = degree_to_key(&r.scale, deg);
            deg += steps[rng.gen_range(0..4)];
            s
        })
        .collect();
    if let Some(end) = r.end {
        semis.push(degree_to_key(&r.scale, end));
    }

    let mut events = Vec::new();
    let mut t = 0.0;
    let count = semis.len();
    for (i, semi) in semis.into_iter().enumerate() {
        let last = i == count - 1;
        let wait = ((rng.gen_range(r.dur[0]..=r.dur[1]) / 0.04).round() * 0.04).max(0.04);
        if last && r.arp {
            // finale: rapid major-triad arpeggio on the resolution note
            for k in 0..12 {
                events.push(Event {
                    offset: t + k as f64 * 0.045,
                    kind: VoiceKind::Tone,
                    freq: midicps((r.base + semi + [0, 4, 7][k % 3]) as f64),
                    dur: 0.1,
                    amp: r.amp,
                    pan: (k % 3) as f64 * 0.3 - 0.3,
                    bend: 1.0,
                    rate: 6.0,
                });
            }
        } else {
            events.push(Event {
                offset: t,
                kind,
                freq: midicps((r.base + semi) as f64),
                dur: wait * if last { 5.0 } else { 2.0 },
                amp: r.amp,
                pan: rng.gen_range(-0.2..0.2),
                bend: if last { r.bend.unwrap_or(1.0) } else { 1.0 },
                rate: 6.0,
            });
            t += wait;
        }
    }
    events
}
