use std::collections::HashMap;

use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
pub struct Recipe {
    pub scale: Vec<i32>,
    pub dir: i32,
    pub base: i32,
    pub notes: [usize; 2],
    pub dur: [f64; 2],
    pub end: Option<i32>,
    pub synth: String,
    pub amp: f64,
    pub bend: Option<f64>,
    #[serde(default)]
    pub arp: bool,
}

const DEFAULT: &str = include_str!("../recipes.toml");

/// Load recipes: the compiled-in set, with ~/.config/emotune/recipes.toml merged
/// over it (a user recipe replaces the built-in of the same name; new names add).
pub fn load() -> HashMap<String, Recipe> {
    let mut map: HashMap<String, Recipe> =
        toml::from_str(DEFAULT).expect("built-in recipes.toml is valid");
    let Ok(home) = std::env::var("HOME") else { return map };
    let path = format!("{home}/.config/emotune/recipes.toml");
    let Ok(text) = std::fs::read_to_string(&path) else { return map };
    match toml::from_str::<HashMap<String, Recipe>>(&text) {
        Ok(user) => map.extend(user),
        Err(e) => eprintln!("emotune: bad {path} ({e}); using built-in recipes only"),
    }
    map
}
