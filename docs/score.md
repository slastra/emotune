# emotune scores (v2)

`emotune play <file.toml|->` performs a composed score through the running
daemon. `--check` analyzes without playing and prints a report with warnings;
`--json` emits the report as JSON. Errors exit 1 with a positioned message.
The format is designed for LLM composition: compact, note-name based, and
impossible to mis-syntax into anything worse than a parse error.

```toml
tempo = 140          # bpm, 20..=300
swing = 0.12         # optional, 0..=0.4: delays offbeat eighths
humanize = 0.01      # optional, 0..=0.05 beats of timing jitter

[fx]                 # optional; moods reset it afterwards
echo = 0.16          # delay seconds 0.03..=0.9   (default 0.19)
feedback = 0.35      # echo feedback 0..=0.9      (default 0.335)
verb = 0.4           # reverb wet mix 0..=0.6     (default 0.18)

[[tracks]]
name = "melody"      # how sections refer to it (defaults to the voice name)
voice = "tone"       # tone | soft | womp | bell | wobble | kick | snare | hat
amp = 0.22           # clamped to 0.35
pan = 0.2            # -1..1
bend = 1.12          # pitch glide, applied only to notes marked ~
rate = 4.0           # wobble LFO Hz (wobble voice only)
notes = "a4:0.75~ r:0.25 c5:0.5? e5 f5:1"
```

## Notation

Whitespace-separated tokens; each note advances the cursor by its duration.

| token | meaning |
|---|---|
| `c4` `f#3` `bb2` | pitch name, `c4` = middle C (midi 60) |
| `x` | percussion hit (kick 36, snare 38, hat 42) |
| `r` | rest |
| `:1.5` | duration in beats, **sticky** until changed (initial 1) |
| `!` / `?` | accent (amp ×1.3) / ghost note (×0.6) |
| `~` | apply the track's `bend` to this note |
| `c4+e4+g4` | chord (shared start and duration) |
| `(...)x4` | repeat group, nestable to 4 |
| `\|` | barline, ignored (readability only) |

`x:0.5 x! r x?` is a bar of drums. `(c5 e5 g5):0.5` then `x2` arpeggiates
twice. Modifiers and duration combine: `g#5:1.5!`.

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

Limits: ≤1500 events, beats 0..=256, pitches c0..=c9 region (midi 12..=120),
note duration ≤4s after tempo scaling.

A complete worked example lives at the repo's
[boss-battle score](../README.md) style: see `emotune play --check` output
above; the v1-to-v2 conversion of the same piece is in the git history.
