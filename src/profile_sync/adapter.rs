//! Maps desktop settings to the portable profile and back, with no I/O. Only the
//! dictionary and the Global mode's cleanup fields sync; every other setting, and
//! every other field of that mode, is left exactly as it is.
use serde_json::{Value, json};

use super::{is_deleted, next_stamp, section, spellings, term_key};
use crate::app_settings::DictationPostProcessing;
use crate::dictation::vocabulary::VocabularyEntry;

/// Portable settings key, and its field in `post_processing`.
const FIELDS: [(&str, &str); 8] = [
    ("cleanup.polish.enabled", "enabled"),
    ("cleanup.polish.prompt", "prompt"),
    ("cleanup.polish.model", "model"),
    ("cleanup.polish.variant", "variant"),
    ("cleanup.polish.deadline_seconds", "deadline_seconds"),
    (
        "cleanup.polish.quota_fallback_provider",
        "quota_fallback_provider",
    ),
    ("cleanup.polish.min_words", "min_words"),
    ("cleanup.polish.min_chars", "min_chars"),
];

/// Whether a stored record says this entry exists exactly as written here.
fn holds(record: Option<&Value>, entry: &VocabularyEntry) -> bool {
    record.is_some_and(|record| {
        !is_deleted(record)
            && record.get("term").and_then(Value::as_str) == Some(entry.term.as_str())
            && spellings(record) == entry.sounds_like
    })
}

/// Records what a person changed on this computer as new stamped edits. `applied` is the
/// document this computer last fully applied, so an edit is a difference from that. A
/// remote change that a failed sync didn't finish applying isn't one: it keeps its place in
/// `profile` and the next apply finishes it, instead of being reversed here.
pub fn absorb(
    profile: &mut Value,
    applied: &Value,
    vocabulary: &[VocabularyEntry],
    post: &DictationPostProcessing,
    device: &str,
    now_ms: u64,
) {
    let post = serde_json::to_value(post).expect("post-processing settings serialize");
    for (key, field) in FIELDS {
        let value = post.get(field).cloned().unwrap_or(Value::Null);
        let baseline = section(applied, "settings")
            .get(key)
            .and_then(|r| r.get("value"));
        let current = section(profile, "settings")
            .get(key)
            .and_then(|r| r.get("value"));
        let unchanged = baseline == Some(&value) || (value.is_null() && baseline.is_none());
        if unchanged || current == Some(&value) || (value.is_null() && current.is_none()) {
            continue;
        }
        let stamp = next_stamp(profile, device, now_ms);
        profile["settings"][key] = json!({"value": value, "stamp": stamp});
    }
    let mut seen = Vec::with_capacity(vocabulary.len());
    for entry in vocabulary {
        let key = term_key(&entry.term);
        let changed = !holds(section(applied, "vocabulary").get(&key), entry)
            && !holds(section(profile, "vocabulary").get(&key), entry);
        if changed {
            let stamp = next_stamp(profile, device, now_ms);
            let mut record = json!({"term": entry.term, "stamp": stamp});
            if !entry.sounds_like.is_empty() {
                record["sounds_like"] = json!(entry.sounds_like);
            }
            profile["vocabulary"][&key] = record;
        }
        seen.push(key);
    }
    // Removed here: it was applied and is now gone. A term only in `profile` is new, not removed.
    let removed: Vec<String> = section(applied, "vocabulary")
        .iter()
        .filter(|(key, record)| !is_deleted(record) && !seen.contains(*key))
        .filter(|(key, _)| {
            section(profile, "vocabulary")
                .get(*key)
                .is_some_and(|record| !is_deleted(record))
        })
        .map(|(key, _)| key.clone())
        .collect();
    for key in removed {
        let stamp = next_stamp(profile, device, now_ms);
        let term = profile["vocabulary"][&key]["term"].clone();
        profile["vocabulary"][&key] = json!({"term": term, "stamp": stamp, "deleted": true});
    }
}

/// The dictionary a profile describes, sorted as the window lists it.
pub fn vocabulary(profile: &Value) -> Vec<VocabularyEntry> {
    let mut entries: Vec<VocabularyEntry> = section(profile, "vocabulary")
        .values()
        .filter(|e| !e.get("deleted").and_then(Value::as_bool).unwrap_or(false))
        .filter_map(|e| {
            Some(VocabularyEntry {
                term: e.get("term")?.as_str()?.to_owned(),
                sounds_like: e
                    .get("sounds_like")
                    .and_then(Value::as_array)
                    .map(|items| {
                        items
                            .iter()
                            .filter_map(Value::as_str)
                            .map(str::to_owned)
                            .collect()
                    })
                    .unwrap_or_default(),
            })
        })
        .collect();
    entries.sort_by_key(|entry| term_key(&entry.term));
    entries
}

