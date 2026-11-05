//! The `speaker` filter of a transcript query: a source, or a named
//! participant of the call audio (Labs › Meeting speaker names).

use color_eyre::eyre::{Result, eyre};

use super::fold;
use crate::meeting::{MeetingSource, TranscriptEntry};

/// Which lines a transcript shows: one source, or one named participant
/// (a part of the call audio).
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum SpeakerFilter {
    Source(MeetingSource),
    /// A participant's name, folded.
    Named(String),
}

impl SpeakerFilter {
    pub(super) fn matches(&self, entry: &TranscriptEntry) -> bool {
        match self {
            Self::Source(source) => entry.source == *source,
            Self::Named(name) => entry
                .speaker_name()
                .is_some_and(|known| fold(known) == *name),
        }
    }
}

pub(super) fn parse_speaker(speaker: &str) -> Result<Option<SpeakerFilter>> {
    match fold(speaker.trim()).as_str() {
        "" | "all" | "both" | "any" => Ok(None),
        "you" | "me" | "kit" | "microphone" | "mic" | "local" => {
            Ok(Some(SpeakerFilter::Source(MeetingSource::Microphone)))
        }
        "them" | "others" | "other" | "remote" | "system" | "computer" => {
            Ok(Some(SpeakerFilter::Source(MeetingSource::System)))
        }
        other if crate::meeting::speakers::is_valid_name(speaker.trim()) => {
            Ok(Some(SpeakerFilter::Named(other.into())))
        }
        other => Err(eyre!(
            "Unknown speaker {other:?}; use \"you\", \"them\", \"all\", or a participant's name."
        )),
    }
}

/// Why a name filter matched nothing, with the names the meeting has.
pub(super) fn unknown_name(name: &str, entries: &[TranscriptEntry]) -> color_eyre::Report {
    let mut names = entries
        .iter()
        .filter_map(TranscriptEntry::speaker_name)
        .collect::<Vec<_>>();
    names.sort_unstable();
    names.dedup();
    if names.is_empty() {
        eyre!(
            "No speaker {name:?} in this meeting: it has no named speakers. Use \"you\", \"them\", or \"all\"."
        )
    } else {
        let shown = names
            .iter()
            .take(20)
            .copied()
            .collect::<Vec<_>>()
            .join(", ");
        eyre!("No speaker {name:?} in this meeting; its named speakers are {shown}.")
    }
}
