use std::f64::consts::TAU;
use std::sync::mpsc::Receiver;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

use crate::gen::{Event, VoiceKind};

use crate::score::FxCfg;

const ECHO_GAIN: f32 = 0.35;
const ECHO_MAX_SECS: f64 = 1.0;

struct Voice {
    kind: VoiceKind,
    freq: f64,
    dur: f64,
    amp: f64,
    pan: f64,
    bend: f64,
    rate: f64,
    t: f64,
    ph1: f64,
    ph2: f64,
    lp: f64,
    noise: u32,
}

impl Voice {
    fn new(e: &Event) -> Self {
        Voice {
            kind: e.kind,
            freq: e.freq,
            dur: e.dur,
            amp: e.amp,
            pan: e.pan,
            bend: e.bend,
            rate: e.rate,
            t: 0.0,
            ph1: 0.0,
            ph2: 0.0,
            lp: 0.0,
            noise: (e.freq.to_bits() as u32) | 1,
        }
    }

    /// xorshift32 white noise in [-1, 1)
    fn white(&mut self) -> f64 {
        let mut x = self.noise;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.noise = x;
        x as f64 / (u32::MAX as f64 / 2.0) - 1.0
    }

    fn total(&self) -> f64 {
        match self.kind {
            VoiceKind::Tone => 0.002 + self.dur,
            VoiceKind::Womp => 0.005 + self.dur,
            VoiceKind::Soft | VoiceKind::Bell => 0.005 + self.dur * 2.0,
            VoiceKind::Kick | VoiceKind::Snare | VoiceKind::Hat | VoiceKind::Wobble => {
                0.005 + self.dur
            }
        }
    }

    fn done(&self) -> bool {
        self.t >= self.total()
    }