/// `post` with the profile's synced fields written over it. A value of the wrong type
/// keeps what's there. Fields the profile doesn't name are untouched.
pub fn apply(profile: &Value, post: &DictationPostProcessing) -> DictationPostProcessing {
    let mut object = serde_json::to_value(post).expect("post-processing settings serialize");
    for (key, field) in FIELDS {
        if let Some(value) = section(profile, "settings")
            .get(key)
            .and_then(|r| r.get("value"))
        {
            let mut candidate = object.clone();
            candidate[field] = value.clone();
            if serde_json::from_value::<DictationPostProcessing>(candidate).is_ok() {
                object[field] = value.clone();
            }
        }
    }
    serde_json::from_value(object).unwrap_or_else(|_| post.clone())
}

/// The vocabulary commands that take `current` to `wanted`.
pub fn vocabulary_changes(
    current: &[VocabularyEntry],
    wanted: &[VocabularyEntry],
) -> (Vec<String>, Vec<VocabularyEntry>) {
    let removed = current
        .iter()
        .filter(|c| {
            !wanted
                .iter()
                .any(|w| term_key(&w.term) == term_key(&c.term))
        })
        .map(|c| c.term.clone())
        .collect();
    let added = wanted
        .iter()
        .filter(|w| {
            !current
                .iter()
                .any(|c| c.term == w.term && c.sounds_like == w.sounds_like)
        })
        .cloned()
        .collect();
    (removed, added)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::profile_sync::empty;

    fn term(term: &str, spellings: &[&str]) -> VocabularyEntry {
        VocabularyEntry {
            term: term.into(),
            sounds_like: spellings.iter().map(|s| (*s).into()).collect(),
        }
    }

    fn post() -> DictationPostProcessing {
        DictationPostProcessing {
            enabled: true,
            prompt: "Keep every qualifier.".into(),
            model: Some("opencode-go/gpt-6-luna".into()),
            variant: Some("none".into()),
            deadline_seconds: 8,
            quota_fallback_provider: Some("openai".into()),
            min_words: 150,
            min_chars: 3000,
        }
    }

    #[test]
    fn a_full_prompt_and_policy_round_trip_unchanged() {
        let long = "keep this sentence. ".repeat(500);
        let settings = DictationPostProcessing {
            prompt: long.clone(),
            ..post()
        };
        let mut profile = empty();
        absorb(
            &mut profile,
            &empty(),
            &[term("Nimbus", &["nimbas"])],
            &settings,
            "mac",
            1000,
        );
        let restored = apply(&profile, &DictationPostProcessing::default());
        assert_eq!(restored, settings);
        assert_eq!(restored.prompt.len(), long.len());
        assert_eq!(vocabulary(&profile), vec![term("Nimbus", &["nimbas"])]);
    }

    #[test]
    fn absorbing_nothing_new_stamps_nothing() {
        let mut profile = empty();
        absorb(
            &mut profile,
            &empty(),
            &[term("Nimbus", &[])],
            &post(),
            "mac",
            1000,
        );
        let before = profile.clone();
        absorb(
            &mut profile,
            &before.clone(),
            &[term("Nimbus", &[])],
            &post(),
            "mac",
            9000,
        );
        assert_eq!(profile, before);
    }

    #[test]
    fn a_term_removed_on_this_computer_becomes_a_tombstone() {
        let mut profile = empty();
        absorb(
            &mut profile,
            &empty(),
            &[term("Nimbus", &[]), term("Quartz", &[])],
            &post(),
            "mac",
            1000,
        );
        let applied = profile.clone();
        absorb(
            &mut profile,
            &applied,
            &[term("Nimbus", &[])],
            &post(),
            "mac",
            2000,
        );
        assert_eq!(profile["vocabulary"]["quartz"]["deleted"], json!(true));
        assert_eq!(vocabulary(&profile), vec![term("Nimbus", &[])]);
    }

    /// A sync merged a remote edit, then failed before finishing applying it. The next sync
    /// must finish it, not treat this computer's stale values as new edits that reverse it.
    #[test]
    fn a_partly_applied_sync_finishes_rather_than_reverses() {
        let settings = post();
        let mut applied = empty();
        absorb(
            &mut applied,
            &empty(),
            &[term("Nimbus", &[]), term("Quartz", &[])],
            &settings,
            "mac",
            1000,
        );

        let mut merged = applied.clone();
        merged["settings"]["cleanup.polish.prompt"] =
            json!({"value": "phone wording", "stamp": "0000000005000-0000-phone"});
        merged["vocabulary"]["quartz"] =
            json!({"term": "Quartz", "stamp": "0000000005000-0000-phone", "deleted": true});
        merged["vocabulary"]["zephyr"] =
            json!({"term": "Zephyr", "stamp": "0000000005000-0000-phone"});

        // Nothing was applied: this computer still has the old prompt, Quartz, and no Zephyr.
        let mut next = merged.clone();
        absorb(
            &mut next,
            &applied,
            &[term("Nimbus", &[]), term("Quartz", &[])],
            &settings,
            "mac",
            9000,
        );
        assert_eq!(
            next, merged,
            "unapplied remote changes aren't this computer's edits"
        );
        assert_eq!(apply(&next, &settings).prompt, "phone wording");
        assert_eq!(
            vocabulary(&next),
            vec![term("Nimbus", &[]), term("Zephyr", &[])]
        );

        // Half applied: Zephyr arrived, Quartz is still here. Still no edits to record.
        let mut next = merged.clone();
        absorb(
            &mut next,
            &applied,
            &[
                term("Nimbus", &[]),
                term("Quartz", &[]),
                term("Zephyr", &[]),
            ],
            &settings,
            "mac",
            9000,
        );
        assert_eq!(next, merged);

        // And an actual edit made in between still counts.
        let edited = DictationPostProcessing {
            min_words: 99,
            ..settings.clone()
        };
        let mut next = merged.clone();
        absorb(
            &mut next,
            &applied,
            &[term("Nimbus", &[]), term("Quartz", &[])],
            &edited,
            "mac",
            9000,
        );
        assert_eq!(
            next["settings"]["cleanup.polish.min_words"]["value"],
            json!(99)
        );
        assert_eq!(
            next["settings"]["cleanup.polish.prompt"],
            merged["settings"]["cleanup.polish.prompt"]
        );
    }

    #[test]
    fn settings_outside_the_profile_stay_as_they_are() {
        let mut profile = empty();
        profile["settings"]["cleanup.polish.min_words"] = json!({"value": 200, "stamp": "1"});
        profile["settings"]["future.key"] = json!({"value": "x", "stamp": "1"});
        let restored = apply(&profile, &post());
        assert_eq!(restored.min_words, 200);
        assert_eq!(restored.prompt, post().prompt);
    }

    #[test]
    fn a_wrongly_typed_value_keeps_the_setting() {
        let mut profile = empty();
        profile["settings"]["cleanup.polish.min_words"] = json!({"value": "many", "stamp": "1"});
        assert_eq!(apply(&profile, &post()).min_words, 150);
    }

    #[test]
    fn vocabulary_changes_are_minimal() {
        let current = [term("Nimbus", &[]), term("Quartz", &[])];
        let wanted = [term("Nimbus", &["nimbas"]), term("Zephyr", &[])];
        let (removed, added) = vocabulary_changes(&current, &wanted);
        assert_eq!(removed, vec!["Quartz".to_string()]);
        assert_eq!(
            added,
            vec![term("Nimbus", &["nimbas"]), term("Zephyr", &[])]
        );
        assert_eq!(vocabulary_changes(&wanted, &wanted), (vec![], vec![]));
    }
}

