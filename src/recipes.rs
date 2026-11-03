use std::collections::HashMap;

use rand::SeedableRng;

use crate::score::{self, Score};

const DEFAULT: &str = include_str!("../recipes.toml");

/// Parse a recipe set and compile every variant of every recipe once, so a
/// broken motif is reported at load instead of going silent at play time.
/// Bad recipes are dropped with a message; the rest still load.
fn parse_set(text: &str, origin: &str) -> HashMap<String, Score> {
    let all: HashMap<String, Score> = match toml::from_str(text) {
        Ok(m) => m,
        Err(e) => {
            eprintln!("emotune: bad {origin} ({e}); skipping it");
            return HashMap::new();
        }
    };
    let mut rng = rand::rngs::SmallRng::seed_from_u64(0);
    all.into_iter()
        .filter(|(name, s)| {
            let bad = (0..s.variants()).find_map(|v| score::compile(s, v, &mut rng).err());
            if let Some(e) = &bad {
                eprintln!("emotune: {origin}: recipe {name:?}: {e}");
            }
            bad.is_none()
        })
        .collect()
}

/// Load recipes: the compiled-in set, with ~/.config/emotune/recipes.toml merged
/// over it (a user recipe replaces the built-in of the same name; new names add).
pub fn load() -> HashMap<String, Score> {
    let mut map = parse_set(DEFAULT, "built-in recipes.toml");
    let Ok(home) = std::env::var("HOME") else {
        return map;
    };
    let path = format!("{home}/.config/emotune/recipes.toml");
    if let Ok(text) = std::fs::read_to_string(&path) {
        map.extend(parse_set(&text, &path));
    }
    map
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_builtin_recipe_compiles() {
        let all: HashMap<String, Score> = toml::from_str(DEFAULT).unwrap();
        let kept = parse_set(DEFAULT, "built-in");
        assert_eq!(kept.len(), all.len(), "a built-in recipe failed to compile");
        for name in [
            "info",
            "question",
            "success",
            "error",
            "hello",
            "goodbye",
            "doc_listen",
        ] {
            assert!(kept.contains_key(name), "missing {name}");
        }
    }

    #[test]
    fn every_variant_renders_clean_and_returns_to_silence() {
        let mut rng = rand::rngs::SmallRng::seed_from_u64(3);
        for (name, s) in parse_set(DEFAULT, "built-in") {
            for v in 0..s.variants() {
                for _ in 0..4 {
                    let (ev, fx, _) = score::compile(&s, v, &mut rng).unwrap();
                    let pcm = crate::synth::render(ev, fx, 48000.0, 10.0);
                    let secs = pcm.len() as f64 / 96000.0;
                    let peak = pcm.iter().fold(0.0f32, |m, x| m.max(x.abs()));
                    assert!(pcm.iter().all(|x| x.is_finite()), "{name} v{v}: NaN");
                    assert!(
                        peak < 0.8,
                        "{name} v{v}: peak {peak:.2} reaches the soft clip"
                    );
                    assert!(peak > 0.03, "{name} v{v}: peak {peak:.3}, barely audible");
                    assert!(secs < 6.0, "{name} v{v}: {secs:.1}s before going idle");
                }
            }
        }
    }

    #[test]
    fn classifier_moods_all_have_recipes() {
        let kept = parse_set(DEFAULT, "built-in");
        for mood in crate::classify::MOODS
            .iter()
            .map(|(m, _)| *m)
            .chain(["info"])
        {
            assert!(
                kept.contains_key(mood),
                "classify can return {mood:?} but no recipe"
            );
        }
    }
}
