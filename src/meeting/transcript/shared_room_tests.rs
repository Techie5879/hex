//! Per-meeting shared-room mode, through the same paths as recordings: echo
//! suppression, the rolling view, and the final pass. Every line here is
//! invented; none comes from a real meeting.

use super::super::*;

fn line(source: MeetingSource, start_ms: u64, end_ms: u64, text: &str) -> TranscriptEntry {
    TranscriptEntry {
        source,
        start_ms,
        end_ms,
        text: text.into(),
        speaker: None,
        mode: MeetingMode::Standard,
    }
}

fn room(start_ms: u64, end_ms: u64, text: &str) -> TranscriptEntry {
    line(MeetingSource::Microphone, start_ms, end_ms, text)
}

fn call(start_ms: u64, end_ms: u64, text: &str) -> TranscriptEntry {
    line(MeetingSource::System, start_ms, end_ms, text)
}

/// Shared-room suppression where the signal calls every microphone span
/// local speech, as it does for people talking in the room.
fn shared(entries: Vec<TranscriptEntry>) -> (Vec<TranscriptEntry>, u64) {
    suppress_echo(entries, MeetingMode::SharedRoom, |_, _| Some(Signal::Local))
}

fn texts(entries: &[TranscriptEntry], source: MeetingSource) -> Vec<&str> {
    entries
        .iter()
        .filter(|entry| entry.source == source)
        .map(|entry| entry.text.as_str())
        .collect()
}

const PLAN: &str = "Let's move the design review to Thursday afternoon.";

#[test]
fn matching_a_suffix_never_erases_polarity_or_a_call_qualification() {
    for (said, heard) in [
        (
            "Ship the build on Friday afternoon.",
            "Do not ship the build on Friday afternoon.",
        ),
        (
            "Ship the build on Friday afternoon.",
            "Ship the build on Friday afternoon unless the tests fail.",
        ),
        (
            "Do not ship the build on Friday afternoon.",
            "Ship the build on Friday afternoon.",
        ),
    ] {
        let (kept, dropped) = shared(vec![
            room(10_000, 12_500, said),
            call(10_200, 12_700, heard),
        ]);
        assert_eq!(dropped, 0, "{said:?} versus {heard:?}");
        assert_eq!(texts(&kept, MeetingSource::Microphone), [said]);
    }
    // A recognizer can split the qualification from its sentence. Never
    // remove just the remaining suffix after the full utterance disagrees.
    let (kept, dropped) = shared(vec![
        room(10_000, 10_400, "Do not"),
        room(10_500, 12_500, "ship the build on Friday afternoon."),
        call(10_200, 12_700, "Ship the build on Friday afternoon."),
    ]);
    assert_eq!(dropped, 0);
    assert_eq!(kept.len(), 3);
}

#[test]
fn numeric_signs_decimal_points_and_units_are_comparison_content() {
    for (said, heard) in [
        ("-0.5", "+0.5"),
        (".5", "5"),
        ("-.5", "-5"),
        ("0.5%", "0.5"),
    ] {
        let said = format!("Set the adjustment to {said} this quarter.");
        let heard = format!("Set the adjustment to {heard} this quarter.");
        let (kept, dropped) = shared(vec![
            room(10_000, 12_500, &said),
            call(10_200, 12_700, &heard),
        ]);
        assert_eq!(dropped, 0, "{said:?} versus {heard:?}");
        assert_eq!(kept.len(), 2);
    }
    let signed = "Set the adjustment to -0.5 this quarter.";
    assert_eq!(
        shared(vec![
            room(10_000, 12_500, signed),
            call(10_200, 12_700, signed)
        ])
        .1,
        1
    );
}

#[test]
fn room_speech_the_call_repeats_is_kept_once_even_when_the_signal_says_local() {
    let entries = vec![room(10_000, 12_500, PLAN), call(10_600, 13_100, PLAN)];
    let (kept, dropped) = shared(entries.clone());
    assert_eq!(dropped, 1);
    assert!(texts(&kept, MeetingSource::Microphone).is_empty());
    // The call copy is the one kept.
    assert_eq!(texts(&kept, MeetingSource::System), [PLAN]);

    // Standard meetings are unchanged: local speech is never echo.
    let (kept, dropped) = suppress_echo(entries, MeetingMode::Standard, |_, _| Some(Signal::Local));
    assert_eq!(dropped, 0);
    assert_eq!(kept.len(), 2);
}

