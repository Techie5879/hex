//! Word-level echo suppression: which microphone utterances repeat the call
//! audio, given what the signal says about each span.

use super::super::echo::Signal;
use super::super::{MeetingMode, MeetingSource, TranscriptEntry};
use super::{ECHO_SIMILARITY, shared_room, tokens, tokens_match};

const ECHO_WINDOW_MS: u64 = 1_500;
/// Word-level segments closer than this are judged together.
const UTTERANCE_GAP_MS: u64 = 700;
/// Entries with at least this many words are also judged on their own.
const SENTENCE_WORDS: usize = 3;

/// Drops microphone utterances that echo remote speech, which happens when
/// the call plays through speakers. An utterance is echo when the signal
/// says delayed system audio explains it, or when its words repeat
/// overlapping remote speech unless the signal shows local speech
/// (double-talk). `signal` classifies a microphone time span. Returns the
/// dropped count.
///
/// In a shared room the microphone also hears people the call carries
/// again, so words alone never call a span echo there: only the signal's
/// bleed does, and then [`shared_room::drop_call_repeats`] drops sentences
/// the call audio repeats word for word, whatever the signal says.
pub(super) fn suppress_echo(
    entries: Vec<TranscriptEntry>,
    mode: MeetingMode,
    signal: impl Fn(u64, u64) -> Option<Signal>,
) -> (Vec<TranscriptEntry>, u64) {
    let system = entries
        .iter()
        .filter(|entry| entry.source == MeetingSource::System)
        .collect::<Vec<_>>();
    let mut microphone = entries
        .iter()
        .enumerate()
        .filter(|(_, entry)| entry.source == MeetingSource::Microphone)
        .collect::<Vec<_>>();
    microphone.sort_by_key(|(_, entry)| entry.start_ms);
    let mut utterances: Vec<Vec<(usize, &TranscriptEntry)>> = Vec::new();
    for (index, entry) in microphone {
        match utterances.last_mut() {
            Some(utterance)
                if entry
                    .start_ms
                    .saturating_sub(utterance.last().unwrap().1.end_ms)
                    <= UTTERANCE_GAP_MS =>
            {
                utterance.push((index, entry));
            }
            _ => utterances.push(vec![(index, entry)]),
        }
    }
    let is_echo = |spoken: &[String], start_ms: u64, end_ms: u64, judge_words: bool| match signal(
        start_ms, end_ms,
    ) {
        Some(Signal::Bleed) => true,
        Some(Signal::Local) => false,
        None => {
            mode.is_standard()
                && judge_words
                && echo_score(spoken, start_ms, end_ms, &system) >= ECHO_SIMILARITY
        }
    };
    let mut dropped = vec![false; entries.len()];
    let mut dropped_utterances = 0;
    for utterance in utterances {
        let spoken = utterance
            .iter()
            .flat_map(|(_, entry)| tokens(&entry.text))
            .collect::<Vec<_>>();
        let end = utterance
            .iter()
            .map(|(_, entry)| entry.end_ms)
            .max()
            .unwrap_or_default();
        if is_echo(&spoken, utterance[0].1.start_ms, end, true) {
            dropped_utterances += 1;
            // Bleed can run straight into the user's own sentence; a sentence
            // the signal shows is local survives its echoing neighbours.
            for (index, entry) in utterance {
                dropped[index] = tokens(&entry.text).len() < SENTENCE_WORDS
                    || signal(entry.start_ms, entry.end_ms) != Some(Signal::Local);
            }
            continue;
        }
        if utterance.len() == 1 {
            continue;
        }
        for (index, entry) in utterance {
            let spoken = tokens(&entry.text);
            if is_echo(
                &spoken,
                entry.start_ms,
                entry.end_ms,
                spoken.len() >= SENTENCE_WORDS,
            ) {
                dropped_utterances += 1;
                dropped[index] = true;
            }
        }
    }
    if mode == MeetingMode::SharedRoom {
        dropped_utterances += shared_room::drop_call_repeats(&entries, &mut dropped);
    }
    let kept = entries
        .into_iter()
        .zip(dropped)
        .filter_map(|(entry, dropped)| (!dropped).then_some(entry))
        .collect();
    (kept, dropped_utterances)
}

/// The share of `spoken` that remote speech within the echo window contains.
fn echo_score(spoken: &[String], start_ms: u64, end_ms: u64, remote: &[&TranscriptEntry]) -> f32 {
    let start = start_ms.saturating_sub(ECHO_WINDOW_MS);
    let end = end_ms + ECHO_WINDOW_MS;
    let remote = remote
        .iter()
        .filter(|entry| entry.start_ms < end && start < entry.end_ms)
        .flat_map(|entry| tokens(&entry.text))
        .collect::<Vec<_>>();
    if remote.is_empty() || spoken.is_empty() {
        return 0.0;
    }
    let matched = spoken
        .iter()
        .filter(|token| remote.iter().any(|remote| tokens_match(token, remote)))
        .count();
    matched as f32 / spoken.len() as f32
}
