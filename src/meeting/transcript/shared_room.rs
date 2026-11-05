//! Labs › Shared room meetings: the people in the room speak into the
//! microphone directly, and the call (through another computer in the room)
//! can carry the same speech back as system audio a moment later. The signal
//! then calls the microphone copy local speech, so echo suppression keeps
//! both. This pass drops a microphone utterance only when the call audio
//! nearby says the same words in the same order, and keeps the call copy.
//!
//! It is deliberately narrow, because a false match deletes room speech:
//! - Ordered alignment, not a bag of words: every microphone word must match
//!   a call word exactly (case and punctuation aside), in order, with only
//!   the [`TOLERATED`] fillers and articles allowed to differ, at most one
//!   per six words, rounded up. A changed number, name, negation (`can` against
//!   `can't`), or any added word keeps the microphone utterance. Numeric
//!   signs, decimal points and attached units remain comparison content.
//! - Adjacent room lines within 700 ms are judged together, and matches
//!   must span complete call entries. A context-free suffix never cancels
//!   a differently qualified statement. Long or partly matching utterances
//!   stay twice rather than losing their unmatched context.
//! - Utterances under [`MIN_WORDS`] words, or with fewer than
//!   [`MIN_CONTENT_WORDS`] words outside [`TOLERATED`], are always kept:
//!   "sounds good" said in the room and on the call can be two people.
//! - One-to-one: each call word can stand behind one microphone utterance,
//!   so a phrase the room says twice while the call carries it once keeps
//!   one room copy. Two copies within one source are never compared.
//! - Bounded in time: the call copy must start within [`CALL_LEAD_MS`]
//!   before to [`CALL_LAG_MS`] after the microphone utterance. Entries carry
//!   only sentence start and end times, so call words are placed by
//!   spreading each entry's words evenly across it; the window allows for
//!   that and for call latency. These bounds are estimates chosen for the
//!   coarse timestamps, not measured on a real room.
//! - Bounded work: each sentence aligns only against the call words in its
//!   window, found by binary search, so a live refresh stays linear in the
//!   meeting's length.

use super::super::{MeetingSource, TranscriptEntry};

/// How long after the room hears a sentence the call audio may carry it.
pub(in crate::meeting) const CALL_LAG_MS: u64 = 4_000;
/// How long before the microphone the call audio may carry the same words:
/// a remote speaker the room's loudspeaker plays, and timestamp slack.
const CALL_LEAD_MS: u64 = 2_000;
const MIN_WORDS: usize = 5;
const MIN_CONTENT_WORDS: usize = 4;
/// Longer sentences are kept rather than aligned.
const MAX_WORDS: usize = 120;
/// Wait for this much settled microphone audio beyond an utterance before
/// publishing it, so a following qualification cannot extend it later.
pub(super) const ROOM_GAP_MS: u64 = 700;
/// Words either copy may add or drop without changing what was said.
const TOLERATED: &[&str] = &[
    "a", "ah", "an", "and", "er", "erm", "hmm", "mhm", "mm", "oh", "so", "the", "uh", "um", "uhm",
];

fn tolerated(word: &str) -> bool {
    TOLERATED.contains(&word)
}

/// Lower-cased words, keeping apostrophes inside a word, so `can't` never
/// reads as `can`.
fn words(text: &str) -> Vec<String> {
    let characters = text.chars().collect::<Vec<_>>();
    let mut words = Vec::new();
    let mut word = String::new();
    for (index, &character) in characters.iter().enumerate() {
        let next_digit = characters.get(index + 1).is_some_and(char::is_ascii_digit);
        let previous_digit = index > 0 && characters[index - 1].is_ascii_digit();
        let number_mark = match character {
            '-' | '+' | '−' => {
                next_digit
                    || (characters.get(index + 1) == Some(&'.')
                        && characters.get(index + 2).is_some_and(char::is_ascii_digit))
            }
            '.' | ',' => next_digit,
            '%' | '$' | '£' | '€' => next_digit || previous_digit,
            _ => false,
        };
        if character.is_alphanumeric() || matches!(character, '\'' | '’') || number_mark {
            word.push(match character {
                '’' => '\'',
                '−' => '-',
                other => other,
            });
        } else if !word.is_empty() {
            words.push(std::mem::take(&mut word));
        }
    }
    if !word.is_empty() {
        words.push(word);
    }
    words
        .into_iter()
        .map(|word| word.trim_matches('\'').to_lowercase())
        .filter(|word| !word.is_empty())
        .collect()
}

/// One call-audio word and where it probably was.
struct CallWord {
    word: String,
    at_ms: u64,
    first: bool,
    last: bool,
}

