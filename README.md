# emotune

Chiptune mood-cue daemon: send it a feeling, hear a motif you learn to recognise.

One cheap shell call plays a short synthesized cue matching the moment.

## Install

```sh
cargo install --path .
emotune daemon        # or run it under a service manager
```

A systemd user unit is the intended home:

```ini
[Unit]
Description=emotune mood-melody daemon
After=pipewire.service

[Service]
ExecStart=%h/.local/bin/emotune daemon
Restart=on-failure

[Install]
WantedBy=default.target
```

With the unit installed, `emotune start|stop|restart|status` wraps systemctl.

## Usage

```sh
emotune "tests are green"      # cheerful ascending run
emotune "build failed"         # falling, sagging "uh-oh"
emotune "should I deploy?"     # unresolved rise
```

A phrase that names a recipe plays it; otherwise keywords map it to a mood
(`success`, `big_success`, `error`, `warning`, `question`, `thinking`,
`progress`, `hello`, `goodbye`, `info`). Raw OSC senders get the same
resolution in the daemon. Each mood is a short **motif** with a few written
variants, performed with a little transposition, tempo drift, and the odd
grace note: recognisable without looking, never quite identical. The voices
are chip-style: a bandlimited PWM pulse, tracker arpeggios, a 4-bit stepped
triangle, and LFSR noise.

It also performs composed pieces. `emotune play score.toml` plays an exact
multi-track score on the same chip voices plus drums (kick, snare, hat),
an FM bell, an LFO wobble bass, and per-score echo and reverb settings. Scores are plain TOML data,
which makes them a friendly target for LLM composition: have a model write
a score, validate it with `emotune play --check`, and nothing it hallucinates
can do worse than fail parsing. See [docs/score.md](docs/score.md).

```
emotune daemon                     run the synth daemon
emotune start|stop|restart|status  manage emotune.service
emotune play [--check] [--json] <score.toml|->
emotune demo [recipe...]           audition recipes one after another
emotune list                       recipe names
emotune render <recipe|score.toml> <out.wav> [variant]
emotune --version
emotune "major success"            play a recipe by name, or a mood for any phrase
```

## Configuration

Mood recipes live in `recipes.toml` (compiled in), written in the score
format with variants and a `vary` table ([docs/score.md](docs/score.md#recipes)).
Recipes in `~/.config/emotune/recipes.toml` merge over them at startup: the
same name replaces a built-in, a new name adds a mood. `emotune restart` to
reload, no rebuild. Timbres and effects are in `src/synth.rs`.

## How it works

A single Rust binary. The daemon holds a cpal audio stream and listens for
OSC over UDP on port 57120; the client classifies the phrase and sends it.
All synthesis is in-process: chip voices with 60 Hz stepped envelopes, a
ping-pong echo into a small Schroeder reverb, and a console-style output
stage (DC block, 12 kHz lowpass, soft ceiling). Once the tails die away the
callback skips all DSP and writes silence. No SuperCollider, no samples, no
runtime dependencies beyond an audio server.

## Development

```sh
cargo build --release
cargo test
```

## License

MIT © 2026 Shaun Lastra. See [LICENSE](LICENSE).
