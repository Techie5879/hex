//! The portable profile shared with Hex on Android: dictionary, cleanup prompts, and
//! cleanup policy, versioned and merged per field. Off by default (Labs › Profile sync);
//! the running Mac app hosts paired devices only while enabled. No audio or history.
//!
//! `merge` mirrors `android/core`'s `ProfileMerge`; both run the vectors in
//! `tests/fixtures/profile-sync`, so either device reaches the same document.
pub(crate) mod adapter;
pub(crate) mod cli;
pub(crate) mod host;
pub(crate) mod listener;
pub(crate) mod transport;

use serde_json::{Map, Value, json};
use unicode_normalization::UnicodeNormalization;

pub const SCHEMA: &str = "hex.profile";
pub const VERSION: i64 = 1;
pub const MAX_BYTES: usize = 1024 * 1024;
const MAX_SOUNDS_LIKE: usize = 16;

/// A term's identity: Unicode-normalised, case-folded, and space-collapsed.
pub fn term_key(term: &str) -> String {
    term.nfc()
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

pub fn empty() -> Value {
    json!({"schema": SCHEMA, "version": VERSION, "vocabulary": {}, "settings": {}, "conflicts": {}})
}

/// Parses a stored or fetched profile, refusing anything this build can't merge safely.
pub fn decode(text: &str) -> Result<Value, String> {
    if text.len() > MAX_BYTES {
        return Err("Profile is larger than 1 MiB".into());
    }
    let value: Value = serde_json::from_str(text).map_err(|_| "Profile is not JSON".to_string())?;
    if value.get("schema").and_then(Value::as_str) != Some(SCHEMA) {
        return Err("Not a Hex profile".into());
    }
    match value.get("version").and_then(Value::as_i64) {
        Some(version) if version <= VERSION => {}
        Some(version) => return Err(format!("Profile version {version} needs a newer Hex")),
        None => return Err("Profile has no version".into()),
    }
    for section in ["vocabulary", "settings", "conflicts"] {
        if value
            .get(section)
            .is_some_and(|section| !section.is_object())
        {
            return Err(format!("Profile {section} is malformed"));
        }
    }
    Ok(value)
}

/// Compact, with every object's keys in order, so equal documents encode equally.
pub fn encode(value: &Value) -> String {
    fn sorted(value: &Value) -> Value {
        match value {
            Value::Object(map) => {
                let mut keys: Vec<_> = map.keys().collect();
                keys.sort();
                Value::Object(
                    keys.into_iter()
                        .map(|k| (k.clone(), sorted(&map[k])))
                        .collect(),
                )
            }
            Value::Array(items) => Value::Array(items.iter().map(sorted).collect()),
            other => other.clone(),
        }
    }
    sorted(value).to_string()
}

pub(crate) fn section<'a>(profile: &'a Value, name: &str) -> &'a Map<String, Value> {
    static EMPTY: std::sync::OnceLock<Map<String, Value>> = std::sync::OnceLock::new();
    profile
        .get(name)
        .and_then(Value::as_object)
        .unwrap_or_else(|| EMPTY.get_or_init(Map::new))
}

fn stamp(value: &Value) -> &str {
    value.get("stamp").and_then(Value::as_str).unwrap_or("")
}

pub(crate) fn is_deleted(value: &Value) -> bool {
    value
        .get("deleted")
        .and_then(Value::as_bool)
        .unwrap_or(false)
}

/// An unchanged side takes the other. Otherwise `both_changed` decides.
fn three_way(
    local: Option<&Value>,
    remote: Option<&Value>,
    ancestor: Option<&Value>,
    both_changed: impl FnOnce(&Value, &Value) -> Value,
) -> Option<Value> {
    match (local, remote) {
        (None, None) => None,
        (None, Some(value)) | (Some(value), None) => Some(value.clone()),
        (Some(l), Some(r)) if l == r => Some(l.clone()),
        (Some(l), Some(r)) if Some(l) == ancestor => Some(r.clone()),
        (Some(l), Some(r)) if Some(r) == ancestor => Some(l.clone()),
        (Some(l), Some(r)) => Some(both_changed(l, r)),
    }
}

fn newest<'a>(l: &'a Value, r: &'a Value) -> (&'a Value, &'a Value) {
    if stamp(l) >= stamp(r) { (l, r) } else { (r, l) }
}

