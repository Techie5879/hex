//! Reconcile coarse model segments without using their start alone to drop text.
use super::*;

impl TrackTranscript {
    pub(super) fn accept_segments(
        &mut self,
        chunk: &Chunk,
        chunk_ms: u64,
        duration_ms: u64,
        transcript: Transcript,
        previous_settled: u64,
    ) {
        // A model without word timings (Apple Speech) took over mid-track.
        self.finish();
        let segments = transcript.segments;
        // A model that gives no usable time range for a segment (Cohere
        // reports one segment at zero) keeps its text over the whole chunk.
        let timed = |segment: &TranscriptSegment| {
            let start = (segment.start_ms.max(0) as u64).min(duration_ms);
            let end = (segment.end_ms.max(0) as u64).min(duration_ms);
            segment.text.trim().is_empty() || end > start
        };
        let segmented = !segments.is_empty() && segments.iter().all(timed);
        let text = if segmented || !transcript.text.trim().is_empty() {
            transcript.text
        } else {
            segments
                .iter()
                .map(|segment| segment.text.trim())
                .filter(|text| !text.is_empty())
                .collect::<Vec<_>>()
                .join(" ")
        };
        let mut incoming: Vec<TranscriptEntry> = if segmented {
            segments
                .into_iter()
                .filter_map(|segment| {
                    let text = segment.text.trim().to_string();
                    let start = (segment.start_ms.max(0) as u64).min(duration_ms);
                    let end = (segment.end_ms.max(0) as u64).min(duration_ms);
                    (!text.is_empty() && end > start).then(|| TranscriptEntry {
                        source: self.source,
                        start_ms: chunk_ms + start,
                        end_ms: chunk_ms + end,
                        text,
                        speaker: None,
                    })
                })
                .collect()
        } else {
            let text = text.trim();
            (!text.is_empty())
                .then(|| TranscriptEntry {
                    source: self.source,
                    start_ms: chunk_ms,
                    end_ms: chunk_ms + duration_ms,
                    text: text.into(),
                    speaker: None,
                })
                .into_iter()
                .collect()
        };
        if chunk.overlap > 0 {
            reconcile(&mut self.entries, &mut incoming, chunk_ms, previous_settled);
        }
        self.entries.extend(incoming);
    }
}

fn words(text: &str) -> Vec<String> {
    text.split_whitespace().map(normalize_word).collect()
}

fn protected(word: &str) -> bool {
    word.chars().any(|c| c.is_ascii_digit())
        || matches!(
            word,
            "no" | "not"
                | "never"
                | "without"
                | "cannot"
                | "cant"
                | "dont"
                | "didnt"
                | "wont"
                | "zero"
                | "one"
                | "two"
                | "three"
                | "four"
                | "five"
                | "six"
                | "seven"
                | "eight"
                | "nine"
                | "ten"
                | "eleven"
                | "twelve"
                | "thirteen"
                | "fourteen"
                | "fifteen"
                | "sixteen"
                | "seventeen"
                | "eighteen"
                | "nineteen"
                | "twenty"
                | "thirty"
                | "forty"
                | "fifty"
                | "sixty"
                | "seventy"
                | "eighty"
                | "ninety"
                | "hundred"
                | "thousand"
        )
}

fn replacement(old: &TranscriptEntry, new: &TranscriptEntry) -> bool {
    if old.start_ms >= new.end_ms || new.start_ms >= old.end_ms {
        return false;
    }
    let a = words(&old.text);
    let b = words(&new.text);
    if a.is_empty() || b.is_empty() {
        return false;
    }
    // Never infer a revision which loses a qualifier or changes a number.
    if a.iter().filter(|s| protected(s)).any(|s| !b.contains(s)) {
        return false;
    }
    if a.len() <= b.len() && b.windows(a.len()).any(|part| part == a) {
        return true;
    }
    // A partial utterance at precisely the same onset is superseded by the
    // longer observation, even if the model revises a name's spelling.
    if !old.text.trim_end().ends_with(['.', '!', '?'])
        && (2..=3).contains(&a.len())
        && b.len() > a.len()
        && a[1..] == b[1..a.len()]
        && !protected(&a[0])
        && old.start_ms.abs_diff(new.start_ms) <= 300
    {
        return true;
    }
    // A singular/plural revision of the same multiword sentence is not a
    // second utterance. Require aligned onset and all other words unchanged.
    a.len() >= 4
        && a.len() == b.len()
        && old.start_ms.abs_diff(new.start_ms) <= 300
        && a.iter().zip(&b).all(|(a, b)| {
            a == b
                || (!protected(a)
                    && !protected(b)
                    && a.trim_end_matches('s') == b.trim_end_matches('s'))
        })
}

