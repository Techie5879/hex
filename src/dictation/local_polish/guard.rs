//! Reject known destructive output instead of pasting a partial transcript.
use color_eyre::eyre::{Result, bail};
use regex::Regex;
use std::collections::HashSet;
use std::sync::LazyLock;

static NUMBERS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\b\d+(?:[.,:]\d+)*\b").unwrap());
static CORRECTIONS: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\b(\d+(?:[.,:]\d+)*)\s*,?\s+no\s*,?\s+(\d+(?:[.,:]\d+)*)\b").unwrap()
});
static QUOTES: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#""([^"\n]+)"|“([^”\n]+)”"#).unwrap());
static TECHNICAL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\b[A-Za-z][\w]*_[\w]+\b|\b(?:https?://|www\.)[^\s]+|/[A-Za-z0-9_.-]+(?:/[A-Za-z0-9_.-]+)*").unwrap()
});

pub(super) fn validate(input: &str, output: &str) -> Result<()> {
    if output.trim().is_empty() {
        bail!("Local cleanup returned empty text");
    }
    for marker in [
        "<transcript>",
        "</transcript>",
        "Profile instructions:",
        "Foreground application:",
        "Browser host:",
        "Vocabulary (spell these exactly",
    ] {
        if output.contains(marker) && !input.contains(marker) {
            bail!("Local cleanup included prompt metadata");
        }
    }
    for c in QUOTES.captures_iter(input) {
        let quote = c.get(1).or_else(|| c.get(2)).unwrap().as_str();
        if !output.contains(quote) {
            bail!("Local cleanup changed quoted text");
        }
    }
    let remaining: HashSet<&str> = NUMBERS.find_iter(output).map(|m| m.as_str()).collect();
    let corrected: HashSet<&str> = CORRECTIONS
        .captures_iter(input)
        .filter_map(|c| {
            let whole = c.get(0).unwrap();
            let quoted = QUOTES
                .find_iter(input)
                .any(|q| q.start() <= whole.start() && q.end() >= whole.end());
            (!quoted && remaining.contains(c.get(2).unwrap().as_str()))
                .then(|| c.get(1).unwrap().as_str())
        })
        .collect();
    if NUMBERS
        .find_iter(input)
        .any(|n| !remaining.contains(n.as_str()) && !corrected.contains(n.as_str()))
    {
        bail!("Local cleanup removed a number");
    }
    for token in TECHNICAL.find_iter(input) {
        let token = token.as_str().trim_end_matches(['.', ',', ';', ')']);
        if !output.contains(token) {
            bail!("Local cleanup changed a technical token");
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_prompt_echo_and_lost_identifiers() {
        assert!(validate("Commit push bro.", "Browser host: none\nCommit push bro.").is_err());
        assert!(
            validate(
                "Use run_agent_v2 and case 91742.",
                "Use run_agent and case 91742."
            )
            .is_err()
        );
        assert!(validate("Use case 91742.", "Use case 91743.").is_err());
        assert!(validate("Keep \"do not change\".", "Keep do change.").is_err());
        assert!(validate("Hello", "").is_err());
    }
    #[test]
    fn accepts_explicit_numeric_correction_and_ordinary_edits() {
        assert!(validate("Use 12, no, 13 records.", "Use 13 records.").is_ok());
        assert!(validate("the report is ready", "The report is ready.").is_ok());
        assert!(validate("Keep \"12, no, 13\".", "Keep \"13\".").is_err());
    }
}
