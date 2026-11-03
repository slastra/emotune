/// The synth's voices. `tone` and `soft` are kept as score aliases for
/// `pulse` and `tri` so older scores pick up the newer chip voices.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum VoiceKind {
    /// bandlimited pulse: PWM drift, detuned second osc, delayed vibrato
    Pulse,
    /// tracker arpeggio: one pulse voice cycling through a chord
    Arp,
    /// 4-bit stepped triangle
    Tri,
    /// 15-bit LFSR noise, long mode (hiss)
    Noise,
    /// 15-bit LFSR noise, short mode (metallic, tonal)
    Metal,
    Womp,
    Kick,
    Snare,
    Hat,
    Bell,
    Wobble,
}

/// Every voice name a score or recipe may use.
pub const VOICE_NAMES: [&str; 13] = [
    "pulse", "tone", "arp", "tri", "soft", "noise", "metal", "womp", "bell", "wobble", "kick",
    "snare", "hat",
];

/// Most notes an arp chord can cycle through.
pub const ARP_MAX: usize = 4;

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
    /// starting pulse width for pulse/arp, 0.05..=0.5
    pub duty: f64,
    /// arp chord as semitones above `freq`; first `arp_n` entries are used
    pub arp: [f32; ARP_MAX],
    pub arp_n: u8,
}

pub fn midicps(m: f64) -> f64 {
    440.0 * 2f64.powf((m - 69.0) / 12.0)
}

pub fn voice_kind(name: &str) -> Option<VoiceKind> {
    Some(match name {
        "pulse" | "tone" => VoiceKind::Pulse,
        "arp" => VoiceKind::Arp,
        "tri" | "soft" => VoiceKind::Tri,
        "noise" => VoiceKind::Noise,
        "metal" => VoiceKind::Metal,
        "womp" => VoiceKind::Womp,
        "kick" => VoiceKind::Kick,
        "snare" => VoiceKind::Snare,
        "hat" => VoiceKind::Hat,
        "bell" => VoiceKind::Bell,
        "wobble" => VoiceKind::Wobble,
        _ => return None,
    })
}

/// Unpitched voices: transposition skips them and the report shows no range.
pub fn is_perc(name: &str) -> bool {
    matches!(name, "kick" | "snare" | "hat" | "noise" | "metal")
}
