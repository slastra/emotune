use std::f64::consts::TAU;
use std::sync::mpsc::Receiver;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

use crate::gen::{Event, VoiceKind, ARP_MAX};

use crate::score::FxCfg;

const ECHO_GAIN: f32 = 0.35;
const ECHO_MAX_SECS: f64 = 1.0;

/// Chip "tick": stepped envelopes update at this rate, like a tracker frame.
const FRAME: f64 = 1.0 / 60.0;
/// Arp chord step.
const ARP_STEP: f64 = 1.0 / 40.0;
/// Release tail of the chip voices after their written duration.
const REL: f64 = 0.07;

struct Voice {
    kind: VoiceKind,
    freq: f64,
    dur: f64,
    amp: f64,
    pan: f64,
    bend: f64,
    rate: f64,
    duty: f64,
    arp: [f32; ARP_MAX],
    arp_n: usize,
    t: f64,
    ph1: f64,
    ph2: f64,
    lp: f64,
    /// stepped envelope, smoothed ~1.5 ms so 60 Hz steps don't click
    env: f64,
    lfsr: u16,
    noise: u32,
}

/// PolyBLEP residual: removes most aliasing from a hard edge at phase 0.
fn blep(t: f64, dt: f64) -> f64 {
    if t < dt {
        let x = t / dt;
        x + x - x * x - 1.0
    } else if t > 1.0 - dt {
        let x = (t - 1.0) / dt;
        x * x + x + x + 1.0
    } else {
        0.0
    }
}

