# emotune

Generative chiptune mood daemon: send it a feeling, hear a melody.

One cheap shell call plays a short synthesized melody matching the moment.

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
emotune "build failed"         # low womp
emotune "should I deploy?"     # unresolved rise
```

Keywords in the phrase map to a mood (`success`, `big_success`, `error`,
`warning`, `question`, `thinking`, `progress`, `hello`, `goodbye`, `info`), and each
mood improvises a fresh melody from a recipe: a scale, a drift direction, a
note-count range, a resolution degree. Same feeling, never the same tune.

It also performs composed pieces. `emotune play score.toml` plays an exact
multi-track score with drums (kick, snare, hat), an FM bell, an LFO wobble
bass, and per-score echo and reverb settings. Scores are plain TOML data,
which makes them a friendly target for LLM composition: have a model write
a score, validate it with `emotune play --check`, and nothing it hallucinates
can do worse than fail parsing. See [docs/score.md](docs/score.md).

```
emotune daemon                     run the synth daemon
emotune start|stop|restart|status  manage emotune.service
emotune play [--check] [--json] <score.toml|->
emotune "major success"            play a mood for any phrase
```

## Configuration

Mood recipes live in `recipes.toml` (compiled in). Recipes in
`~/.config/emotune/recipes.toml` merge over them at startup: the same name
replaces a built-in, a new name adds a mood. `emotune restart` to reload, no rebuild.
Timbres and effects are in `src/synth.rs`.

## How it works

A single Rust binary. The daemon holds a cpal audio stream and listens for
OSC over UDP on port 57120; the client classifies the phrase and sends it.
All synthesis is in-process: chip-style pulse, triangle, and FM voices with
a ping-pong echo into a small Schroeder reverb. No SuperCollider, no
samples, no runtime dependencies beyond an audio server.

## Development

```sh
cargo build --release
cargo test
```

## License

MIT © 2026 Shaun Lastra. See [LICENSE](LICENSE).