/// Marks the microphone entries the call audio repeats as `dropped` and
/// returns how many it marked. Entries already dropped are left alone, and
/// call-audio entries are never dropped.
pub(super) fn drop_call_repeats(entries: &[TranscriptEntry], dropped: &mut [bool]) -> u64 {
    let mut call = entries
        .iter()
        .filter(|entry| entry.source == MeetingSource::System)
        .flat_map(|entry| {
            let words = words(&entry.text);
            let count = words.len() as u64;
            let span = entry.end_ms.saturating_sub(entry.start_ms);
            words
                .into_iter()
                .enumerate()
                .map(move |(index, word)| CallWord {
                    word,
                    at_ms: entry.start_ms + span * (2 * index as u64 + 1) / (2 * count),
                    first: index == 0,
                    last: index as u64 + 1 == count,
                })
        })
        .collect::<Vec<_>>();
    call.sort_by_key(|word| word.at_ms);
    let mut used = vec![false; call.len()];
    let mut room = (0..entries.len())
        .filter(|&index| entries[index].source == MeetingSource::Microphone && !dropped[index])
        .collect::<Vec<_>>();
    room.sort_by_key(|&index| entries[index].start_ms);
    let mut groups: Vec<Vec<usize>> = Vec::new();
    for index in room {
        if let Some(group) = groups.last_mut()
            && entries[index]
                .start_ms
                .saturating_sub(entries[*group.last().unwrap()].end_ms)
                <= ROOM_GAP_MS
        {
            group.push(index);
        } else {
            groups.push(vec![index]);
        }
    }
    let mut count = 0;
    for group in groups {
        // Match the complete room utterance. Trying individual suffixes
        // after a failure could discard a qualification split across lines.
        let spoken = group
            .iter()
            .flat_map(|&index| words(&entries[index].text))
            .collect::<Vec<_>>();
        let start_ms = entries[group[0]].start_ms;
        let end_ms = group
            .iter()
            .map(|&index| entries[index].end_ms)
            .max()
            .unwrap();
        if let Some((start, end)) = matching_span(&spoken, start_ms, end_ms, &call, &used) {
            used[start..=end].fill(true);
            for index in group {
                dropped[index] = true;
                count += 1;
            }
        }
    }
    count
}

fn matching_span(
    spoken: &[String],
    start_ms: u64,
    end_ms: u64,
    call: &[CallWord],
    used: &[bool],
) -> Option<(usize, usize)> {
    let content = spoken.iter().filter(|word| !tolerated(word)).count();
    if spoken.len() < MIN_WORDS || content < MIN_CONTENT_WORDS || spoken.len() > MAX_WORDS {
        return None;
    }
    let first = call.partition_point(|word| word.at_ms < start_ms.saturating_sub(CALL_LEAD_MS));
    let last = call.partition_point(|word| word.at_ms <= end_ms + CALL_LAG_MS);
    if last <= first || last - first > 4 * spoken.len() + 64 {
        return None;
    }
    let in_time = |start: usize, end: usize| {
        let near = |at: u64, mark: u64| {
            mark.saturating_sub(CALL_LEAD_MS) <= at && at <= mark + CALL_LAG_MS
        };
        call[start].first
            && call[end].last
            && near(call[start].at_ms, start_ms)
            && near(call[end].at_ms, end_ms)
    };
    let mut run_start = first;
    while run_start < last {
        if used[run_start] {
            run_start += 1;
            continue;
        }
        let run_end = (run_start..last).find(|&at| used[at]).unwrap_or(last);
        let run = &call[run_start..run_end];
        if let Some((start, end)) = align(spoken, run, |start, end| {
            in_time(run_start + start, run_start + end)
        })
        .map(|(start, end)| (run_start + start, run_start + end))
        {
            return Some((start, end));
        }
        run_start = run_end;
    }
    None
}

/// Where `spoken` appears in `call` as one stretch, in order, differing only
/// by tolerated words: the first and last call word of the closest match
/// that `accept` (the time bound) admits.
fn align(
    spoken: &[String],
    call: &[CallWord],
    accept: impl Fn(usize, usize) -> bool,
) -> Option<(usize, usize)> {
    let allowed = spoken.len().div_ceil(6);
    // row[j]: the fewest edits, and the first call word, aligning the spoken
    // words so far with a stretch of call words ending before word `j`.
    let mut previous = (0..=call.len()).map(|j| Some((0, j))).collect::<Vec<_>>();
    let mut current = vec![None; call.len() + 1];
    for word in spoken {
        for j in 0..=call.len() {
            // The room said a filler the call copy lacks.
            let mut best = previous[j]
                .filter(|_| tolerated(word))
                .map(|(edits, start)| (edits + 1, start));
            if j > 0 {
                let heard = &call[j - 1].word;
                let matched = previous[j - 1].filter(|_| heard == word);
                // The call copy has a filler the room's lacks.
                let extra = current[j - 1]
                    .filter(|_| tolerated(heard))
                    .map(|(edits, start)| (edits + 1, start));
                for candidate in [matched, extra].into_iter().flatten() {
                    if best.is_none_or(|(edits, _)| candidate.0 < edits) {
                        best = Some(candidate);
                    }
                }
            }
            current[j] = best.filter(|&(edits, _)| edits <= allowed);
        }
        std::mem::swap(&mut previous, &mut current);
    }
    (1..=call.len())
        .filter_map(|j| previous[j].map(|(edits, start)| (edits, start, j)))
        .filter(|&(_, start, end)| start < end && accept(start, end - 1))
        .min_by_key(|&(edits, start, end)| (edits, end - start))
        .map(|(_, start, end)| (start, end - 1))
}

#[cfg(test)]
#[path = "shared_room_tests.rs"]
mod tests;
