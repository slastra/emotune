// Phrase → mood keyword table, ported from the Python client.
// Order matters: "major success" must hit big_success before success.
const MOODS: &[(&str, &[&str])] = &[
    ("big_success", &["major success", "huge", "epic", "shipped", "celebrate", "nailed"]),
    ("success", &["success", "done", "pass", "green", "complete", "fixed", "works", "deployed"]),
    ("error", &["wrong", "error", "fail", "broke", "crash", "bad", "died", "red"]),
    ("warning", &["warn", "careful", "attention", "slow", "sketchy"]),
    ("question", &["?", "question", "input", "decide", "which", "should"]),
    ("thinking", &["thinking", "hmm", "pondering", "working on", "searching"]),
    ("progress", &["progress", "milestone", "step", "halfway", "building"]),
    ("goodbye", &["bye", "goodnight", "shutting down", "later", "signing off"]),
];

pub fn classify(phrase: &str) -> &'static str {
    let p = phrase.to_lowercase();
    for (mood, words) in MOODS {
        if words.iter().any(|w| p.contains(w)) {
            return mood;
        }
    }
    "info"
}
