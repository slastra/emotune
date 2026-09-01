# emotune

Send a feeling, hear a melody. A tiny generative chiptune daemon: one cheap
shell call plays a short synthesized melody matching the moment.

```sh
emotune "tests are green"      # cheerful ascending run
emotune "build failed"         # low womp
emotune "should I deploy?"     # unresolved rise
```

Keywords in the phrase map to a mood (`success`, `big_success`, `error`,
`warning`, `question`, `thinking`, `progress`, `goodbye`, `info`), and each
mood improvises a fresh melody from a recipe: a scale, a drift direction, a
note-count range, a resolution degree. Same feeling, never the same tune.

It also performs composed pieces. `emotune play score.toml` plays an exact
multi-track score with drums (kick, snare, hat), an FM bell, an LFO wobble
bass, and per-score echo and reverb settings. Scores are plain TOML data,
which makes them a friendly target for LLM composition: have a model write
a score, validate it with `emotune play --check`, and nothing it hallucinates
can do worse than fail parsing. See [docs/score.md](docs/score.md).

## How it works

A single Rust binary. The daemon holds a cpal audio stream and listens for
OSC over UDP on port 57120; the client classifies the phrase and sends it.
All synthesis is in-process: chip-style pulse, triangle, and FM voices with
a ping-pong echo into a small Schroeder reverb. No SuperCollider, no
samples, no runtime dependencies beyond an audio server.

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

## Customizing

Mood recipes live in `recipes.toml` (compiled in;
`~/.config/emotune/recipes.toml` overrides at runtime, no rebuild).
Timbres and effects are in `src/synth.rs`.

## License

MIT
