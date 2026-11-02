//! Plain-text helpers shared by the terminal core and its consumers.

/// Drop control characters except `\n` and `\t`, so text taken out of the
/// grid (or produced by a model) never carries escape sequences.
pub fn strip_controls(text: &str) -> String {
    text.chars()
        .filter(|c| !c.is_control() || matches!(c, '\n' | '\t'))
        .collect()
}