pub(crate) fn spellings(entry: &Value) -> Vec<String> {
    entry
        .get("sounds_like")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

fn merge_term(l: &Value, r: &Value) -> Value {
    let (winner, loser) = newest(l, r);
    if is_deleted(winner) || is_deleted(loser) {
        return winner.clone();
    }
    let mut union: Vec<String> = Vec::new();
    for spelling in spellings(winner).into_iter().chain(spellings(loser)) {
        if !union.contains(&spelling) && union.len() < MAX_SOUNDS_LIKE {
            union.push(spelling);
        }
    }
    let mut merged = winner.clone();
    if !union.is_empty() {
        merged["sounds_like"] = json!(union);
    }
    merged
}

fn merge_conflict(local: Option<&Value>, remote: Option<&Value>) -> Value {
    let first = local.or(remote).expect("a conflict exists on one side");
    let second = remote.or(local).expect("a conflict exists on one side");
    let resolved = [first, second]
        .into_iter()
        .filter_map(|conflict| conflict.get("resolved_stamp").and_then(Value::as_str))
        .max();
    let mut merged = first.clone();
    if let Some(resolved) = resolved {
        merged["value"] = Value::Null;
        merged["resolved_stamp"] = json!(resolved);
    }
    merged
}

/// Three-way merge against the last document this device agreed with the server.
/// Concurrent spelling edits union; deletes win or lose by stamp; a concurrent
/// settings edit keeps the newest and records the other as an open conflict.
pub fn merge(local: &Value, remote: &Value, ancestor: &Value) -> Value {
    let keys = |name: &str| -> Vec<String> {
        let mut keys: Vec<String> = section(local, name)
            .keys()
            .chain(section(remote, name).keys())
            .cloned()
            .collect();
        keys.sort();
        keys.dedup();
        keys
    };

    let mut conflicts = Map::new();
    for id in keys("conflicts") {
        let merged = merge_conflict(
            section(local, "conflicts").get(&id),
            section(remote, "conflicts").get(&id),
        );
        conflicts.insert(id, merged);
    }

    let mut vocabulary = Map::new();
    for key in keys("vocabulary") {
        let merged = three_way(
            section(local, "vocabulary").get(&key),
            section(remote, "vocabulary").get(&key),
            section(ancestor, "vocabulary").get(&key),
            merge_term,
        );
        if let Some(merged) = merged {
            vocabulary.insert(key, merged);
        }
    }

    let mut settings = Map::new();
    for key in keys("settings") {
        let merged = three_way(
            section(local, "settings").get(&key),
            section(remote, "settings").get(&key),
            section(ancestor, "settings").get(&key),
            |l, r| {
                let (winner, loser) = newest(l, r);
                if l.get("value") != r.get("value") {
                    let id = format!("{key}#{}", stamp(loser));
                    conflicts.entry(id).or_insert_with(|| {
                        json!({
                            "key": key,
                            "value": loser.get("value").cloned().unwrap_or(Value::Null),
                            "stamp": stamp(loser),
                            "winner_stamp": stamp(winner),
                        })
                    });
                }
                winner.clone()
            },
        );
        if let Some(merged) = merged {
            settings.insert(key, merged);
        }
    }

    // Unknown top-level fields survive; a newer build may have written them.
    let mut result = remote.as_object().cloned().unwrap_or_default();
    if let Some(local) = local.as_object() {
        result.extend(local.iter().map(|(k, v)| (k.clone(), v.clone())));
    }
    result.insert("schema".into(), json!(SCHEMA));
    result.insert("version".into(), json!(VERSION));
    result.insert("vocabulary".into(), Value::Object(vocabulary));
    result.insert("settings".into(), Value::Object(settings));
    result.insert("conflicts".into(), Value::Object(conflicts));
    Value::Object(result)
}

/// A later stamp than anything in `profile`: `millis-counter-device`, ordered as text.
pub fn next_stamp(profile: &Value, device: &str, now_ms: u64) -> String {
    let highest = section(profile, "vocabulary")
        .values()
        .chain(section(profile, "settings").values())
        .map(stamp)
        .max()
        .unwrap_or("");
    let mut parts = highest.splitn(3, '-');
    let seen_ms: u64 = parts.next().and_then(|p| p.parse().ok()).unwrap_or(0);
    let seen_counter: u32 = parts.next().and_then(|p| p.parse().ok()).unwrap_or(0);
    let ms = now_ms.max(seen_ms);
    let counter = if ms == seen_ms { seen_counter + 1 } else { 0 };
    format!("{ms:013}-{counter:04}-{device}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(name: &str) -> Value {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/profile-sync")
            .join(name);
        serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
    }

    #[test]
    fn shared_merge_vectors() {
        let vectors = fixture("vectors.json");
        let cases = vectors["cases"].as_array().unwrap();
        assert!(cases.len() >= 10);
        for case in cases {
            let name = case["name"].as_str().unwrap();
            let expected = encode(&case["expected"]);
            let forward = merge(&case["local"], &case["remote"], &case["ancestor"]);
            assert_eq!(encode(&forward), expected, "{name}");
            let swapped = merge(&case["remote"], &case["local"], &case["ancestor"]);
            assert_eq!(encode(&swapped), expected, "{name} (swapped)");
        }
    }

    #[test]
    fn shared_term_keys() {
        for pair in fixture("term_keys.json")["terms"].as_array().unwrap() {
            assert_eq!(
                term_key(pair[0].as_str().unwrap()),
                pair[1].as_str().unwrap()
            );
        }
    }

    #[test]
    fn refuses_newer_versions_and_other_documents() {
        assert!(
            decode(r#"{"schema":"hex.profile","version":2}"#)
                .unwrap_err()
                .contains("newer")
        );
        assert!(decode(r#"{"schema":"other","version":1}"#).is_err());
        assert!(decode("not json").is_err());
        assert!(decode(r#"{"schema":"hex.profile","version":1,"settings":[]}"#).is_err());
    }

    #[test]
    fn stamps_always_move_forward() {
        let mut profile = empty();
        profile["settings"]["k"] = json!({"value": 1, "stamp": "0000000005000-0003-mac"});
        assert_eq!(next_stamp(&profile, "mac", 1000), "0000000005000-0004-mac");
        assert_eq!(next_stamp(&profile, "mac", 6000), "0000000006000-0000-mac");
        assert_eq!(next_stamp(&empty(), "mac", 1), "0000000000001-0000-mac");
    }
}
