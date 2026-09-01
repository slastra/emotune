# emotune scores

`emotune play <file.toml|->` performs a composed score through the running
daemon (validated locally first; exit 1 with a message on a bad score).
Unlike the mood recipes, which *improvise*, a score is played exactly as
written — it's the target format for LLM-composed jingles.

```toml
tempo = 128          # bpm, 20..=300

[fx]                 # optional; omitted fields keep defaults; moods reset it
echo = 0.23          # echo delay seconds, 0.03..=0.9   (default 0.19)
feedback = 0.45      # echo feedback,      0..=0.9      (default 0.335)
verb = 0.3           # reverb wet mix,     0..=0.6      (default 0.18)

[[tracks]]
voice = "bell"       # tone | soft | womp | bell | wobble | kick | snare | hat
amp = 0.2            # track loudness, clamped to 0.35
pan = -0.2           # -1..1, optional
bend = 1.0           # pitch glide multiplier over each note, 0.5..2, optional
rate = 4.0           # wobble LFO Hz, 0.25..16, optional (wobble only)
notes = [            # [beat, midinote, dur_beats] (+ optional amp multiplier)
  [0, 77, 0.5], [0.5, 80, 0.5], [1, 84, 1.5, 0.8],
]
```

Voices: `tone` chip pulse w/ vibrato · `soft` mellow triangle · `womp` falling
bass · `bell` FM chime · `wobble` LFO-filtered bass (uses `rate`) · `kick`,
`snare`, `hat` drums (pitch mostly ignored; 36/38/42 by convention).

Limits: ≤600 events, beats 0..=64, midinote 12..=120, note dur ≤4s.
