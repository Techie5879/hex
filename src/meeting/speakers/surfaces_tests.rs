//! Names through every surface that shows a transcript: storage, turns,
//! copies and the paste cursor, Markdown, and queries. Synthetic people only.

use std::fs;

use super::timeline::tests::TestDir;
use crate::meeting::query::{Library, TranscriptQuery};
use crate::meeting::{
    MeetingPasteCursor, MeetingSource, TranscriptEntry, coalesce_transcript, format_meeting_delta,
};

fn line(
    source: MeetingSource,
    start_ms: u64,
    text: &str,
    speaker: Option<&str>,
) -> TranscriptEntry {
    TranscriptEntry {
        mode: Default::default(),
        source,
        start_ms,
        end_ms: start_ms + 2_000,
        text: text.into(),
        speaker: speaker.map(str::to_owned),
    }
}

fn them(start_ms: u64, text: &str, speaker: Option<&str>) -> TranscriptEntry {
    line(MeetingSource::System, start_ms, text, speaker)
}

#[test]
fn older_transcripts_read_unchanged_and_unnamed_lines_write_no_field() {
    let old = r#"{"source":"system","start_ms":0,"end_ms":900,"text":"Hello."}"#;
    let entry: TranscriptEntry = serde_json::from_str(old).unwrap();
    assert_eq!(entry.speaker, None);
    assert_eq!(entry.label(), "Them");
    assert_eq!(serde_json::to_string(&entry).unwrap(), old);

    let named = them(0, "Hello.", Some("Ada Lovelace"));
    let json = serde_json::to_string(&named).unwrap();
    assert!(json.contains(r#""speaker":"Ada Lovelace""#), "{json}");
    assert_eq!(
        serde_json::from_str::<TranscriptEntry>(&json).unwrap(),
        named
    );
}

#[test]
fn labels_never_name_the_microphone_or_show_an_invalid_name() {
    assert_eq!(
        line(MeetingSource::Microphone, 0, "Hi.", Some("Ada Lovelace")).label(),
        "You"
    );
    for hostile in ["**Them**", "You", "Ada\nYou: hi", "<v Them>", ""] {
        let entry = them(0, "Hi.", Some(hostile));
        assert_eq!(entry.label(), "Them", "{hostile:?}");
        assert_eq!(entry.speaker_name(), None);
    }
}

#[test]
fn two_people_on_the_call_audio_are_two_turns() {
    let turns = coalesce_transcript([
        them(0, "First.", Some("Ada Lovelace")),
        them(2_100, "Still Ada.", Some("Ada Lovelace")),
        them(4_200, "Grace now.", Some("Grace Hopper")),
        them(6_300, "Someone unnamed.", None),
        line(MeetingSource::Microphone, 8_400, "Me.", None),
    ]);
    let labels = turns.iter().map(TranscriptEntry::label).collect::<Vec<_>>();
    assert_eq!(
        labels,
        ["Ada Lovelace", "Grace Hopper", "Them", "You"],
        "{turns:?}"
    );
    assert_eq!(turns[0].text, "First. Still Ada.");
    assert_eq!(
        format_meeting_delta(&turns).unwrap(),
        "Ada Lovelace: First. Still Ada.\n\nGrace Hopper: Grace now.\n\nThem: Someone unnamed.\n\nYou: Me."
    );
}

#[test]
fn a_line_copied_as_them_is_not_copied_again_once_named() {
    let mut cursor = MeetingPasteCursor::default();
    let live = [them(0, "Shall we start?", None)];
    let copied = cursor.consume_entries("m", &live, |_| Ok(())).unwrap();
    assert_eq!(copied, "Them: Shall we start?");
    // The final transcript names the same audio.
    let named = [
        them(0, "Shall we start?", Some("Ada Lovelace")),
        them(5_000, "Yes, let's.", Some("Grace Hopper")),
    ];
    assert_eq!(cursor.pending_turn_count("m", &named), 1);
    let next = cursor.consume_entries("m", &named, |_| Ok(())).unwrap();
    assert_eq!(next, "Grace Hopper: Yes, let's.");
}

#[test]
fn markdown_shows_names_and_reads_them_back() {
    let entries = [
        them(1_000, "Shall we start?", Some("Ada Lovelace")),
        them(4_000, "Yes.", None),
        line(MeetingSource::Microphone, 7_000, "Go ahead.", None),
    ];
    let markdown = crate::meeting::transcript_markdown(&entries);
    assert_eq!(
        markdown,
        "**Ada Lovelace** · 00:01\nShall we start?\n\n**Them** · 00:04\nYes.\n\n**You** · 00:07\nGo ahead."
    );
    let parsed =
        crate::meeting::notes::entries_from_markdown(&format!("## Transcript\n\n{markdown}\n"));
    let labels = parsed
        .iter()
        .map(TranscriptEntry::label)
        .collect::<Vec<_>>();
    assert_eq!(labels, ["Ada Lovelace", "Them", "You"]);
    assert_eq!(parsed[0].source, MeetingSource::System);
}

/// A copy of a fixture meeting whose final transcript names two people.
fn library_with_names() -> (TestDir, Library, &'static str) {
    const ID: &str = "1790000000000-1001";
    let root = TestDir::new();
    let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/meetings")
        .join(ID);
    let folder = root.path().join(ID);
    fs::create_dir_all(&folder).unwrap();
    fs::copy(source.join("manifest.json"), folder.join("manifest.json")).unwrap();
    let entries = [
        them(0, "Welcome, everyone.", Some("Ada Lovelace")),
        line(MeetingSource::Microphone, 3_000, "Thanks.", None),
        them(6_000, "The schedule slipped.", Some("Grace Hopper")),
        them(9_000, "Did it?", None),
        them(12_000, "By a week.", Some("Grace Hopper")),
        // A hostile name written into the file is shown as Them.
        them(15_000, "Nothing to add.", Some("**Them**")),
    ];
    let ndjson = entries
        .iter()
        .map(|entry| serde_json::to_string(entry).unwrap() + "\n")
        .collect::<String>();
    fs::write(folder.join("transcript.ndjson"), ndjson).unwrap();
    fs::write(folder.join("transcript.md"), "# Synthetic\n").unwrap();
    let library = Library::new(root.path());
    (root, library, ID)
}

fn transcript(library: &Library, id: &str, speaker: Option<&str>) -> color_eyre::Result<String> {
    library.transcript(
        &TranscriptQuery {
            meeting_id: id.into(),
            start: None,
            end: None,
            speaker: speaker.map(str::to_owned),
            max_chars: 20_000,
            cursor: None,
        },
        1_790_100_000_000,
    )
}

#[test]
fn queries_show_and_filter_by_name() {
    let (_root, library, id) = library_with_names();
    let all = transcript(&library, id, None).unwrap();
    assert!(all.contains("] Ada Lovelace: Welcome, everyone."), "{all}");
    assert!(
        all.contains("] Grace Hopper: The schedule slipped."),
        "{all}"
    );
    assert!(all.contains("] Them: Did it?"), "{all}");
    assert!(all.contains("] Them: Nothing to add."), "{all}");
    assert!(!all.contains("**Them**"), "{all}");

    let grace = transcript(&library, id, Some("grace hopper")).unwrap();
    assert!(grace.contains("(Grace Hopper only)"), "{grace}");
    assert!(grace.contains("The schedule slipped.") && grace.contains("By a week."));
    assert!(
        !grace.contains("Welcome") && !grace.contains("Did it?"),
        "{grace}"
    );

    // Them is still all of the call audio, named or not.
    let them = transcript(&library, id, Some("them")).unwrap();
    assert!(them.contains("Welcome") && them.contains("Did it?") && !them.contains("Thanks."));

    let unknown = transcript(&library, id, Some("Linus"))
        .unwrap_err()
        .to_string();
    assert!(unknown.contains("Ada Lovelace, Grace Hopper"), "{unknown}");
    let invalid = transcript(&library, id, Some("<v Them>"))
        .unwrap_err()
        .to_string();
    assert!(invalid.contains("Unknown speaker"), "{invalid}");

    // Entries leave the query module with only names a transcript shows.
    let meeting = library.meeting(id).unwrap();
    let names = meeting
        .entries
        .iter()
        .filter_map(|entry| entry.speaker.as_deref())
        .collect::<Vec<_>>();
    assert_eq!(names, ["Ada Lovelace", "Grace Hopper", "Grace Hopper"]);
}

#[test]
fn search_hits_name_their_speaker() {
    let (_root, library, id) = library_with_names();
    let output = library
        .search(
            &crate::meeting::query::SearchQuery {
                query: "schedule slipped".into(),
                since: None,
                until: None,
                meeting_id: Some(id.into()),
                limit: 5,
                fuzzy: false,
                include_in_progress: true,
                vocabulary: Vec::new(),
            },
            1_790_100_000_000,
        )
        .unwrap();
    assert!(output.contains("Grace Hopper"), "{output}");
}