#[test]
fn room_only_speech_survives_beside_unrelated_call_audio() {
    let aside = "Can someone grab the whiteboard markers from the cupboard?";
    let (kept, dropped) = shared(vec![
        room(5_000, 7_000, aside),
        call(
            5_200,
            8_000,
            "I think the numbers from last quarter look fine.",
        ),
    ]);
    assert_eq!(dropped, 0);
    assert_eq!(texts(&kept, MeetingSource::Microphone), [aside]);
}

#[test]
fn two_copies_from_one_source_are_never_compared() {
    let again = "Please read the second paragraph aloud again.";
    let (kept, dropped) = shared(vec![
        room(1_000, 3_000, again),
        room(3_500, 5_500, again),
        call(20_000, 22_000, PLAN),
        call(22_500, 24_500, PLAN),
    ]);
    assert_eq!(dropped, 0);
    assert_eq!(kept.len(), 4);
}

#[test]
fn short_and_common_phrases_stay_on_both_sides() {
    // Said in the room and on the call, these can be two people.
    let phrases = [
        "Sounds good to me.",
        "Yeah, that works.",
        "Thank you so much.",
    ];
    let mut entries = Vec::new();
    for (index, phrase) in phrases.iter().enumerate() {
        let at = index as u64 * 20_000;
        entries.push(room(at, at + 1_200, phrase));
        entries.push(call(at + 300, at + 1_500, phrase));
    }
    let (kept, dropped) = shared(entries);
    assert_eq!(dropped, 0);
    assert_eq!(texts(&kept, MeetingSource::Microphone), phrases);
}

#[test]
fn a_changed_negation_number_name_qualifier_or_extra_clause_keeps_the_room_copy() {
    let pairs = [
        (
            "We can ship the beta on Friday.",
            "We can't ship the beta on Friday.",
        ),
        (
            "We can't ship the beta on Friday.",
            "We can ship the beta on Friday.",
        ),
        (
            "The vendor is ready for the migration.",
            "The vendor is not ready for the migration.",
        ),
        (
            "We need forty seats for the offsite in March.",
            "We need fourteen seats for the offsite in March.",
        ),
        (
            "Book 14 rooms for the offsite in March.",
            "Book 40 rooms for the offsite in March.",
        ),
        (
            "Send the contract to Priya before the review.",
            "Send the contract to Maria before the review.",
        ),
        (
            "We should ship on Friday unless the tests fail again.",
            "We should ship on Friday.",
        ),
        (
            "That damn build broke again this morning.",
            "That build broke again this morning.",
        ),
        (
            "The launch is probably slipping to next week.",
            "The launch is slipping to next week.",
        ),
    ];
    let mut entries = Vec::new();
    for (index, (said, heard)) in pairs.iter().enumerate() {
        let at = index as u64 * 30_000;
        entries.push(room(at, at + 3_000, said));
        entries.push(call(at + 400, at + 3_400, heard));
    }
    let (kept, dropped) = shared(entries);
    assert_eq!(dropped, 0, "{kept:#?}");
    assert_eq!(
        texts(&kept, MeetingSource::Microphone),
        pairs.map(|(said, _)| said)
    );
}

#[test]
fn fillers_articles_case_and_punctuation_may_differ() {
    let (kept, dropped) = shared(vec![
        room(
            10_000,
            13_000,
            "Um, so the BUILD broke — again this morning.",
        ),
        call(10_500, 13_500, "The build broke again this morning."),
    ]);
    assert_eq!(dropped, 1);
    assert!(texts(&kept, MeetingSource::Microphone).is_empty());
}

#[test]
fn one_call_copy_stands_behind_one_room_copy() {
    let slide = "Put the third quarter numbers on the second slide.";
    // The room says it twice; the call carries it once.
    let (kept, dropped) = shared(vec![
        room(10_000, 12_000, slide),
        room(13_000, 15_000, slide),
        call(10_400, 12_400, slide),
    ]);
    assert_eq!(dropped, 1);
    assert_eq!(texts(&kept, MeetingSource::Microphone), [slide]);
    assert_eq!(
        kept.iter()
            .find(|entry| entry.source == MeetingSource::Microphone)
            .unwrap()
            .start_ms,
        13_000
    );
}

#[test]
fn the_call_copy_must_be_close_in_time() {
    let judge = |call_start: u64| {
        shared(vec![
            room(10_000, 12_000, PLAN),
            call(call_start, call_start + 2_000, PLAN),
        ])
        .1
    };
    // Up to CALL_LAG_MS after and CALL_LEAD_MS before.
    assert_eq!(judge(12_500), 1);
    assert_eq!(judge(8_500), 1);
    // A repeat much later, or well before, is someone saying it again.
    assert_eq!(judge(17_500), 0);
    assert_eq!(judge(6_000), 0);
}

