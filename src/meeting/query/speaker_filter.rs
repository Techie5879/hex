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

pub(super) fn parse_speaker(
    speaker: &str,
    entries: &[TranscriptEntry],
) -> Result<Option<SpeakerFilter>> {
    let name = fold(speaker.trim());
    // Canonical selectors keep their meaning; recorded names win over
    // historical aliases such as Kit, Local, or Computer.
    if !matches!(
        name.as_str(),
        "" | "you" | "them" | "all" | "room mic" | "call audio"
    ) && entries.iter().any(|entry| {
        entry
            .speaker_name()
            .is_some_and(|known| fold(known) == name)
    }) {
        return Ok(Some(SpeakerFilter::Named(name)));
    }
    match name.as_str() {
        "" | "all" | "both" | "any" => Ok(None),
        "you" | "me" | "kit" | "microphone" | "mic" | "local" | "room" | "room mic" => {
            Ok(Some(SpeakerFilter::Source(MeetingSource::Microphone)))
        }
        "them" | "others" | "other" | "remote" | "system" | "computer" | "call" | "call audio" => {
            Ok(Some(SpeakerFilter::Source(MeetingSource::System)))
        }
        other if crate::meeting::speakers::is_valid_name(speaker.trim()) => {
            Ok(Some(SpeakerFilter::Named(other.into())))
        }
        other => Err(eyre!(
            "Unknown speaker {other:?}; use \"you\" or \"room mic\", \"them\" or \"call audio\", \"all\", or a participant's name."
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_recorded_name_wins_over_a_legacy_source_alias() {
        for name in ["Kit", "Local", "Computer", "Other"] {
            let entries = [TranscriptEntry {
                mode: Default::default(),
                source: MeetingSource::System,
                start_ms: 0,
                end_ms: 1_000,
                text: "Remote words".into(),
                speaker: Some(name.into()),
            }];
            let filter = parse_speaker(name, &entries).unwrap().unwrap();
            assert_eq!(filter, SpeakerFilter::Named(fold(name)));
            assert!(filter.matches(&entries[0]));
        }
        assert_eq!(
            parse_speaker("Kit", &[]).unwrap(),
            Some(SpeakerFilter::Source(MeetingSource::Microphone))
        );
        for canonical in ["you", "them", "all"] {
            assert!(!matches!(
                parse_speaker(canonical, &[]).unwrap(),
                Some(SpeakerFilter::Named(_))
            ));
        }
    }
}
