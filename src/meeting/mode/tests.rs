use super::*;
use std::fs;
use std::path::PathBuf;

struct Fixture {
    directory: PathBuf,
    manifest: MeetingManifest,
    owner: Owner,
}

impl Fixture {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        let id = format!(
            "mode-test-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        );
        let directory = std::env::temp_dir().join(&id);
        fs::create_dir_all(&directory).unwrap();
        let manifest: MeetingManifest = serde_json::from_value(serde_json::json!({
            "schema_version": 1, "id": id, "title": "Synthetic planning",
            "status": "recording", "created_at_ms": 1_760_000_000_000_u64,
            "ended_at_ms": null, "duration_ms": null,
            "system": {"file": "system.wav", "samples": 0, "dropped_packets": 0},
            "microphone": {"file": "microphone.wav", "samples": 0, "dropped_packets": 0},
            "error": null
        }))
        .unwrap();
        let (_, requests) = mpsc::sync_channel(1);
        let owner = Owner {
            id: manifest.id.clone(),
            requests,
            pending: None,
        };
        write_manifest(&directory, &manifest).unwrap();
        Self {
            directory,
            manifest,
            owner,
        }
    }

    fn change(&self, mode: MeetingMode) -> Change {
        Change::new(self.manifest.id.clone(), mode)
    }

    fn saved(&self) -> MeetingManifest {
        serde_json::from_slice(&fs::read(self.directory.join("manifest.json")).unwrap()).unwrap()
    }

    fn apply(&mut self, mode: MeetingMode) -> Change {
        let change = self.change(mode);
        self.owner
            .begin(&self.directory, &mut self.manifest, change.clone(), |_| {
                let (reply, updated) = mpsc::sync_channel(1);
                reply.send(Ok(())).unwrap();
                Ok(updated)
            })
            .unwrap();
        self.owner
            .finish(&self.directory, &mut self.manifest)
            .unwrap();
        change
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.directory);
    }
}

#[test]
fn an_old_global_true_is_ignored_and_unrelated_labs_survive() {
    let old: crate::app_settings::LabsSettings = serde_json::from_value(serde_json::json!({
        "shared_room_meetings": true, "meeting_speaker_names": true, "stop_when_call_ends": true
    }))
    .unwrap();
    assert!(old.meeting_speaker_names && old.stop_when_call_ends);
    let saved = serde_json::to_value(old).unwrap();
    assert!(saved.get("shared_room_meetings").is_none());
    assert!(crate::labs::Feature::for_settings_key("labs.shared_room_meetings").is_none());
    assert_eq!(MeetingMode::default(), MeetingMode::Standard);
    assert_eq!(Fixture::new().manifest.mode, MeetingMode::Standard);
}

#[test]
fn toggle_both_ways_persists_only_this_recording_and_survives_recovery() {
    let mut fixture = Fixture::new();
    let other = Fixture::new();
    assert_eq!(fixture.manifest.mode, MeetingMode::Standard);
    let on = fixture.apply(MeetingMode::SharedRoom);
    assert_eq!(on.result(), Some(Ok(())));
    assert_eq!(fixture.saved().mode, MeetingMode::SharedRoom);
    // A restart sees the saved choice without any global setting.
    let recovered = fixture.saved();
    assert_eq!(recovered.mode, MeetingMode::SharedRoom);
    assert_eq!(other.saved().mode, MeetingMode::Standard);
    let off = fixture.apply(MeetingMode::Standard);
    assert_eq!(off.result(), Some(Ok(())));
    assert_eq!(fixture.saved().mode, MeetingMode::Standard);
    assert!(
        !fs::read_to_string(fixture.directory.join("manifest.json"))
            .unwrap()
            .contains("\"mode\"")
    );
    assert_eq!(Fixture::new().manifest.mode, MeetingMode::Standard);
}

#[test]
fn the_switch_stays_pending_until_the_live_worker_acknowledges() {
    let mut fixture = Fixture::new();
    let change = fixture.change(MeetingMode::SharedRoom);
    let (reply, updated) = mpsc::sync_channel(1);
    fixture
        .owner
        .begin(
            &fixture.directory,
            &mut fixture.manifest,
            change.clone(),
            |_| Ok(updated),
        )
        .unwrap();
    assert_eq!(fixture.saved().mode, MeetingMode::SharedRoom);
    fixture
        .owner
        .settle(&fixture.directory, &mut fixture.manifest, false)
        .unwrap();
    assert_eq!(change.result(), None);
    reply.send(Ok(())).unwrap();
    fixture
        .owner
        .finish(&fixture.directory, &mut fixture.manifest)
        .unwrap();
    assert_eq!(change.result(), Some(Ok(())));
}

