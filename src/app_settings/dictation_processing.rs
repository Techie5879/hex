//! What happens to a dictation's text before it pastes: Global's steps and
//! the modes chosen by application or website.

use serde::{Deserialize, Serialize};

use super::{DictationPostProcessing, TextReplacement};
use crate::dictation::vocabulary::{VocabularyEntry, serialize_saved};

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(default)]
pub struct DictationProcessingSettings {
    pub default_mode: DictationMode,
    pub modes: Vec<DictationMode>,
}

impl Default for DictationProcessingSettings {
    fn default() -> Self {
        Self {
            default_mode: DictationMode {
                name: "Global".into(),
                ..Default::default()
            },
            modes: Vec::new(),
        }
    }
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
#[serde(default)]
pub struct DictationMode {
    pub name: String,
    pub applications: Vec<String>,
    pub browser_hosts: Vec<String>,
    pub replacements: Vec<TextReplacement>,
    pub transformations: Vec<String>,
    pub post_processing: DictationPostProcessing,
    /// Names heard and spelled here ahead of the shared vocabulary.
    #[serde(
        skip_serializing_if = "Vec::is_empty",
        serialize_with = "serialize_saved"
    )]
    pub vocabulary: Vec<VocabularyEntry>,
}