    fn sample(&mut self, dt: f64) -> f64 {
        let t = self.t;
        self.t += dt;
        let s = match self.kind {
            VoiceKind::Tone => {
                // 25%-duty pulse, 6Hz vibrato, exponential bend glide on the last note
                let glide = self.bend.powf((t / self.dur).min(1.0));
                let vib = (TAU * 6.0 * t).sin() * self.freq * 0.006;
                let f = self.freq * glide + vib;
                self.ph1 = (self.ph1 + f * dt).fract();
                let sig = if self.ph1 < 0.25 { 1.0 } else { -1.0 };
                // env: 2ms attack -> 1, sag to 0.6 over dur*0.4, curved out over dur*0.6
                let env = if t < 0.002 {
                    t / 0.002
                } else if t < 0.002 + self.dur * 0.4 {
                    1.0 - 0.4 * ((t - 0.002) / (self.dur * 0.4))
                } else {
                    let x = ((t - 0.002 - self.dur * 0.4) / (self.dur * 0.6)).min(1.0);
                    0.6 * (1.0 - x).powi(3)
                };
                sig * env * 0.7
            }
            VoiceKind::Womp => {
                // two 50% pulses an octave apart, pitch falling, lowpassed
                let fall = 0.85f64.powf((t / self.dur).min(1.0));
                let f = self.freq * fall;
                self.ph1 = (self.ph1 + f * dt).fract();
                self.ph2 = (self.ph2 + f * 0.5 * dt).fract();
                let p = |ph: f64| if ph < 0.5 { 1.0 } else { -1.0 };
                let sig = p(self.ph1) + 0.6 * p(self.ph2);
                let a = 1.0 - (-TAU * 2000.0 * dt).exp();
                self.lp += a * (sig - self.lp);
                let env = if t < 0.005 {
                    t / 0.005
                } else {
                    (1.0 - ((t - 0.005) / self.dur).min(1.0)).powi(3)
                };
                self.lp * env * 0.7
            }
            VoiceKind::Soft => {
                self.ph1 = (self.ph1 + self.freq * dt).fract();
                let sig = 4.0 * (self.ph1 - 0.5).abs() - 1.0;
                let env = if t < 0.005 {
                    t / 0.005
                } else {
                    (1.0 - ((t - 0.005) / (self.dur * 2.0)).min(1.0)).powi(3)
                };
                sig * env
            }
            VoiceKind::Kick => {
                // pitch-swept sine: 4x freq collapsing to freq, plus a 3ms noise click
                let f = self.freq * (1.0 + 3.0 * (-t * 35.0).exp());
                self.ph1 = (self.ph1 + f * dt).fract();
                let body = (TAU * self.ph1).sin();
                let click = if t < 0.003 { self.white() * 0.4 } else { 0.0 };
                let env = if t < 0.003 {
                    t / 0.003
                } else {
                    (1.0 - ((t - 0.003) / self.dur).min(1.0)).powi(4)
                };
                (body + click) * env
            }
            VoiceKind::Snare => {
                // 190Hz triangle body under a bright noise burst
                self.ph1 = (self.ph1 + 190.0 * dt).fract();
                let body = (4.0 * (self.ph1 - 0.5).abs() - 1.0) * 0.4;
                let n = self.white();
                let a = 1.0 - (-TAU * 4500.0 * dt).exp();
                self.lp += a * (n - self.lp);
                let env = if t < 0.002 {
                    t / 0.002
                } else {
                    (1.0 - ((t - 0.002) / self.dur).min(1.0)).powi(4)
                };
                (body + self.lp * 0.9) * env
            }
            VoiceKind::Hat => {
                // highpassed noise tick: white minus its lowpassed self
                let n = self.white();
                let a = 1.0 - (-TAU * 6000.0 * dt).exp();
                self.lp += a * (n - self.lp);
                let hp = n - self.lp;
                let env = if t < 0.002 {
                    t / 0.002
                } else {
                    (1.0 - ((t - 0.002) / self.dur).min(1.0)).powi(6)
                };
                hp * env * 0.8
            }
            VoiceKind::Bell => {
                // 2-op FM chime: modulator at 2.01x for slow inharmonic shimmer,
                // modulation index decaying faster than the amp envelope
                self.ph1 = (self.ph1 + self.freq * dt).fract();
                self.ph2 = (self.ph2 + self.freq * 2.01 * dt).fract();
                let idx = 2.5 * (-t * 5.0).exp();
                let sig = (TAU * self.ph1 + idx * (TAU * self.ph2).sin()).sin();
                let env = if t < 0.002 {
                    t / 0.002
                } else {
                    (1.0 - ((t - 0.002) / (self.dur * 2.0)).min(1.0)).powi(2)
                };
                sig * env * 0.8
            }
            VoiceKind::Wobble => {
                // dubby LFO-swept lowpass over a 50% pulse + sub sine
                self.ph1 = (self.ph1 + self.freq * dt).fract();
                self.ph2 = (self.ph2 + self.freq * 0.5 * dt).fract();
                let pulse = if self.ph1 < 0.5 { 1.0 } else { -1.0 };
                let sub = (TAU * self.ph2).sin() * 0.5;
                let lfo = 0.5 * (1.0 + (TAU * self.rate * t - TAU / 4.0).sin());
                let cutoff = 150.0 + 2800.0 * lfo * lfo;
                let a = 1.0 - (-TAU * cutoff * dt).exp();
                self.lp += a * (pulse + sub - self.lp);
                let env = if t < 0.005 {
                    t / 0.005
                } else {
                    (1.0 - ((t - 0.005) / self.dur).min(1.0)).powi(2)
                };
                self.lp * env * 0.7
            }
        };
        s * self.amp
    }
}

/// Ping-pong echo + small Schroeder reverb, matching \efx's topology.
struct Fx {
    comb: Vec<f32>,
    ci: usize,
    right: Vec<f32>,
    ri: usize,
    len: usize,
    feedback: f32,
    verb_mix: f32,
    sr: f64,
    verb_combs: Vec<(Vec<f32>, usize, f32)>,
    verb_ap: Vec<(Vec<f32>, usize)>,
}

impl Fx {
    fn new(sr: f64) -> Self {
        let echo_len = (ECHO_MAX_SECS * sr) as usize;
        let scale = sr / 44100.0;
        let combs = [1116, 1188, 1277, 1356]
            .iter()
            .map(|&l| (vec![0.0; (l as f64 * scale) as usize], 0, 0.0))
            .collect();
        let aps = [556, 441]
            .iter()
            .map(|&l| (vec![0.0; (l as f64 * scale) as usize], 0))
            .collect();
        let mut fx = Fx {
            comb: vec![0.0; echo_len],
            ci: 0,
            right: vec![0.0; echo_len],
            ri: 0,
            len: 1,
            feedback: 0.0,
            verb_mix: 0.0,
            sr,
            verb_combs: combs,
            verb_ap: aps,
        };
        fx.set(FxCfg::default());
        fx
    }

