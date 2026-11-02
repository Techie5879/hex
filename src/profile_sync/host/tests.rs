use super::*;
use serde_json::json;
use std::sync::atomic::{AtomicU64, Ordering};
fn temp() -> PathBuf {
    static ID: AtomicU64 = AtomicU64::new(0);
    let path = std::env::temp_dir().join(format!(
        "hex-profile-host-{}-{}",
        std::process::id(),
        ID.fetch_add(1, Ordering::Relaxed)
    ));
    private_directory(&path).unwrap();
    path
}
fn settings() -> AppSettings {
    let mut s = AppSettings::default();
    s.labs.profile_sync = true;
    s
}
fn get(path: &Path, s: &mut AppSettings) -> Reply {
    exchange(path, Exchange::Get, s, |_| Ok(())).unwrap()
}
fn put(path: &Path, s: &mut AppSettings, body: &Value, etag: Option<String>) -> Reply {
    exchange(
        path,
        Exchange::Put {
            text: encode(body),
            if_match: etag,
            if_none_match: None,
        },
        s,
        |_| Ok(()),
    )
    .unwrap()
}
#[test]
fn edits_are_visible_and_stale_phone_writes_cannot_overwrite_them() {
    let path = temp();
    let mut s = settings();
    let first = get(&path, &mut s);
    s.vocabulary
        .push(crate::dictation::vocabulary::VocabularyEntry {
            term: "Nimbus".into(),
            sounds_like: vec![],
        });
    assert_eq!(put(&path, &mut s, &empty(), first.etag).status, 412);
    let fresh = get(&path, &mut s);
    let p = decode(&fresh.body).unwrap();
    assert_eq!(adapter::vocabulary(&p)[0].term, "Nimbus");
    assert_eq!(s.vocabulary[0].term, "Nimbus");
    fs::remove_dir_all(path).unwrap();
}
#[test]
fn a_phone_edit_applies_only_profile_fields_and_requires_a_precondition() {
    let path = temp();
    let mut s = settings();
    let before = s.clone();
    let first = get(&path, &mut s);
    let mut p = decode(&first.body).unwrap();
    p["settings"]["cleanup.polish.prompt"] =
        json!({"value":"Keep every qualifier.","stamp":"0000000009000-0000-phone"});
    assert_eq!(put(&path, &mut s, &p, None).status, 428);
    assert_eq!(put(&path, &mut s, &p, first.etag).status, 204);
    assert_eq!(
        s.dictation_processing.default_mode.post_processing.prompt,
        "Keep every qualifier."
    );
    s.dictation_processing.default_mode.post_processing = before
        .dictation_processing
        .default_mode
        .post_processing
        .clone();
    assert_eq!(s, before);
    fs::remove_dir_all(path).unwrap();
}
#[test]
fn disabled_sync_never_reads_or_creates_profile_state() {
    let path = temp();
    let mut s = AppSettings::default();
    assert_eq!(get(&path, &mut s).status, 503);
    assert!(!path.join("state.json").exists());
    fs::remove_dir_all(path).unwrap();
}
#[test]
fn failed_apply_survives_restart_without_reverting_the_phone_edit() {
    let path = temp();
    let mut s = settings();
    let first = get(&path, &mut s);
    let mut p = decode(&first.body).unwrap();
    p["settings"]["cleanup.polish.prompt"] =
        json!({"value":"Phone edit","stamp":"0000000009000-0000-phone"});
    let failure = exchange(
        &path,
        Exchange::Put {
            text: encode(&p),
            if_match: first.etag,
            if_none_match: None,
        },
        &mut s,
        |_| Err(eyre!("disk unavailable")),
    );
    assert!(failure.is_err());
    get(&path, &mut s);
    assert_eq!(
        s.dictation_processing.default_mode.post_processing.prompt,
        "Phone edit"
    );
    fs::remove_dir_all(path).unwrap();
}
#[test]
fn device_tokens_are_hashed_and_revocation_is_immediate() {
    let path = temp();
    let token = path.join("pairing.txt");
    add_device(&path, "phone", &token).unwrap();
    let secret = fs::read_to_string(&token).unwrap();
    assert!(authorized(&path, &secret));
    assert!(!authorized(&path, "wrong"));
    assert!(
        !fs::read_to_string(path.join("devices.json"))
            .unwrap()
            .contains(&secret)
    );
    revoke(&path, "phone").unwrap();
    assert!(!authorized(&path, &secret));
    fs::remove_dir_all(path).unwrap();
}
#[test]
fn oversized_local_growth_does_not_corrupt_the_last_valid_profile() {
    let path = temp();
    let mut s = settings();
    let first = get(&path, &mut s);
    let mut p = decode(&first.body).unwrap();
    p["future"] = Value::String("x".repeat(super::super::MAX_BYTES - encode(&p).len() - 16));
    assert_eq!(put(&path, &mut s, &p, first.etag).status, 204);
    let saved = fs::read(path.join("state.json")).unwrap();
    s.vocabulary
        .push(crate::dictation::vocabulary::VocabularyEntry {
            term: "Additional vocabulary".into(),
            sounds_like: vec![],
        });
    assert!(exchange(&path, Exchange::Get, &mut s, |_| Ok(())).is_err());
    assert_eq!(fs::read(path.join("state.json")).unwrap(), saved);
    s.vocabulary.clear();
    assert_eq!(get(&path, &mut s).status, 200);
    fs::remove_dir_all(path).unwrap();
}

#[test]
fn migration_keeps_pairing_and_applies_imported_profile_before_absorbing_defaults() {
    let path = temp();
    let old = temp();
    let mut p = empty();
    p["settings"]["cleanup.polish.prompt"] =
        json!({"value":"Imported words","stamp":"0000000009000-0000-phone"});
    fs::write(old.join("profile.json"), encode(&p)).unwrap();
    fs::write(
        old.join("tokens"),
        format!("{} phone\n", hash(b"migration-secret")),
    )
    .unwrap();
    import_companion(&path, &old).unwrap();
    // Simulate a crash after writing the state but before pairing records.
    fs::remove_file(path.join("devices.json")).unwrap();
    import_companion(&path, &old).unwrap();
    assert!(authorized(&path, "migration-secret"));
    let mut s = settings();
    get(&path, &mut s);
    assert_eq!(
        s.dictation_processing.default_mode.post_processing.prompt,
        "Imported words"
    );
    assert!(import_companion(&path, &old).is_err());
    assert!(old.join("profile.json").exists());
    fs::remove_dir_all(path).unwrap();
    fs::remove_dir_all(old).unwrap();
}