#[test]
fn an_earlier_repeat_outside_the_window_does_not_hide_the_copy_inside_it() {
    // Both call copies lie in the window, but the first ends too early to
    // be this sentence; the later one is its copy.
    let (kept, dropped) = shared(vec![
        call(8_000, 9_900, PLAN),
        room(10_000, 12_000, PLAN),
        call(10_500, 12_500, PLAN),
    ]);
    assert_eq!(dropped, 1);
    assert!(texts(&kept, MeetingSource::Microphone).is_empty());
    assert_eq!(texts(&kept, MeetingSource::System), [PLAN, PLAN]);
}

#[test]
fn sentences_split_differently_by_the_two_sources_still_match() {
    let whole = "We agreed to freeze the API on Monday and ship the client on Wednesday.";
    let (kept, dropped) = shared(vec![
        room(10_000, 16_000, whole),
        call(10_300, 13_000, "We agreed to freeze the API on Monday"),
        call(13_100, 16_500, "and ship the client on Wednesday."),
    ]);
    assert_eq!(dropped, 1);
    assert!(texts(&kept, MeetingSource::Microphone).is_empty());

    let (kept, dropped) = shared(vec![
        room(10_000, 12_800, "We agreed to freeze the API on Monday"),
        room(13_000, 16_000, "and ship the client on Wednesday."),
        call(10_400, 16_400, whole),
    ]);
    assert_eq!(dropped, 2);
    assert_eq!(texts(&kept, MeetingSource::System), [whole]);
}

#[test]
fn bleed_the_signal_hears_is_still_dropped() {
    let (kept, dropped) = suppress_echo(
        vec![
            room(10_000, 12_000, "Remote audio the loudspeaker played."),
            call(9_950, 11_950, "Remote audio the loudspeaker played back."),
        ],
        MeetingMode::SharedRoom,
        |_, _| Some(Signal::Bleed),
    );
    assert_eq!(dropped, 1);
    assert!(texts(&kept, MeetingSource::Microphone).is_empty());
}

#[test]
fn shared_words_out_of_order_are_not_a_duplicate() {
    // Most words recur, so the standard word check calls it echo when the
    // signal can't tell; a shared room needs the same words in order.
    let entries = vec![
        room(10_000, 12_500, "Friday is when the roadmap review happens."),
        call(10_200, 12_700, "The roadmap review happens on Friday."),
    ];
    let (_, standard) = suppress_echo(entries.clone(), MeetingMode::Standard, |_, _| None);
    assert_eq!(standard, 1);
    let (kept, dropped) = suppress_echo(entries, MeetingMode::SharedRoom, |_, _| None);
    assert_eq!(dropped, 0);
    assert_eq!(texts(&kept, MeetingSource::Microphone).len(), 1);
}

#[test]
fn a_long_meeting_drops_every_repeat_and_keeps_every_aside() {
    let mut entries = Vec::new();
    for index in 0..2_000_u64 {
        let at = index * 8_000;
        let said = format!("Item {index} on the agenda moves to the next sprint.");
        entries.push(room(at, at + 2_500, &said));
        entries.push(call(at + 500, at + 3_000, &said));
        entries.push(room(
            at + 4_000,
            at + 5_000,
            &format!("Room aside number {index} about the projector."),
        ));
    }
    let (kept, dropped) = shared(entries);
    assert_eq!(dropped, 2_000);
    let room_kept = texts(&kept, MeetingSource::Microphone);
    assert_eq!(room_kept.len(), 2_000);
    assert!(room_kept.iter().all(|text| text.starts_with("Room aside")));
}

fn track(source: MeetingSource, through_ms: u64, entries: &[TranscriptEntry]) -> TrackTranscript {
    let mut track = TrackTranscript::new(source, 0);
    track.settled_ms = through_ms;
    track.entries = entries.to_vec();
    track
}

