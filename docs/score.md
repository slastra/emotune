# emotune scores (v2)

Mood recipes (`recipes.toml`) use this same format; see *Recipes* at the end.

`emotune play <file.toml|->` performs a composed score through the running
daemon. `--check` analyzes without playing and prints a report with warnings;
`--json` emits the report as JSON. Errors exit 1 with a positioned message.
The format is designed for LLM composition: compact, note-name based, and
impossible to mis-syntax into anything worse than a parse error.

```toml
tempo = 140          # bpm, 20..=480
swing = 0.12         # optional, 0..=0.4: delays offbeat eighths
humanize = 0.01      # optional, 0..=0.05 beats of timing jitter

[fx]                 # optional; moods reset it afterwards
echo = 0.16          # delay seconds 0.03..=0.9   (default 0.19)
feedback = 0.35      # echo feedback 0..=0.9      (default 0.335)
verb = 0.4           # reverb wet mix 0..=0.6     (default 0.18)

[[tracks]]
name = "melody"      # how sections refer to it (defaults to the voice name)
voice = "pulse"      # see Voices below
amp = 0.22           # clamped to 0.35
pan = 0.2            # -1..1
bend = 1.12          # pitch glide, applied only to notes marked ~
rate = 4.0           # wobble LFO Hz (wobble voice only)
duty = 0.125         # pulse/arp starting width 0.05..=0.5 (default 0.125, arp 0.25)
notes = "a4:0.75~ r:0.25 c5:0.5? e5 f5:1"
```

## Voices

| voice | sound |
|---|---|
| `pulse` (alias `tone`) | chip pulse lead: PWM drift toward square, a detuned second square, vibrato only on held notes, a pitch bite on the attack |
| `arp` | tracker arpeggio: a chord (`c4+e4+g4`) cycles in one voice at 40 Hz |
| `tri` (alias `soft`) | 4-bit stepped triangle, NES-style; bass or a soft lead |
| `noise` | LFSR hiss; the pitch sets its brightness (`c4` dark, `c6` bright) |
| `metal` | short-mode LFSR: tonal, ringing |
| `womp` | two falling, lowpassed pulses |
| `bell` | 2-op FM chime |
| `wobble` | LFO-swept lowpass over pulse + sub (`rate` Hz) |
| `kick` `snare` `hat` | drums |

Chip voices use 60 Hz stepped volume envelopes: full on the attack, sagging
toward 55%, a 70 ms release after the written duration. Percussion voices
(`noise`, `metal`, drums) are not transposed by `vary`.

## Notation

Whitespace-separated tokens; each note advances the cursor by its duration.

| token | meaning |
|---|---|
| `c4` `f#3` `bb2` | pitch name, `c4` = middle C (midi 60) |
| `x` | percussion hit (kick 36, snare 38, hat 42, noise 72, metal 84) |
| `r` | rest |
| `:1.5` | duration in beats, **sticky** until changed (initial 1) |
| `!` / `?` | accent (amp ×1.3) / ghost note (×0.6) |
| `~` | apply the track's `bend` to this note |
| `c4+e4+g4` | chord (shared start and duration) |
| `(...)x4` | repeat group, nestable to 4 |
| `(...):2` | time-squash the group into 2 beats (also sets the sticky dur) |
| `\|` | barline, ignored (readability only) |

`x:0.5 x! r x?` is a bar of drums. `(c5 e5 g5)x2` repeats the figure twice.
Modifiers combine with durations in either order: `g#5:1.5!` and `g#5!:1.5`
both work.

## Sections

For longer pieces, name your phrases once and arrange them:

```toml
structure = "A A B A"

[sections.A]
melody = "a4 c5 e5:2"
kick   = "(x:1)x4"

[sections.B]
melody = "f5 e5 c5:2"
```

Section length = its longest track; sections play back to back in `structure`
order. With `structure` present, tracks must not carry `notes` directly.

Forgiving corners, learned from real LLM output: a `structure = ...` line that
TOML would swallow into a preceding `[table]` is recovered from the text, and
if there are no `[[tracks]]` at all, section keys that are voice names get
default tracks inferred. Still, put `structure` at the top and declare tracks
when you want non-default amp/pan/bend.

## The report

`--check` is the feedback loop: compose, check, revise against the warnings.

```
score ok: 70 events, 6.2s @ 140bpm
  bass (womp): 11 notes, a1–a2, 1.8/s
  melody (tone): 16 notes, f4–g#5, 2.8/s
polyphony peak 7, est. level peak 1.32
warnings:
  - track "hat": 14 notes, all identical velocity (consider ! and ?)
```

Warnings cover flat velocity, machine-gun density, hot summed levels, unused
tracks/sections, and over-length jingles. Structural mistakes (unknown pitch,
unmatched parens, undefined section) are hard errors with the location.

An unknown voice name is an error.

Limits: ≤1500 events, beats 0..=256, pitches c0..=c9 region (midi 12..=120),
note duration ≤4s after tempo scaling.

A complete worked example lives at the repo's
[boss-battle score](../README.md) style: see `emotune play --check` output
above; the v1-to-v2 conversion of the same piece is in the git history.

## Recipes

A recipe is a score under a name, usually with **variants** and a **`vary`**
table, so a motif stays recognisable but never plays identically twice:

```toml
[success]
tempo = 320
vary = { transpose = [0, 0, 2, -3], tempo = 0.05, grace = 0.25 }
tracks = [
  { voice = "pulse", amp = 0.17, notes = ["c6:0.25 e6 g6 c7:1", "c6:0.25 g6 e6 c7:1"] },
  { voice = "tri",   amp = 0.14, notes = ["c4:0.75 c5:1",       "c4:0.75 c5:1"] },
]
```

- `notes` may be a list: one index is picked per play and shared by every
  track (shorter lists wrap). The daemon never repeats the last index.
- `vary.transpose`: semitone offsets, one picked per play (repeat a value to
  weight it). `vary.tempo`: jitter fraction, up to 0.2. `vary.grace`: chance
  of a chromatic grace note into one lead note.
- A score file may carry variants and `vary` too; `emotune play` performs
  variant 1 as written, and `--check` compiles every variant.

`emotune demo [name...]` auditions recipes; `emotune render <name|file> out.wav
[variant]` renders offline and prints the peak level. User recipes in
`~/.config/emotune/recipes.toml` merge over the built-ins by name (restart the
daemon to reload); a broken recipe is skipped with a message, the rest load.