/// Rebase an open editor's actual profile changes onto newly synced settings.
/// Unrelated UI saves must not write its stale dictionary or prompt back.
pub(crate) fn rebase_editor(
    before: &crate::app_settings::AppSettings,
    candidate: &mut crate::app_settings::AppSettings,
    current: &crate::app_settings::AppSettings,
) {
    let mut baseline = super::empty();
    absorb(
        &mut baseline,
        &super::empty(),
        &before.vocabulary,
        &before.dictation_processing.default_mode.post_processing,
        "baseline",
        1,
    );
    let mut edited = baseline.clone();
    absorb(
        &mut edited,
        &baseline,
        &candidate.vocabulary,
        &candidate.dictation_processing.default_mode.post_processing,
        "editor",
        3,
    );
    let mut remote = baseline.clone();
    absorb(
        &mut remote,
        &baseline,
        &current.vocabulary,
        &current.dictation_processing.default_mode.post_processing,
        "current",
        2,
    );
    let rebased = super::merge(&edited, &remote, &baseline);
    candidate.vocabulary = vocabulary(&rebased);
    candidate.dictation_processing.default_mode.post_processing = apply(
        &rebased,
        &candidate.dictation_processing.default_mode.post_processing,
    );
}

#[cfg(test)]
mod editor_tests {
    use super::*;
    #[test]
    fn stale_editor_preserves_remote_edits_and_applies_only_its_own_changes() {
        let before = crate::app_settings::AppSettings::default();
        let mut current = before.clone();
        current
            .dictation_processing
            .default_mode
            .post_processing
            .prompt = "Phone wording".into();
        current.vocabulary.push(VocabularyEntry {
            term: "Nimbus".into(),
            sounds_like: vec![],
        });
        let mut candidate = before.clone();
        candidate.show_dock_icon = !candidate.show_dock_icon;
        rebase_editor(&before, &mut candidate, &current);
        assert_eq!(
            candidate
                .dictation_processing
                .default_mode
                .post_processing
                .prompt,
            "Phone wording"
        );
        assert_eq!(candidate.vocabulary, current.vocabulary);
        let mut edited = before.clone();
        edited
            .dictation_processing
            .default_mode
            .post_processing
            .min_words = 250;
        rebase_editor(&before, &mut edited, &current);
        assert_eq!(
            edited
                .dictation_processing
                .default_mode
                .post_processing
                .prompt,
            "Phone wording"
        );
        assert_eq!(
            edited
                .dictation_processing
                .default_mode
                .post_processing
                .min_words,
            250
        );
    }
}