#[test]
fn a_failed_save_does_not_change_memory_or_contact_the_worker() {
    let mut fixture = Fixture::new();
    let change = fixture.change(MeetingMode::SharedRoom);
    fs::create_dir(fixture.directory.join("manifest.json.tmp")).unwrap();
    fixture
        .owner
        .begin(
            &fixture.directory,
            &mut fixture.manifest,
            change.clone(),
            |_| panic!("a failed save must not reach the live worker"),
        )
        .unwrap();
    assert!(change.result().unwrap().is_err());
    assert_eq!(fixture.manifest.mode, MeetingMode::Standard);
    assert_eq!(fixture.saved().mode, MeetingMode::Standard);
}

#[test]
fn a_full_or_failed_live_worker_restores_the_durable_choice() {
    for enqueue_failure in [true, false] {
        let mut fixture = Fixture::new();
        let change = fixture.change(MeetingMode::SharedRoom);
        fixture
            .owner
            .begin(
                &fixture.directory,
                &mut fixture.manifest,
                change.clone(),
                |_| {
                    if enqueue_failure {
                        return Err(eyre!("queue full"));
                    }
                    let (reply, updated) = mpsc::sync_channel(1);
                    reply.send(Err("snapshot write failed".into())).unwrap();
                    Ok(updated)
                },
            )
            .unwrap();
        fixture
            .owner
            .finish(&fixture.directory, &mut fixture.manifest)
            .unwrap();
        assert!(change.result().unwrap().is_err());
        assert_eq!(fixture.saved().mode, MeetingMode::Standard);
        assert_eq!(fixture.manifest.mode, MeetingMode::Standard);
    }
}

#[test]
fn a_wrong_id_or_finished_recording_never_writes() {
    for wrong_id in [true, false] {
        let mut fixture = Fixture::new();
        let mut change = fixture.change(MeetingMode::SharedRoom);
        if wrong_id {
            change.id = "the-previous-meeting".into();
        } else {
            fixture.manifest.status = MeetingStatus::Transcribing;
        }
        fixture
            .owner
            .begin(
                &fixture.directory,
                &mut fixture.manifest,
                change.clone(),
                |_| panic!("stale or finished changes must not reach the live worker"),
            )
            .unwrap();
        assert!(change.result().unwrap().is_err());
        assert_eq!(fixture.saved().mode, MeetingMode::Standard);
    }
}

#[test]
fn closing_rejects_queued_clicks_and_a_late_click_cannot_change_the_next_meeting() {
    let old = Fixture::new();
    let mut owner = Owner::register(&old.manifest.id);
    let queued = old.change(MeetingMode::SharedRoom);
    request(queued.clone()).unwrap();
    owner.close();
    assert!(queued.result().unwrap().is_err());
    let next = Fixture::new();
    let _next_owner = Owner::register(&next.manifest.id);
    assert!(request(old.change(MeetingMode::SharedRoom)).is_err());
    assert_eq!(old.saved().mode, MeetingMode::Standard);
    assert_eq!(next.saved().mode, MeetingMode::Standard);
}

#[test]
fn a_mode_change_queued_before_stop_is_drained_and_kept_for_the_final_pass() {
    let mut fixture = Fixture::new();
    let mut rolling = transcript::Rolling::start(
        fixture.directory.clone(),
        Default::default(),
        MeetingMode::Standard,
    );
    // No audio is offered, no model is loaded, and no device is opened.
    let feed = rolling.feed();
    for mode in [MeetingMode::SharedRoom, MeetingMode::Standard] {
        let change = fixture.change(mode);
        fixture
            .owner
            .begin(
                &fixture.directory,
                &mut fixture.manifest,
                change.clone(),
                |mode| rolling.change_mode(mode),
            )
            .unwrap();
        fixture
            .owner
            .finish(&fixture.directory, &mut fixture.manifest)
            .unwrap();
        assert_eq!(change.result(), Some(Ok(())));
        assert_eq!(fixture.saved().mode, mode);
    }
    let last = fixture.change(MeetingMode::SharedRoom);
    fixture
        .owner
        .begin(
            &fixture.directory,
            &mut fixture.manifest,
            last.clone(),
            |mode| rolling.change_mode(mode),
        )
        .unwrap();
    fixture.owner.close();
    drop(feed);
    drop(rolling.finish());
    fixture
        .owner
        .finish(&fixture.directory, &mut fixture.manifest)
        .unwrap();
    fixture.manifest.status = MeetingStatus::Complete;
    write_manifest(&fixture.directory, &fixture.manifest).unwrap();
    assert_eq!(last.result(), Some(Ok(())));
    assert_eq!(fixture.saved().mode, MeetingMode::SharedRoom);
    assert_eq!(fixture.saved().status, MeetingStatus::Complete);
}
