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

/// Load recipes: ~/.config/emotune/recipes.toml if present, else the compiled-in set.
pub fn load() -> HashMap<String, Recipe> {
    let user = std::env::var("HOME")
        .map(|h| format!("{h}/.config/emotune/recipes.toml"))
        .ok()
        .and_then(|p| std::fs::read_to_string(p).ok());
    let text = user.as_deref().unwrap_or(DEFAULT);
    match toml::from_str(text) {
        Ok(map) => map,
        Err(e) => {
            eprintln!("emotune: bad recipes.toml ({e}); using built-in");
            toml::from_str(DEFAULT).expect("built-in recipes.toml is valid")
        }
    }
}
