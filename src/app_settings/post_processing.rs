use serde::{Deserialize, Serialize};

/// Persisted OpenCode rewrite settings for one mode.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default)]
pub struct DictationPostProcessing {
    pub enabled: bool,
    pub prompt: String,
    /// Optional `provider/model` key from the OpenCode catalog.
    pub model: Option<String>,
    pub variant: Option<String>,
    pub deadline_seconds: u64,
    /// Same model and variant, retried only for typed provider rate/quota limits.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quota_fallback_provider: Option<String>,
    /// Run after local cleanup when either positive size threshold is met.
    /// Both zero preserves the default: rewrite every enabled dictation.
    pub min_words: usize,
    pub min_chars: usize,
}

impl Default for DictationPostProcessing {
    fn default() -> Self {
        Self {
            enabled: false,
            prompt: "Rewrite the transcript into clear, natural text while preserving the speaker's intended meaning.".into(),
            model: None,
            variant: None,
            deadline_seconds: 30,
            quota_fallback_provider: None,
            min_words: 0,
            min_chars: 0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fork_cleanup_settings_survive_json_round_trip() {
        let settings: DictationPostProcessing = serde_json::from_value(serde_json::json!({
            "enabled": true,
            "prompt": "Preserve every qualification.",
            "model": "opencode-go/gpt-6-luna",
            "variant": "none",
            "deadline_seconds": 8,
            "quota_fallback_provider": "openai",
            "min_words": 150,
            "min_chars": 3000
        }))
        .unwrap();
        let saved = serde_json::to_value(&settings).unwrap();
        let loaded: DictationPostProcessing = serde_json::from_value(saved).unwrap();
        assert_eq!(loaded, settings);
        assert_eq!(loaded.variant.as_deref(), Some("none"));
        assert_eq!((loaded.min_words, loaded.min_chars), (150, 3000));
        assert_eq!(loaded.quota_fallback_provider.as_deref(), Some("openai"));
    }
}