fn reconcile(
    previous: &mut Vec<TranscriptEntry>,
    incoming: &mut Vec<TranscriptEntry>,
    overlap_start: u64,
    previous_settled: u64,
) {
    previous.retain(|old| {
        old.end_ms <= previous_settled || !incoming.iter().any(|new| replacement(old, new))
    });
    // Search the complete recent stable tail, not just its last segment:
    // decoders can combine a repeated tail and several new sentences.
    let recent = previous
        .iter()
        .filter(|e| e.end_ms > overlap_start)
        .map(|e| e.text.as_str())
        .collect::<Vec<_>>()
        .join(" ");
    if let Some(first) = incoming.first_mut() {
        first.text = strip_repeated_prefix(&recent, &first.text);
    }
    incoming.retain(|e| !e.text.is_empty());
}

/// Ignore a long nonspeech tail in coarse native timestamps, without
/// trimming the input or removing any recognized text. Keep 200 ms context;
/// small differences stay approximate rather than pretending word alignment.
pub(super) fn bound_end(transcript: &mut Transcript, speech_end: Option<usize>) {
    let Some(end) = speech_end else {
        return;
    };
    let end_ms = samples_ms(end.saturating_add(RATE / 5)) as i64;
    for segment in &mut transcript.segments {
        if segment.end_ms.saturating_sub(end_ms) > 1000 && segment.start_ms < end_ms {
            segment.end_ms = end_ms;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn entry(start: u64, end: u64, text: &str) -> TranscriptEntry {
        TranscriptEntry {
            source: MeetingSource::System,
            start_ms: start,
            end_ms: end,
            text: text.into(),
            speaker: None,
        }
    }
    #[test]
    fn nonspeech_tail_does_not_extend_a_coarse_sentence() {
        let mut transcript = Transcript {
            text: "Keep all 12 files, not 10.".into(),
            segments: vec![TranscriptSegment {
                start_ms: 0,
                end_ms: 8760,
                text: "Keep all 12 files, not 10.".into(),
            }],
            ..Default::default()
        };
        bound_end(&mut transcript, Some(2240 * RATE / 1000));
        assert_eq!(transcript.segments[0].end_ms, 2440);
        assert_eq!(transcript.text, "Keep all 12 files, not 10.");
        assert_eq!(transcript.segments[0].text, transcript.text);
        bound_end(&mut transcript, None);
        assert_eq!(transcript.segments[0].end_ms, 2440);
    }

    #[test]
    fn revised_partial_name_does_not_leave_an_orphan() {
        let mut old = vec![entry(44800, 45460, "Mahia will")];
        let mut new = vec![entry(44800, 50980, "Myha will check the budget on Friday.")];
        reconcile(&mut old, &mut new, 43460, 44460);
        assert!(old.is_empty());
        assert_eq!(new.len(), 1);
    }
    #[test]
    fn combined_segment_matches_the_full_recent_tail() {
        let mut old = vec![
            entry(25444, 27824, "Keep the original deadline."),
            entry(27824, 28904, "Assign the review to Morgan."),
            entry(28904, 30024, "The reference number"),
        ];
        let mut new = vec![entry(
            28024,
            32104,
            "Review to Morgan. The reference number is 42, not 43.",
        )];
        reconcile(&mut old, &mut new, 28000, 29000);
        assert_eq!(old.len(), 2);
        assert_eq!(new[0].text, "The reference number is 42, not 43.");
    }
    #[test]
    fn singular_plural_revision_replaces_only_unpublished_sentence() {
        let mut old = vec![entry(68064, 70024, "Morgan will confirm the final plans.")];
        let mut new = vec![entry(68024, 76784, "Morgan will confirm the final plan.")];
        reconcile(&mut old, &mut new, 68000, 69000);
        assert!(old.is_empty());
    }
    #[test]
    fn revision_cannot_discard_negation_or_change_a_number() {
        assert!(!replacement(
            &entry(0, 1000, "Do not send"),
            &entry(0, 2000, "Do send the report.")
        ));
        assert!(!replacement(
            &entry(0, 1000, "Keep 12"),
            &entry(0, 2000, "Keep 20 files.")
        ));
        assert!(!replacement(
            &entry(0, 1000, "Keep twelve"),
            &entry(0, 2000, "Keep twenty files.")
        ));
    }
}