#[test]
fn room_publication_waits_until_an_adjacent_qualification_is_settled() {
    let first = "We should ship the client on Wednesday.";
    let qualification = "Unless the final review finds a blocker.";
    let whole = format!("{first} {qualification}");
    let system = track(
        MeetingSource::System,
        20_000,
        &[call(10_200, 14_200, &whole)],
    );
    let mut microphone = track(
        MeetingSource::Microphone,
        12_600,
        &[room(10_000, 12_500, first)],
    );
    let envelope = EchoEnvelope::default();
    let early = rolling_view(
        MeetingMode::SharedRoom,
        [Some(&system), Some(&microphone)],
        true,
        0,
        &envelope,
    );
    assert!(texts(&early.entries, MeetingSource::Microphone).is_empty());
    assert_eq!(early.covered_ms.microphone, Some(11_900));
    microphone.entries.push(room(12_600, 14_200, qualification));
    microphone.settled_ms = 20_000;
    let settled = rolling_view(
        MeetingMode::SharedRoom,
        [Some(&system), Some(&microphone)],
        true,
        0,
        &envelope,
    );
    assert!(texts(&settled.entries, MeetingSource::Microphone).is_empty());
    assert_eq!(texts(&settled.entries, MeetingSource::System), [whole]);
}

#[test]
fn the_rolling_view_waits_for_the_call_copy_before_showing_room_speech() {
    let envelope = EchoEnvelope::default();
    let microphone = track(
        MeetingSource::Microphone,
        20_000,
        &[
            room(10_000, 12_500, PLAN),
            room(14_000, 15_000, "Can someone close the window please?"),
        ],
    );
    let view = |system: &TrackTranscript| {
        rolling_view(
            MeetingMode::SharedRoom,
            [Some(system), Some(&microphone)],
            true,
            0,
            &envelope,
        )
    };
    // The call covers the room sentence, but its copy may still arrive.
    let early = view(&track(MeetingSource::System, 13_000, &[]));
    assert!(early.entries.is_empty(), "{:#?}", early.entries);
    assert_eq!(early.covered_ms.microphone, Some(9_000));

    // Once it has, the room copy is never shown and the aside is.
    let late = view(&track(
        MeetingSource::System,
        20_000,
        &[call(10_600, 13_100, PLAN)],
    ));
    assert_eq!(
        late.entries
            .iter()
            .map(|entry| (entry.label(), entry.text.as_str()))
            .collect::<Vec<_>>(),
        [
            ("Call audio", PLAN),
            ("Room mic", "Can someone close the window please?")
        ]
    );
    assert!(
        late.entries
            .iter()
            .all(|entry| entry.mode == MeetingMode::SharedRoom)
    );
}

fn manifest(mode: Option<&str>) -> MeetingManifest {
    let mut json = serde_json::json!({
        "schema_version": 1,
        "id": "1",
        "title": "Planning",
        "status": "transcribing",
        "created_at_ms": 1_760_000_000_000_u64,
        "ended_at_ms": null,
        "duration_ms": null,
        "system": {"file": "system.wav", "samples": 0, "dropped_packets": 0},
        "microphone": {"file": "microphone.wav", "samples": 0, "dropped_packets": 0},
        "error": null,
    });
    if let Some(mode) = mode {
        json["mode"] = mode.into();
    }
    serde_json::from_value(json).unwrap()
}

/// The final pass after a restart, from the rolling tracks and the stored
/// manifest alone: no preference or runtime switch is consulted.
fn final_pass(manifest: &MeetingManifest) -> (Vec<TranscriptEntry>, u64) {
    let directory = std::env::temp_dir().join(format!(
        "hex-shared-room-final-{}-{}",
        std::process::id(),
        crate::desktop::events::now_ms()
    ));
    fs::create_dir_all(&directory).unwrap();
    let mut system = track(
        MeetingSource::System,
        30_000,
        &[
            call(10_600, 13_100, PLAN),
            call(20_000, 22_000, "Remote update."),
        ],
    );
    system.finish();
    let mut microphone = track(
        MeetingSource::Microphone,
        30_000,
        &[room(10_000, 12_500, PLAN)],
    );
    microphone.finish();
    let outcome = RollingOutcome {
        tracks: [Some(system), Some(microphone)],
        cuts: Default::default(),
        envelope: EchoEnvelope::default(),
        model: ChunkModel::new(TranscriptionSelection::default()),
    };
    let result = transcribe(
        &directory,
        manifest,
        &TranscriptionSelection::default(),
        Some(outcome),
    )
    .unwrap();
    fs::remove_dir_all(directory).unwrap();
    result
}

#[test]
fn the_final_pass_uses_the_mode_stored_with_the_meeting() {
    let (entries, dropped) = final_pass(&manifest(Some("shared_room")));
    assert_eq!(dropped, 1);
    assert_eq!(
        entries
            .iter()
            .map(TranscriptEntry::label)
            .collect::<Vec<_>>(),
        ["Call audio", "Call audio"]
    );

    // A manifest written before the mode existed is a standard meeting.
    let older = manifest(None);
    assert_eq!(older.mode, MeetingMode::Standard);
    let (entries, _) = final_pass(&older);
    assert!(
        entries
            .iter()
            .all(|entry| entry.mode == MeetingMode::Standard)
    );
    assert!(entries.iter().any(|entry| entry.label() == "Them"));
}