/// Bandlimited pulse of width `duty` at phase `ph` advancing `inc` per sample.
fn pulse(ph: f64, duty: f64, inc: f64) -> f64 {
    let naive = if ph < duty { 1.0 } else { -1.0 };
    naive + blep(ph, inc) - blep((ph - duty).rem_euclid(1.0), inc)
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
            duty: e.duty,
            arp: e.arp,
            arp_n: (e.arp_n as usize).clamp(1, ARP_MAX),
            t: 0.0,
            ph1: 0.0,
            ph2: 0.0,
            lp: 0.0,
            env: 0.0,
            lfsr: 1,
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
            VoiceKind::Pulse | VoiceKind::Arp | VoiceKind::Tri => self.dur + REL + 0.01,
            VoiceKind::Noise | VoiceKind::Metal => self.dur + 0.01,
            VoiceKind::Womp => 0.005 + self.dur,
            VoiceKind::Bell => 0.005 + self.dur * 2.0,
            VoiceKind::Kick | VoiceKind::Snare | VoiceKind::Hat | VoiceKind::Wobble => {
                0.005 + self.dur
            }
        }
    }

    fn done(&self) -> bool {
        self.t >= self.total()
    }

    /// Tracker-style volume: sampled once per 60 Hz frame and quantized to 16
    /// levels, then smoothed just enough to keep the steps from clicking.
    /// Notes hit full, sag toward 0.55, and release over REL after `dur`.
    fn chip_env(&mut self, t: f64, dt: f64) -> f64 {
        let tq = (t / FRAME).floor() * FRAME;
        let sag = |x: f64| 0.55 + 0.45 * (-x / 0.12).exp();
        let target = if tq < self.dur {
            sag(tq)
        } else {
            sag(self.dur) * (1.0 - (tq - self.dur) / REL).max(0.0)
        };
        let target = (target * 15.0).round() / 15.0;
        self.env += (target - self.env) * (1.0 - (-dt / 0.0015).exp());
        self.env
    }

    /// Noise drums decay instead of sustaining.
    fn noise_env(&mut self, t: f64, dt: f64) -> f64 {
        let tq = (t / FRAME).floor() * FRAME;
        let target = ((-tq / (self.dur * 0.35)).exp() * 15.0).round() / 15.0;
        self.env += (target - self.env) * (1.0 - (-dt / 0.001).exp());
        self.env
    }

    fn sample(&mut self, dt: f64) -> f64 {
        let t = self.t;
        self.t += dt;
        let glide = if self.bend == 1.0 {
            1.0
        } else {
            self.bend.powf((t / self.dur).min(1.0))
        };
        let s = match self.kind {
            VoiceKind::Pulse => {
                // pitch bite on the attack, vibrato only once a note is held
                let bite = 1.0 + 0.06 * (-t / 0.008).exp();
                let depth = 0.005 * ((t - 0.12) / 0.15).clamp(0.0, 1.0);
                let vib = 1.0 + depth * (TAU * 5.5 * t).sin();
                let f = self.freq * glide * bite * vib;
                let inc = (f * dt).min(0.45);
                self.ph1 = (self.ph1 + inc).fract();
                // PWM: the width drifts from `duty` halfway toward square
                let duty = self.duty + (0.5 - self.duty) * 0.5 * (t / self.total()).min(1.0);
                let a = pulse(self.ph1, duty, inc);
                // a quiet square 6 cents sharp thickens it, SID-style
                let inc2 = inc * 1.0035;
                self.ph2 = (self.ph2 + inc2).fract();
                let b = pulse(self.ph2, 0.5, inc2);
                (a * 0.78 + b * 0.22) * self.chip_env(t, dt) * 0.6
            }
            VoiceKind::Arp => {
                let step = (t / ARP_STEP) as usize % self.arp_n;
                let f = self.freq * 2f64.powf(self.arp[step] as f64 / 12.0) * glide;
                let inc = (f * dt).min(0.45);
                self.ph1 = (self.ph1 + inc).fract();
                pulse(self.ph1, self.duty, inc) * self.chip_env(t, dt) * 0.55
            }
            VoiceKind::Tri => {
                // NES-style: the triangle walks 16 steps up and 16 down
                let depth = 0.004 * ((t - 0.15) / 0.15).clamp(0.0, 1.0);
                let vib = 1.0 + depth * (TAU * 5.0 * t).sin();
                self.ph1 = (self.ph1 + self.freq * glide * vib * dt).fract();
                let tri = 4.0 * (self.ph1 - 0.5).abs() - 1.0;
                let stepped = ((tri + 1.0) * 7.5).round() / 7.5 - 1.0;
                stepped * self.chip_env(t, dt) * 0.9
            }
            VoiceKind::Noise | VoiceKind::Metal => {
                // 15-bit LFSR clocked from the note's pitch; metal taps bit 6
                // instead of bit 1 for the short, ringing 93-step sequence
                let clock = self.freq * 64.0 * glide;
                self.ph1 += (clock * dt).min(1.0);
                if self.ph1 >= 1.0 {
                    self.ph1 -= 1.0;
                    let tap = if self.kind == VoiceKind::Metal { 6 } else { 1 };
                    let fb = (self.lfsr ^ (self.lfsr >> tap)) & 1;
                    self.lfsr = (self.lfsr >> 1) | (fb << 14);
                }
                let s = if self.lfsr & 1 == 1 { 1.0 } else { -1.0 };
                s * self.noise_env(t, dt) * 0.5
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

    /// Silence every delay line, so an idle stretch resumes from true zero.
    fn clear(&mut self) {
        self.comb.fill(0.0);
        self.right.fill(0.0);
        for (buf, _, damp) in &mut self.verb_combs {
            buf.fill(0.0);
            *damp = 0.0;
        }
        for (buf, _) in &mut self.verb_ap {
            buf.fill(0.0);
        }
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

/// The output stage: a DC blocker and a gentle 12 kHz lowpass, roughly what
/// a console's analog output does to its square waves, then a soft ceiling.
struct Console {
    hp_r: f32,
    lp_a: f32,
    x1: [f32; 2],
    y1: [f32; 2],
    lp: [f32; 2],
}

impl Console {
    fn new(sr: f64) -> Self {
        Console {
            hp_r: (1.0 - TAU * 25.0 / sr) as f32,
            lp_a: (1.0 - (-TAU * 12000.0 / sr).exp()) as f32,
            x1: [0.0; 2],
            y1: [0.0; 2],
            lp: [0.0; 2],
        }
    }

    fn clear(&mut self) {
        self.x1 = [0.0; 2];
        self.y1 = [0.0; 2];
        self.lp = [0.0; 2];
    }

    fn process(&mut self, l: f32, r: f32) -> (f32, f32) {
        let mut ch = |i: usize, x: f32| {
            let y = x - self.x1[i] + self.hp_r * self.y1[i];
            self.x1[i] = x;
            self.y1[i] = y;
            self.lp[i] += self.lp_a * (y - self.lp[i]);
            // linear below 0.8, then a tanh knee instead of a hard clip
            let v = self.lp[i];
            if v.abs() <= 0.8 {
                v
            } else {
                v.signum() * (0.8 + 0.2 * ((v.abs() - 0.8) / 0.2).tanh())
            }
        };
        (ch(0, l), ch(1, r))
    }
}

/// The mixer: schedules events, runs voices through the FX and output stage.
/// Lives inside the audio callback; also renders offline for tests and
/// `emotune render`.
pub struct Engine {
    sr: f64,
    fx: Fx,
    out: Console,
    voices: Vec<Voice>,
    /// sorted latest-first, so the next due event is always at the end
    pending: Vec<(u64, Event)>,
    now: u64,
    /// frames since the output last made a sound with no voices playing
    quiet: u64,
    /// after this many quiet frames, skip all DSP until the next event
    idle_after: u64,
}

impl Engine {
    pub fn new(sr: f64) -> Self {
        let idle_after = (sr * 0.25) as u64;
        Engine {
            sr,
            fx: Fx::new(sr),
            out: Console::new(sr),
            voices: Vec::with_capacity(64),
            pending: Vec::with_capacity(2048),
            now: 0,
            quiet: idle_after,
            idle_after,
        }
    }

    /// Schedule a batch relative to now and apply its FX settings.
    pub fn push(&mut self, batch: Vec<Event>, cfg: FxCfg) {
        self.fx.set(cfg);
        for e in batch {
            self.pending
                .push((self.now + (e.offset * self.sr) as u64, e));
        }
        self.pending
            .sort_unstable_by_key(|p| std::cmp::Reverse(p.0));
    }

    /// True once everything has played out and the tails have died away.
    pub fn idle(&self) -> bool {
        self.voices.is_empty() && self.pending.is_empty() && self.quiet >= self.idle_after
    }

    /// Fill an interleaved buffer.
    pub fn fill(&mut self, data: &mut [f32], channels: usize) {
        if self.idle() {
            data.fill(0.0);
            self.now += (data.len() / channels) as u64;
            return;
        }
        let dt = 1.0 / self.sr;
        for frame in data.chunks_mut(channels) {
            while self.pending.last().is_some_and(|(at, _)| *at <= self.now) {
                let (_, e) = self.pending.pop().expect("checked");
                self.voices.push(Voice::new(&e));
            }
            let (mut l, mut r) = (0.0f32, 0.0f32);
            for v in self.voices.iter_mut() {
                let s = v.sample(dt) as f32;
                l += s * (0.5 * (1.0 - v.pan)) as f32;
                r += s * (0.5 * (1.0 + v.pan)) as f32;
            }
            let (wl, wr) = self.fx.process(l, r);
            let (ol, or) = self.out.process(wl, wr);
            frame[0] = ol;
            if channels > 1 {
                frame[1] = or;
            }
            self.now += 1;
            if self.voices.is_empty() && ol.abs() + or.abs() < 1e-4 {
                self.quiet += 1;
                if self.quiet == self.idle_after {
                    self.fx.clear();
                    self.out.clear();
                }
            } else {
                self.quiet = 0;
            }
        }
        // finished voices output silence, so pruning per buffer is enough
        self.voices.retain(|v| !v.done());
    }
}

/// Render events offline to interleaved stereo until the engine goes idle
/// (capped at `max_secs`).
pub fn render(events: Vec<Event>, cfg: FxCfg, sr: f64, max_secs: f64) -> Vec<f32> {
    let mut engine = Engine::new(sr);
    engine.push(events, cfg);
    let mut out = Vec::new();
    let mut buf = [0.0f32; 1024];
    while !engine.idle() && (out.len() as f64) < max_secs * sr * 2.0 {
        engine.fill(&mut buf, 2);
        out.extend_from_slice(&buf);
    }
    out
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
    let mut engine = Engine::new(sr);

    let stream = device
        .build_output_stream(
            &config.into(),
            move |data: &mut [f32], _| {
                for (batch, cfg) in rx.try_iter() {
                    engine.push(batch, cfg);
                }
                engine.fill(data, channels);
            },
            |err| eprintln!("emotune: audio error: {err}"),
            None,
        )
        .expect("failed to build audio stream");
    stream.play().expect("failed to start audio stream");
    stream
}
