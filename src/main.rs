mod classify;
mod gen;
mod notation;
mod recipes;
mod score;
mod synth;

use std::io::Read;
use std::net::UdpSocket;
use std::process::{exit, Command};
use std::sync::mpsc;

use rand::SeedableRng;
use rosc::{decoder, encoder, OscMessage, OscPacket, OscType};

const PORT: u16 = 57120;

const USAGE: &str = "emotune — send a feeling, hear a melody.

Usage:
  emotune daemon                     run the synth daemon
  emotune start|stop|restart|status  manage emotune.service
  emotune play [--check] [--json] <score.toml|->
                                     play a composed score; --check analyzes
                                     without playing, --json for machines
  emotune \"major success\"            play a mood for any phrase";

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.is_empty() {
        eprintln!("{USAGE}");
        exit(1);
    }
    match args[0].as_str() {
        "daemon" => daemon(),
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
                println!("{}", serde_json::to_string_pretty(&report).expect("report json"));
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

fn send(addr: &str, payload: &str) {
    let shown = if addr == "/emotune" {
        let mood = classify::classify(payload);
        println!("{payload:?} -> {mood}");
        mood.to_string()
    } else {
        payload.to_string()
    };
    let msg = OscPacket::Message(OscMessage {
        addr: addr.into(),
        args: vec![OscType::String(shown)],
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
    println!("emotune daemon ready on OSC port {PORT}");
    // audible "I'm up" cue
    if let Some(r) = recipes.get("info") {
        tx.send((gen::melody(r, &mut rng), score::FxCfg::default())).ok();
    }

    let mut buf = [0u8; 32768];
    loop {
        let Ok((n, _)) = sock.recv_from(&mut buf) else { continue };
        let Ok((_, OscPacket::Message(m))) = decoder::decode_udp(&buf[..n]) else {
            continue;
        };
        let Some(OscType::String(arg)) = m.args.into_iter().next() else {
            continue;
        };
        match m.addr.as_str() {
            "/emotune" => {
                println!("emotune: {arg}");
                if let Some(r) = recipes.get(&arg).or_else(|| recipes.get("info")) {
                    tx.send((gen::melody(r, &mut rng), score::FxCfg::default())).ok();
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
