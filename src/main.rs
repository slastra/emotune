mod classify;
mod gen;
mod notation;
mod recipes;
mod score;
mod synth;

use std::collections::HashMap;
use std::io::Read;
use std::net::UdpSocket;
use std::process::{exit, Command};
use std::sync::mpsc;
use std::time::Duration;

use rand::{Rng, SeedableRng};
use rosc::{decoder, encoder, OscMessage, OscPacket, OscType};

const PORT: u16 = 57120;

const USAGE: &str = "emotune — send a feeling, hear a melody.

Usage:
  emotune daemon                     run the synth daemon
  emotune start|stop|restart|status  manage emotune.service
  emotune play [--check] [--json] <score.toml|->
                                     play a composed score; --check analyzes
                                     without playing, --json for machines
  emotune demo [recipe...]           audition recipes one after another
  emotune list                       recipe names
  emotune render <recipe|score.toml> <out.wav> [variant]
                                     render offline to a 48 kHz WAV
  emotune --version
  emotune \"major success\"            play a recipe by name, or a mood for any phrase";

/// Audition order for `demo`: the cues heard most, then the rest by name.
const DEMO_FIRST: &[&str] = &[
    "info",
    "question",
    "success",
    "big_success",
    "error",
    "warning",
    "thinking",
    "progress",
    "hello",
    "goodbye",
];

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.is_empty() {
        eprintln!("{USAGE}");
        exit(1);
    }
    match args[0].as_str() {
        "daemon" => daemon(),
        "--version" | "-V" | "version" => println!("emotune {}", env!("CARGO_PKG_VERSION")),
        "list" => {
            let mut names: Vec<String> = recipes::load().into_keys().collect();
            names.sort();
            println!("{}", names.join("\n"));
        }
        "demo" => demo(&args[1..]),
        "render" => render(&args[1..]),
        cmd @ ("start" | "stop" | "restart" | "status") => {
            let status = Command::new("systemctl")
                .args(["--user", cmd, "emotune.service"])
                .status()
                .expect("failed to run systemctl");
            exit(status.code().unwrap_or(1));
        }
        "play" => {
            let check = args.iter().any(|a| a == "--check");
            let json = args.iter().any(|a| a == "--json");
            let Some(path) = args[1..].iter().find(|a| !a.starts_with("--")) else {
                eprintln!("{USAGE}");
                exit(1);
            };
            play(path, check, json);
        }
        _ => send("/emotune", &args.join(" ")),
    }
}

/// Validate a score locally, then (unless --check) hand it to the daemon.
fn play(path: &str, check_only: bool, json: bool) {
    let text = if path == "-" {
        let mut s = String::new();
        std::io::stdin().read_to_string(&mut s).expect("read stdin");
        s
    } else {
        std::fs::read_to_string(path).unwrap_or_else(|e| {
            eprintln!("emotune: cannot read {path}: {e}");
            exit(1);
        })
    };
    match score::parse(&text) {
        Ok((events, _, report)) => {
            if !check_only {
                send("/emotune/score", &text);
            }
            if json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&report).expect("report json")
                );
            } else if check_only {
                print!("{}", report.render());
            } else {
                println!("score ok: {} events, {:.1}s", events.len(), report.seconds);
            }
        }
        Err(e) => {
            eprintln!("emotune: bad score: {e}");
            exit(1);
        }
    }
}

/// Which recipe a phrase plays: an exact recipe name, else the mood the
/// keyword classifier picks. The daemon resolves raw OSC phrases the same way.
fn resolve<'a, V>(recipes: &'a HashMap<String, V>, phrase: &str) -> Option<(&'a str, &'a V)> {
    recipes
        .get_key_value(phrase)
        .or_else(|| recipes.get_key_value(classify::classify(phrase)))
        .or_else(|| recipes.get_key_value("info"))
        .map(|(k, v)| (k.as_str(), v))
}

/// Render a recipe (by name) or a score file to a WAV, offline.
fn render(args: &[String]) {
    let (Some(what), Some(out)) = (args.first(), args.get(1)) else {
        eprintln!("{USAGE}");
        exit(1);
    };
    let variant: usize = args.get(2).and_then(|v| v.parse().ok()).unwrap_or(1).max(1) - 1;
    let recipes = recipes::load();
    let score = match recipes.get(what) {
        Some(s) => s.clone(),
        None => {
            let text = std::fs::read_to_string(what).unwrap_or_else(|e| {
                eprintln!("emotune: no recipe {what:?} and cannot read it as a file: {e}");
                exit(1);
            });
            score::load(&text).unwrap_or_else(|e| {
                eprintln!("emotune: bad score: {e}");
                exit(1);
            })
        }
    };
    let mut rng = rand::rngs::SmallRng::from_entropy();
    let (events, fx, _) = score::compile(&score, variant, &mut rng).unwrap_or_else(|e| {
        eprintln!("emotune: {e}");
        exit(1);
    });
    let sr = 48000;
    let pcm = synth::render(events, fx, sr as f64, 60.0);
    let peak = pcm.iter().fold(0.0f32, |m, s| m.max(s.abs()));
    let mut wav = Vec::with_capacity(44 + pcm.len() * 2);
    let data_len = (pcm.len() * 2) as u32;
    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&(36 + data_len).to_le_bytes());
    wav.extend_from_slice(b"WAVEfmt ");
    for v in [16u32, 0x0002_0001, sr, sr * 4, 0x0010_0004] {
        wav.extend_from_slice(&v.to_le_bytes());
    }
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&data_len.to_le_bytes());
    for s in &pcm {
        wav.extend_from_slice(&((s.clamp(-1.0, 1.0) * 32767.0) as i16).to_le_bytes());
    }
    std::fs::write(out, wav).unwrap_or_else(|e| {
        eprintln!("emotune: cannot write {out}: {e}");
        exit(1);
    });
    println!(
        "{out}: {:.2}s, peak {:.2} ({:.1} dBFS), variant {}",
        pcm.len() as f64 / 2.0 / sr as f64,
        peak,
        20.0 * peak.max(1e-9).log10(),
        variant + 1
    );
}

