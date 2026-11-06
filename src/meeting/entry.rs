//! One transcript line, who it is labelled as, and how a meeting was
//! recorded. Labels depend only on the entry: its source, the participant
//! name the call app showed (Labs › Meeting speaker names), and the
//! meeting's [`MeetingMode`], which every entry carries from its manifest.

use serde::{Deserialize, Serialize};

use super::speakers;

const TRANSCRIPT_TURN_GAP_MS: u64 = 3_000;

/// How a meeting is recorded, chosen for the active recording and stored in
/// its manifest. Every recording starts Standard. Older manifests
/// and transcript files have none, which reads as `Standard`.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MeetingMode {
    /// The microphone is the person recording (`You`), the system audio is
    /// everyone else (`Them`).
    #[default]
    Standard,
    /// Shared room: several people share the room and its
    /// microphone, and the call may carry the same room again. Lines are
    /// labelled by capture path (`Room mic`, `Call audio`), never as a
    /// person, and room speech the call audio repeats is kept once.
    SharedRoom,
}

impl MeetingMode {
    pub fn is_standard(&self) -> bool {
        *self == Self::Standard
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MeetingSource {
    System,
    Microphone,
}

impl MeetingSource {
    /// `You` and `Them`, or in a shared room the capture path, since the
    /// microphone there is not one person.
    pub fn label(self, mode: MeetingMode) -> &'static str {
        match (mode, self) {
            (MeetingMode::Standard, Self::System) => "Them",
            (MeetingMode::Standard, Self::Microphone) => "You",
            (MeetingMode::SharedRoom, Self::System) => "Call audio",
            (MeetingMode::SharedRoom, Self::Microphone) => "Room mic",
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
pub struct TranscriptEntry {
    /// Which recording it came from. Echo suppression and the paste cursor
    /// work on this alone; a name never changes it.
    pub source: MeetingSource,
    pub start_ms: u64,
    pub end_ms: u64,
    pub text: String,
    /// The participant the call app showed speaking throughout a system-audio
    /// entry (Labs › Meeting speaker names); absent means `Them`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub speaker: Option<String>,
    /// The meeting's mode, copied from its manifest when the entry is joined.
    #[serde(default, skip_serializing_if = "MeetingMode::is_standard")]
    pub mode: MeetingMode,
}

impl TranscriptEntry {
    /// `You`, `Them`, or the participant's name; in a shared room, `Room
    /// mic` or `Call audio`. A name is shown only on system audio, only
    /// while it is a valid name, whatever a file says, and never in a shared
    /// room, where one call account can carry everyone in the room.
    pub fn label(&self) -> &str {
        match (self.mode, &self.source, self.speaker.as_deref()) {
            (MeetingMode::Standard, MeetingSource::System, Some(name))
                if speakers::is_valid_name(name) =>
            {
                name
            }
            (mode, source, _) => source.label(mode),
        }
    }

    /// The participant's name, when [`label`](Self::label) shows one.
    pub fn speaker_name(&self) -> Option<&str> {
        (self.label() != self.source.label(self.mode)).then(|| self.label())
    }
}

/// What a notes or chapters prompt says first about a shared room's labels,
/// so the model reads neither as a person; empty for other meetings.
pub(super) fn label_context(entries: &[TranscriptEntry]) -> &'static str {
    if entries
        .iter()
        .any(|entry| entry.mode == MeetingMode::SharedRoom)
    {
        "Speaker labels: Room mic is the microphone of the computer that recorded, shared by \
everyone in its room. Call audio is the call as that computer played it, and can carry the room \
again. Neither label is one person; never attribute a line to someone by these labels.\n\n"
    } else {
        ""
    }
}

/// Joins consecutive entries by one speaker: the same source and, for the
/// call audio, the same name, so two people never share a turn.
pub fn coalesce_transcript(
    entries: impl IntoIterator<Item = TranscriptEntry>,
) -> Vec<TranscriptEntry> {
    let mut turns: Vec<TranscriptEntry> = Vec::new();
    for entry in entries {
        if let Some(turn) = turns.last_mut()
            && turn.source == entry.source
            && turn.label() == entry.label()
            && entry.start_ms.saturating_sub(turn.end_ms) <= TRANSCRIPT_TURN_GAP_MS
        {
            if !turn.text.ends_with(char::is_whitespace) {
                turn.text.push(' ');
            }
            turn.text.push_str(entry.text.trim());
            turn.end_ms = turn.end_ms.max(entry.end_ms);
        } else {
            turns.push(entry);
        }
    }
    turns
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(source: MeetingSource, mode: MeetingMode, speaker: Option<&str>) -> TranscriptEntry {
        TranscriptEntry {
            source,
            start_ms: 0,
            end_ms: 1_000,
            text: "Words".into(),
            speaker: speaker.map(str::to_owned),
            mode,
        }
    }

    #[test]
    fn standard_meetings_keep_you_them_and_names() {
        use MeetingSource::{Microphone, System};
        let standard = MeetingMode::Standard;
        assert_eq!(entry(Microphone, standard, None).label(), "You");
        assert_eq!(entry(System, standard, None).label(), "Them");
        let named = entry(System, standard, Some("Ada Lovelace"));
        assert_eq!(named.label(), "Ada Lovelace");
        assert_eq!(named.speaker_name(), Some("Ada Lovelace"));
    }

    #[test]
    fn shared_room_lines_name_the_capture_path_and_never_a_person() {
        use MeetingSource::{Microphone, System};
        let room = MeetingMode::SharedRoom;
        assert_eq!(entry(Microphone, room, None).label(), "Room mic");
        assert_eq!(entry(System, room, None).label(), "Call audio");
        // The call account carrying the room is not every person in it.
        let named = entry(System, room, Some("Ada Lovelace"));
        assert_eq!(named.label(), "Call audio");
        assert_eq!(named.speaker_name(), None);
        assert_eq!(named.speaker.as_deref(), Some("Ada Lovelace"));
    }

    #[test]
    fn older_files_without_a_mode_read_as_standard_and_standard_is_not_written() {
        let old: TranscriptEntry =
            serde_json::from_str(r#"{"source":"microphone","start_ms":0,"end_ms":5,"text":"Hi"}"#)
                .unwrap();
        assert_eq!(old.mode, MeetingMode::Standard);
        assert_eq!(old.label(), "You");
        assert!(!serde_json::to_string(&old).unwrap().contains("mode"));
        let room = TranscriptEntry {
            mode: MeetingMode::SharedRoom,
            ..old
        };
        let written = serde_json::to_string(&room).unwrap();
        assert!(written.contains(r#""mode":"shared_room""#), "{written}");
        assert_eq!(
            serde_json::from_str::<TranscriptEntry>(&written).unwrap(),
            room
        );
    }

    #[test]
    fn room_and_call_lines_never_share_a_turn() {
        let room = MeetingMode::SharedRoom;
        let mut call = entry(MeetingSource::System, room, None);
        call.start_ms = 1_200;
        call.end_ms = 2_000;
        let turns = coalesce_transcript([entry(MeetingSource::Microphone, room, None), call]);
        assert_eq!(turns.len(), 2);
    }
}