struct NoInference;
impl ChunkTranscriber for NoInference {
    fn transcribe_chunk(&mut self, _: &Chunk) -> Result<Chunked> {
        panic!("a mode change must reuse the original transcript without inference")
    }
}

#[test]
fn changing_the_live_mode_rebuilds_existing_text_without_resetting_copy_cuts() {
    let directory = std::env::temp_dir().join(format!(
        "hex-mode-live-{}-{}",
        std::process::id(),
        crate::desktop::events::now_ms()
    ));
    fs::create_dir_all(&directory).unwrap();
    let call = call(10_200, 12_700, "Do not ship the build on Friday afternoon.");
    let room = room(10_000, 12_500, "Ship the build on Friday afternoon.");
    let mut worker = RollingWorker::new(&directory, MeetingMode::Standard, NoInference);
    worker.tracks = [MeetingSource::System, MeetingSource::Microphone].map(|source| {
        let entries = if source == MeetingSource::System {
            vec![call.clone()]
        } else {
            vec![room.clone()]
        };
        Some(RollingTrack {
            resampler: None,
            sample_rate: RATE as u32,
            chunker: Chunker::default(),
            base: 0,
            transcript: track(source, 30_000, &entries),
            fed: RATE * 30,
            next_position: 0,
            pending: 0,
            failed: false,
        })
    });
    worker.cuts = [vec![12_700], vec![12_500]];
    worker.save_snapshot().unwrap();
    let read = || -> Vec<TranscriptEntry> {
        super::super::super::read_ndjson::<RollingLine>(&directory.join(ROLLING_FILE))
            .unwrap()
            .into_iter()
            .collect::<RollingView>()
            .entries
    };
    let original_tracks = worker
        .tracks
        .each_ref()
        .map(|track| track.as_ref().unwrap().transcript.entries.clone());
    let original_cuts = worker.cuts.clone();
    let standard = read();
    assert_eq!(
        standard.len(),
        1,
        "the standard approximate echo rule suppresses the room sentence"
    );
    assert_eq!(standard[0].label(), "Them");
    let mut cursor = crate::meeting::MeetingPasteCursor::default();
    cursor
        .consume_entries("synthetic", &standard, |_| Ok(()))
        .unwrap();

    let (reply, applied) = mpsc::sync_channel(1);
    worker.change_mode(MeetingMode::SharedRoom, reply);
    assert_eq!(applied.recv().unwrap(), Ok(()));
    let shared = read();
    assert_eq!(
        shared.len(),
        2,
        "shared-room comparison preserves the opposite statement"
    );
    assert!(shared.iter().any(|entry| entry.label() == "Room mic"));
    assert!(shared.iter().any(|entry| entry.label() == "Call audio"));
    assert_eq!(
        cursor.pending_turn_count("synthetic", &shared),
        1,
        "the already copied call is not pasted again under its new label"
    );
    let copied = cursor
        .consume_entries("synthetic", &shared, |_| Ok(()))
        .unwrap();
    assert!(copied.starts_with("Room mic:"));
    assert!(!copied.contains("Do not"));

    let (reply, applied) = mpsc::sync_channel(1);
    worker.change_mode(MeetingMode::Standard, reply);
    assert_eq!(applied.recv().unwrap(), Ok(()));
    assert_eq!(read(), standard);
    assert_eq!(cursor.pending_turn_count("synthetic", &read()), 0);
    assert_eq!(worker.cuts, original_cuts);
    assert_eq!(
        worker
            .tracks
            .each_ref()
            .map(|track| track.as_ref().unwrap().transcript.entries.clone()),
        original_tracks
    );
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn a_live_snapshot_write_failure_keeps_the_previous_worker_mode() {
    let missing = std::env::temp_dir()
        .join(format!("hex-missing-mode-{}", std::process::id()))
        .join("missing");
    let mut worker = RollingWorker::new(&missing, MeetingMode::Standard, NoInference);
    worker.microphone_shown_ms = 123;
    let (reply, applied) = mpsc::sync_channel(1);
    worker.change_mode(MeetingMode::SharedRoom, reply);
    assert!(applied.recv().unwrap().is_err());
    assert_eq!(worker.mode, MeetingMode::Standard);
    assert_eq!(worker.microphone_shown_ms, 123);
}