/// Play recipes in turn, waiting out each one's length plus a gap.
fn demo(names: &[String]) {
    let recipes = recipes::load();
    let mut order: Vec<String> = if names.is_empty() {
        let mut rest: Vec<String> = recipes
            .keys()
            .filter(|k| !DEMO_FIRST.contains(&k.as_str()))
            .cloned()
            .collect();
        rest.sort();
        DEMO_FIRST
            .iter()
            .map(|s| s.to_string())
            .chain(rest)
            .collect()
    } else {
        names.to_vec()
    };
    order.retain(|n| {
        let ok = recipes.contains_key(n);
        if !ok {
            eprintln!("emotune: no recipe {n:?}");
        }
        ok
    });
    let mut rng = rand::rngs::SmallRng::from_entropy();
    for name in order {
        let s = &recipes[&name];
        let secs = score::compile(s, 0, &mut rng)
            .map(|(_, _, r)| r.seconds)
            .unwrap_or(1.0);
        let variants = s.variants();
        let note = if variants > 1 {
            format!("  ({variants} variants)")
        } else {
            String::new()
        };
        println!("{name:<14} {secs:.1}s{note}");
        send_raw("/emotune", &name);
        std::thread::sleep(Duration::from_secs_f64(secs + 0.9));
    }
}

fn send(addr: &str, payload: &str) {
    if addr == "/emotune" {
        let recipes = recipes::load();
        if let Some((name, _)) = resolve(&recipes, payload) {
            println!("{payload:?} -> {name}");
        }
    }
    send_raw(addr, payload);
}

fn send_raw(addr: &str, payload: &str) {
    let msg = OscPacket::Message(OscMessage {
        addr: addr.into(),
        args: vec![OscType::String(payload.to_string())],
    });
    let buf = encoder::encode(&msg).expect("osc encode");
    let sock = UdpSocket::bind("0.0.0.0:0").expect("bind");
    sock.send_to(&buf, ("127.0.0.1", PORT)).expect("send");
}

fn daemon() {
    let sock = UdpSocket::bind(("0.0.0.0", PORT))
        .unwrap_or_else(|e| panic!("cannot bind UDP {PORT}: {e}"));
    let recipes = recipes::load();
    let mut rng = rand::rngs::SmallRng::from_entropy();
    let (tx, rx) = mpsc::channel();
    let _stream = synth::start(rx);
    println!(
        "emotune {} ready on OSC port {PORT} ({} recipes)",
        env!("CARGO_PKG_VERSION"),
        recipes.len()
    );
    // last variant played per recipe, so a motif never repeats back to back
    let mut last: HashMap<String, usize> = HashMap::new();
    let mut perform = |name: &str, s: &score::Score, rng: &mut rand::rngs::SmallRng| {
        let n = s.variants();
        let mut v = rng.gen_range(0..n);
        if n > 1 && last.get(name) == Some(&v) {
            v = (v + rng.gen_range(1..n)) % n;
        }
        last.insert(name.to_string(), v);
        match score::compile(s, v, rng) {
            Ok((events, fx, _)) => {
                tx.send((events, fx)).ok();
            }
            Err(e) => eprintln!("emotune: recipe {name:?}: {e}"),
        }
    };
    // audible "I'm up" cue
    if let Some(s) = recipes.get("info") {
        perform("info", s, &mut rng);
    }

    let mut buf = [0u8; 32768];
    loop {
        let Ok((n, _)) = sock.recv_from(&mut buf) else {
            continue;
        };
        let Ok((_, OscPacket::Message(m))) = decoder::decode_udp(&buf[..n]) else {
            continue;
        };
        let Some(OscType::String(arg)) = m.args.into_iter().next() else {
            continue;
        };
        match m.addr.as_str() {
            "/emotune" => {
                if let Some((name, s)) = resolve(&recipes, &arg) {
                    if name == arg {
                        println!("emotune: {name}");
                    } else {
                        println!("emotune: {name} <- {arg:?}");
                    }
                    perform(name, s, &mut rng);
                }
            }
            "/emotune/score" => match score::parse(&arg) {
                Ok((events, fx, _)) => {
                    println!("emotune: score ({} events)", events.len());
                    tx.send((events, fx)).ok();
                }
                Err(e) => eprintln!("emotune: bad score: {e}"),
            },
            _ => {}
        }
    }
}