    /// Apply a score's FX settings. Changing the echo delay clears the
    /// delay lines so stale samples from the old loop length don't replay.
    fn set(&mut self, cfg: FxCfg) {
        let len = ((cfg.echo * self.sr) as usize).clamp(1, self.comb.len());
        if len != self.len {
            self.comb.fill(0.0);
            self.right.fill(0.0);
            self.ci = 0;
            self.ri = 0;
            self.len = len;
        }
        self.feedback = cfg.feedback as f32;
        self.verb_mix = cfg.verb as f32;
    }

    fn process(&mut self, l: f32, r: f32) -> (f32, f32) {
        // ping-pong echo: mono sum into a feedback comb; right channel lags by the same delay
        let mono = (l + r) * 0.5;
        let tap = self.comb[self.ci];
        self.comb[self.ci] = mono + tap * self.feedback;
        self.ci = (self.ci + 1) % self.len;
        let echo_l = tap * ECHO_GAIN;
        let echo_r = self.right[self.ri];
        self.right[self.ri] = echo_l;
        self.ri = (self.ri + 1) % self.len;

        let dl = l + echo_l;
        let dr = r + echo_r;

        // small room: parallel damped combs + series allpasses on the mono sum
        let vin = (dl + dr) * 0.5;
        let mut wet = 0.0;
        for (buf, idx, damp) in &mut self.verb_combs {
            let out = buf[*idx];
            *damp = out * 0.25 + *damp * 0.75;
            buf[*idx] = vin + *damp * 0.72;
            *idx = (*idx + 1) % buf.len();
            wet += out;
        }
        wet *= 0.25;
        for (buf, idx) in &mut self.verb_ap {
            let d = buf[*idx];
            buf[*idx] = wet + d * 0.5;
            *idx = (*idx + 1) % buf.len();
            wet = d - 0.5 * (wet + d * 0.5);
        }
        (
            dl * (1.0 - self.verb_mix) + wet * self.verb_mix,
            dr * (1.0 - self.verb_mix) + wet * self.verb_mix,
        )
    }
}

/// Start the audio stream. Returns the stream (keep it alive) — event batches
/// arriving on `rx` (a mood melody or a composed score) are scheduled and
/// mixed inside the audio callback.
pub fn start(rx: Receiver<(Vec<Event>, FxCfg)>) -> cpal::Stream {
    let host = cpal::default_host();
    // Prefer the PipeWire ALSA PCM: the bare "default" PCM can probe as
    // unavailable even when pipewire-alsa is working.
    let (device, config) = host
        .output_devices()
        .into_iter()
        .flatten()
        .filter(|d| {
            d.name()
                .map(|n| n == "pipewire" || n == "default" || n == "pulse")
                .unwrap_or(false)
        })
        .chain(host.default_output_device())
        .find_map(|d| d.default_output_config().ok().map(|c| (d, c)))
        .expect("no usable audio output device");
    let sr = config.sample_rate().0 as f64;
    let channels = config.channels() as usize;

    let mut fx = Fx::new(sr);
    let mut voices: Vec<Voice> = Vec::new();
    let mut pending: Vec<(u64, Event)> = Vec::new();
    let mut now: u64 = 0;
    let dt = 1.0 / sr;

    let stream = device
        .build_output_stream(
            &config.into(),
            move |data: &mut [f32], _| {
                for (batch, cfg) in rx.try_iter() {
                    fx.set(cfg);
                    for e in batch {
                        pending.push((now + (e.offset * sr) as u64, e));
                    }
                }
                for frame in data.chunks_mut(channels) {
                    let mut i = 0;
                    while i < pending.len() {
                        if pending[i].0 <= now {
                            voices.push(Voice::new(&pending.swap_remove(i).1));
                        } else {
                            i += 1;
                        }
                    }
                    let (mut l, mut r) = (0.0f32, 0.0f32);
                    for v in voices.iter_mut() {
                        let s = v.sample(dt) as f32;
                        l += s * (0.5 * (1.0 - v.pan)) as f32;
                        r += s * (0.5 * (1.0 + v.pan)) as f32;
                    }
                    voices.retain(|v| !v.done());
                    let (ol, or) = fx.process(l, r);
                    frame[0] = ol.clamp(-1.0, 1.0);
                    if channels > 1 {
                        frame[1] = or.clamp(-1.0, 1.0);
                    }
                    now += 1;
                }
            },
            |err| eprintln!("emotune: audio error: {err}"),
            None,
        )
        .expect("failed to build audio stream");
    stream.play().expect("failed to start audio stream");
    stream
}
