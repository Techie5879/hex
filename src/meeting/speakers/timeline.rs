//! Who the call app showed as speaking, on the recording's own timeline, and
//! the one rule that puts a name on a transcript line.
//!
//! Observations live in `speakers.ndjson` beside the manifest (owner-only,
//! bounded, moved to the Trash with its meeting), so the in-call transcript,
//! the final one, and a final pass after a restart all read the same evidence.
//! Times are what a system-audio entry calls that moment (see
//! [`ClockSync`]), so a name is matched against the audio the entry came from
//! rather than against the wall clock.

use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};

use serde::{Deserialize, Serialize};

use super::Provider;
use super::names;
use crate::meeting::{MeetingSource, TranscriptEntry};

pub const FILE: &str = "speakers.ndjson";
/// How often the worker reads the call app.
pub const POLL_MS: u64 = 500;
/// A reading is stored at least this often while it stays the same, so a
/// crash loses at most this much evidence.
const HEARTBEAT_MS: u64 = 2_000;
/// About 8 hours of a new speaker every second.
const MAX_BYTES: u64 = 4 * 1024 * 1024;
/// Readings further apart than this are a gap, not one stretch: nothing is
/// assumed about the time between them. Two polls plus a slow read.
const MAX_SAMPLE_GAP_MS: u64 = 1_500;
/// How long after someone starts talking the app marks them. The sources
/// disagree (500 ms to 1.5 s) and none measured it against audio, so this is
/// an initial value. Incorrect timing can leave a line unnamed or attribute
/// it to the previous speaker, so native audio-aligned checks are required.
const ONSET_LAG_MS: u64 = 500;
/// How long a mark outlasts the voice: Slack's was measured by a third party
/// at about 1.5 s; Teams' is unmeasured.
const RELEASE_MS: u64 = 1_500;
const RELEASE_TOLERANCE_MS: u64 = 500;
/// The ends of a line, where the previous and next speaker's marks can lie.
const EDGE_TRIM_MS: u64 = 400;
/// The share of a line the one speaker must be marked for.
const MIN_SPEAKING_PERCENT: u64 = 60;

/// Maps the capture clock onto the system track's own timeline, which is what
/// transcript entries are stamped with: `first arrival + samples / rate`.
/// The two drift apart when packets drop or the device clock runs slow, by up
/// to the point where the writer pads a gap, so a speaker mark's capture time
/// alone can be most of a second away from its audio.
#[derive(Debug)]
pub struct ClockSync {
    origin_ms: AtomicU64,
    skew_ms: AtomicI64,
}

impl Default for ClockSync {
    fn default() -> Self {
        Self {
            origin_ms: AtomicU64::new(u64::MAX),
            skew_ms: AtomicI64::new(0),
        }
    }
}

impl ClockSync {
    /// A system packet ending at `samples` of the track arrived at
    /// `arrival_us`; the track's first packet arrived at `first_arrival_us`.
    pub(in crate::meeting) fn observe(
        &self,
        first_arrival_us: u64,
        arrival_us: u64,
        samples: u64,
        sample_rate: u32,
    ) {
        let origin_ms = first_arrival_us / 1_000;
        let position_ms = samples.saturating_mul(1_000) / u64::from(sample_rate.max(1));
        let skew = origin_ms.saturating_add(position_ms) as i64 - (arrival_us / 1_000) as i64;
        self.skew_ms.store(skew, Ordering::Release);
        self.origin_ms.store(origin_ms, Ordering::Release);
    }

    /// The system track's origin and `capture_ms` on its timeline; `None`
    /// until its audio starts, when there is nothing to attribute.
    pub(super) fn track_time(&self, capture_ms: u64) -> Option<(u64, u64)> {
        let origin_ms = self.origin_ms.load(Ordering::Acquire);
        (origin_ms != u64::MAX).then(|| {
            let at = capture_ms as i64 + self.skew_ms.load(Ordering::Acquire);
            (origin_ms, at.max(0) as u64)
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Observed {
    /// One remote participant, by name, was marked as speaking.
    Speaking(String),
    /// A verified call marked no remote participant.
    Silent,
    /// Unreadable or contradictory: nothing may be assumed for this time.
    Uncertain,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(untagged)]
enum Line {
    Header {
        version: u8,
        provider: Provider,
        system_origin_ms: u64,
    },
    Run {
        from_ms: u64,
        to_ms: u64,
        binding: u32,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        speaking: Option<String>,
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        silent: bool,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct Run {
    from_ms: u64,
    to_ms: u64,
    binding: u32,
    state: Observed,
}

impl Run {
    fn line(&self) -> Line {
        Line::Run {
            from_ms: self.from_ms,
            to_ms: self.to_ms,
            binding: self.binding,
            speaking: match &self.state {
                Observed::Speaking(name) => Some(name.clone()),
                _ => None,
            },
            silent: self.state == Observed::Silent,
        }
    }
}

/// Appends observations to a meeting's file, one line at a time.
pub struct Recorder {
    path: PathBuf,
    provider: Provider,
    file: Option<File>,
    written: u64,
    pending: Option<Run>,
}

impl Recorder {
    pub fn new(directory: &Path, provider: Provider) -> Self {
        Self {
            path: directory.join(FILE),
            provider,
            file: None,
            written: 0,
            pending: None,
        }
    }

    /// Records a reading at `at_ms` on the system track's timeline.
    pub fn observe(
        &mut self,
        origin_ms: u64,
        at_ms: u64,
        binding: u32,
        state: Observed,
    ) -> std::io::Result<()> {
        if self.file.is_none() {
            self.open(origin_ms)?;
        }
        match &mut self.pending {
            Some(run)
                if run.binding == binding
                    && run.state == state
                    && at_ms >= run.to_ms
                    && at_ms - run.from_ms < HEARTBEAT_MS =>
            {
                run.to_ms = at_ms;
                return Ok(());
            }
            // The track's clock can step back a little when it recovers; the
            // time already recorded stands and this reading adds nothing.
            Some(run) if at_ms < run.to_ms => return Ok(()),
            _ => self.flush()?,
        }
        self.pending = Some(Run {
            from_ms: at_ms,
            to_ms: at_ms,
            binding,
            state,
        });
        Ok(())
    }

    fn open(&mut self, origin_ms: u64) -> std::io::Result<()> {
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)?;
        crate::meeting::set_owner_only(&self.path, false)
            .map_err(|error| std::io::Error::other(error.to_string()))?;
        self.written = file.metadata()?.len();
        let mut line = serde_json::to_vec(&Line::Header {
            version: 1,
            provider: self.provider,
            system_origin_ms: origin_ms,
        })?;
        line.push(b'\n');
        file.write_all(&line)?;
        self.written += line.len() as u64;
        self.file = Some(file);
        Ok(())
    }

    /// Writes the reading in progress, so the transcript can use it.
    pub fn flush(&mut self) -> std::io::Result<()> {
        let (Some(file), Some(run)) = (&mut self.file, self.pending.take()) else {
            return Ok(());
        };
        let mut line = serde_json::to_vec(&run.line())?;
        line.push(b'\n');
        if self.written + line.len() as u64 > MAX_BYTES {
            return Err(std::io::Error::other("speaker observation limit reached"));
        }
        file.write_all(&line)?;
        self.written += line.len() as u64;
        Ok(())
    }

    /// Flushes and syncs at the end of the recording, for its final pass.
    pub fn finish(mut self) -> std::io::Result<()> {
        self.flush()?;
        if let Some(file) = &self.file {
            file.sync_all()?;
        }
        Ok(())
    }
}

/// A meeting's observations in time order.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Timeline {
    origin_ms: Option<u64>,
    runs: Vec<Run>,
}

impl Timeline {
    /// Everything stored for a meeting; an absent or damaged file reads as
    /// no evidence, so every line stays `Them`.
    pub fn load(directory: &Path) -> Self {
        let mut follower = Follower::new(directory);
        follower.refresh();
        follower.timeline
    }

    fn push_line(&mut self, line: &[u8]) {
        let Ok(line) = serde_json::from_slice::<Line>(line) else {
            return;
        };
        match line {
            Line::Header {
                system_origin_ms, ..
            } => {
                // A second origin means two recordings' clocks in one file.
                if *self.origin_ms.get_or_insert(system_origin_ms) != system_origin_ms {
                    self.runs.clear();
                    self.origin_ms = Some(u64::MAX);
                }
            }
            Line::Run {
                from_ms,
                to_ms,
                binding,
                speaking,
                silent,
            } if from_ms <= to_ms => {
                let state = match speaking {
                    Some(name) if names::is_valid(&name) => Observed::Speaking(name),
                    Some(_) => Observed::Uncertain,
                    None if silent => Observed::Silent,
                    None => Observed::Uncertain,
                };
                self.push(Run {
                    from_ms,
                    to_ms,
                    binding,
                    state,
                });
            }
            Line::Run { .. } => {}
        }
    }

    fn push(&mut self, run: Run) {
        match self.runs.last_mut() {
            Some(last) if run.from_ms < last.to_ms => {}
            Some(last)
                if last.binding == run.binding
                    && last.state == run.state
                    && run.from_ms - last.to_ms <= MAX_SAMPLE_GAP_MS =>
            {
                last.to_ms = run.to_ms;
            }
            _ => self.runs.push(run),
        }
    }

    /// The end of what run `index` tells us: until the next reading, and at
    /// most one poll past its last one. Never further.
    fn known_until(&self, index: usize) -> u64 {
        let run = &self.runs[index];
        let limit = run.to_ms + POLL_MS;
        self.runs
            .get(index + 1)
            .map_or(limit, |next| limit.min(next.from_ms))
    }

    /// The one person this timeline positively shows speaking the audio from
    /// `start_ms` to `end_ms` on the system track's timeline. Anything less
    /// than that is `None`: a gap in the readings, a changed window or
    /// session, an uncertain reading, or two people; too little of the line
    /// marked; or, for a line shorter than a mark lasts, a mark left over
    /// from before the line.
    pub fn speaker_for(&self, start_ms: u64, end_ms: u64) -> Option<&str> {
        let duration = end_ms.saturating_sub(start_ms);
        let trim = (duration / 4).min(EDGE_TRIM_MS);
        let from = start_ms + ONSET_LAG_MS + trim;
        let to = (start_ms + duration + ONSET_LAG_MS - trim).max(from + 1);
        let first = self
            .runs
            .iter()
            .enumerate()
            .position(|(index, _)| self.known_until(index) > from)?;
        if self.runs[first].from_ms > from {
            return None;
        }
        let binding = self.runs[first].binding;
        let mut speaker: Option<&Run> = None;
        // Where the marked person's last stretch in the line ends.
        let mut marked_until = 0;
        let mut marked_ms = 0;
        let mut covered_until = from;
        let mut previous_to = self.runs[first].from_ms;
        for (index, run) in self.runs.iter().enumerate().skip(first) {
            if run.from_ms >= to {
                break;
            }
            if run.binding != binding || run.from_ms - previous_to > MAX_SAMPLE_GAP_MS {
                return None;
            }
            let until = self.known_until(index);
            let overlap = until.min(to).saturating_sub(run.from_ms.max(from));
            match &run.state {
                Observed::Uncertain => return None,
                Observed::Silent => {}
                Observed::Speaking(_) => {
                    if speaker.is_some_and(|known| known.state != run.state) {
                        return None;
                    }
                    speaker = speaker.or(Some(run));
                    marked_until = run.to_ms;
                    marked_ms += overlap;
                }
            }
            covered_until = until;
            previous_to = run.to_ms;
        }
        if covered_until < to || marked_ms * 100 < (to - from) * MIN_SPEAKING_PERCENT {
            return None;
        }
        let speaker = speaker?;
        // A mark that began before the line and ends as soon as a lingering
        // one would is a previous speaker's, whoever spoke this short line.
        if duration < RELEASE_MS
            && speaker.from_ms < start_ms
            && marked_until + RELEASE_TOLERANCE_MS < end_ms + RELEASE_MS
        {
            return None;
        }
        match &speaker.state {
            Observed::Speaking(name) => Some(name),
            _ => None,
        }
    }

    /// Names the system-audio entries this timeline can identify. Entries
    /// are on the meeting's timeline, `meeting_origin_ms` after the capture
    /// clock's zero. Nothing is named unless the file's clock is this
    /// recording's system track, `system_origin_ms`. Microphone entries and
    /// everything else stay exactly as they are.
    pub fn label(
        &self,
        entries: &mut [TranscriptEntry],
        meeting_origin_ms: u64,
        system_origin_ms: Option<u64>,
    ) {
        if self.runs.is_empty() || self.origin_ms != system_origin_ms {
            return;
        }
        for entry in entries
            .iter_mut()
            .filter(|entry| entry.source == MeetingSource::System)
        {
            entry.speaker = self
                .speaker_for(
                    entry.start_ms + meeting_origin_ms,
                    entry.end_ms + meeting_origin_ms,
                )
                .map(str::to_owned);
        }
    }
}

/// Reads a growing file incrementally, for the in-call transcript.
pub struct Follower {
    path: PathBuf,
    offset: u64,
    timeline: Timeline,
}

impl Follower {
    pub fn new(directory: &Path) -> Self {
        Self {
            path: directory.join(FILE),
            offset: 0,
            timeline: Timeline::default(),
        }
    }

    /// Takes in complete lines written since the last call.
    pub fn refresh(&mut self) -> &Timeline {
        let Ok(mut file) = File::open(&self.path) else {
            return &self.timeline;
        };
        let length = fs::metadata(&self.path).map_or(0, |metadata| metadata.len());
        if length <= self.offset || self.offset >= MAX_BYTES {
            return &self.timeline;
        }
        let mut bytes = Vec::new();
        let read = file.seek(SeekFrom::Start(self.offset)).and_then(|_| {
            Read::by_ref(&mut file)
                .take(MAX_BYTES - self.offset)
                .read_to_end(&mut bytes)
        });
        if read.is_err() {
            return &self.timeline;
        }
        let complete = bytes
            .iter()
            .rposition(|byte| *byte == b'\n')
            .map_or(0, |newline| newline + 1);
        for line in bytes[..complete].split(|byte| *byte == b'\n') {
            self.timeline.push_line(line);
        }
        self.offset += complete as u64;
        &self.timeline
    }
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;

    pub(in crate::meeting) struct TestDir(PathBuf);

    impl TestDir {
        pub(in crate::meeting) fn new() -> Self {
            use std::sync::atomic::AtomicUsize;
            static NEXT: AtomicUsize = AtomicUsize::new(0);
            let path = std::env::temp_dir().join(format!(
                "hex-speakers-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }

        pub(in crate::meeting) fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    const ADA: &str = "Ada Lovelace";
    const GRACE: &str = "Grace Hopper";

    fn speaking(name: &str) -> Observed {
        Observed::Speaking(name.into())
    }

    /// Readings every poll from `from` up to `to`.
    fn stretch(from: u64, to: u64, binding: u32, state: Observed) -> Vec<(u64, u32, Observed)> {
        (from..to)
            .step_by(POLL_MS as usize)
            .map(|at| (at, binding, state.clone()))
            .collect()
    }

    fn timeline(readings: impl IntoIterator<Item = (u64, u32, Observed)>) -> Timeline {
        let directory = TestDir::new();
        let mut recorder = Recorder::new(directory.path(), Provider::Teams);
        for (at, binding, state) in readings {
            recorder.observe(0, at, binding, state).unwrap();
        }
        recorder.finish().unwrap();
        Timeline::load(directory.path())
    }

    fn entry(source: MeetingSource, start_ms: u64, end_ms: u64) -> TranscriptEntry {
        TranscriptEntry {
            mode: Default::default(),
            source,
            start_ms,
            end_ms,
            text: "words".into(),
            speaker: None,
        }
    }

    #[test]
    fn a_line_inside_one_persons_mark_gets_their_name() {
        let timeline = timeline(stretch(0, 12_000, 1, speaking(ADA)));
        assert_eq!(timeline.speaker_for(2_000, 6_000), Some(ADA));
    }

    #[test]
    fn two_people_on_one_line_stay_unnamed_but_each_side_of_a_handoff_is_named() {
        let timeline = timeline(
            stretch(0, 6_000, 1, speaking(ADA))
                .into_iter()
                .chain(stretch(6_000, 12_000, 1, speaking(GRACE))),
        );
        assert_eq!(timeline.speaker_for(1_000, 4_500), Some(ADA));
        assert_eq!(timeline.speaker_for(6_000, 10_000), Some(GRACE));
        assert_eq!(timeline.speaker_for(3_000, 9_000), None);
    }

    #[test]
    fn the_marks_lag_is_taken_from_the_line_before_comparing() {
        // Ada's mark ends at 6 s because Grace took over; the audio Grace
        // spoke began before her mark, and Ada's last line ended before it.
        let timeline = timeline(
            stretch(0, 6_000, 1, speaking(ADA))
                .into_iter()
                .chain(stretch(6_000, 12_000, 1, speaking(GRACE))),
        );
        assert_eq!(timeline.speaker_for(5_600, 9_000), Some(GRACE));
    }

    #[test]
    fn nothing_is_carried_across_a_gap_in_the_readings() {
        let timeline = timeline(
            stretch(0, 4_000, 1, speaking(ADA))
                .into_iter()
                .chain(stretch(8_000, 12_000, 1, speaking(ADA))),
        );
        assert_eq!(timeline.speaker_for(1_000, 3_000), Some(ADA));
        assert_eq!(timeline.speaker_for(9_000, 11_000), Some(ADA));
        // The gap itself, and lines reaching into it.
        assert_eq!(timeline.speaker_for(5_000, 7_000), None);
        assert_eq!(timeline.speaker_for(3_000, 6_000), None);
        assert_eq!(timeline.speaker_for(6_000, 9_500), None);
    }

    #[test]
    fn a_gap_only_as_long_as_the_last_reading_can_vouch_for_is_filled() {
        let timeline = timeline(
            stretch(0, 3_000, 1, speaking(ADA))
                .into_iter()
                .chain(stretch(3_400, 7_000, 1, speaking(ADA))),
        );
        assert_eq!(timeline.speaker_for(1_000, 5_000), Some(ADA));
    }

    #[test]
    fn the_last_reading_does_not_vouch_for_time_after_it() {
        let timeline = timeline(stretch(0, 4_000, 1, speaking(ADA)));
        // Readings stop at 3.5 s; a line ending later has no evidence yet.
        assert_eq!(timeline.speaker_for(3_000, 9_000), None);
        assert_eq!(timeline.speaker_for(20_000, 21_000), None);
    }

    #[test]
    fn a_changed_window_ends_what_either_side_can_say_about_a_line() {
        let timeline = timeline(
            stretch(0, 5_000, 1, speaking(ADA))
                .into_iter()
                .chain(stretch(5_000, 10_000, 2, speaking(ADA))),
        );
        assert_eq!(timeline.speaker_for(1_000, 3_000), Some(ADA));
        assert_eq!(timeline.speaker_for(2_000, 8_000), None);
        assert_eq!(timeline.speaker_for(6_000, 8_500), Some(ADA));
    }

    #[test]
    fn an_uncertain_reading_inside_a_line_leaves_it_unnamed() {
        let timeline = timeline(
            stretch(0, 4_000, 1, speaking(ADA))
                .into_iter()
                .chain(stretch(4_000, 5_000, 1, Observed::Uncertain))
                .chain(stretch(5_000, 9_000, 1, speaking(ADA))),
        );
        assert_eq!(timeline.speaker_for(2_000, 7_000), None);
        assert_eq!(timeline.speaker_for(0, 3_000), Some(ADA));
    }

    #[test]
    fn mostly_silent_marks_are_not_enough() {
        // A hidden speaker talks while a tile flashes for under a second.
        let timeline = timeline(
            stretch(0, 2_000, 1, Observed::Silent)
                .into_iter()
                .chain(stretch(2_000, 3_000, 1, speaking(ADA)))
                .chain(stretch(3_000, 8_000, 1, Observed::Silent)),
        );
        assert_eq!(timeline.speaker_for(500, 6_000), None);
        // A brief dip in the mark is still that person.
        let dipped = super::tests::timeline(
            stretch(0, 3_000, 1, speaking(ADA))
                .into_iter()
                .chain(stretch(3_000, 3_500, 1, Observed::Silent))
                .chain(stretch(3_500, 8_000, 1, speaking(ADA))),
        );
        assert_eq!(dipped.speaker_for(500, 6_000), Some(ADA));
    }

    #[test]
    fn a_short_line_under_a_lingering_mark_stays_unnamed() {
        // Ada stopped at 3 s and her mark lingers to 4.5 s. Someone Hex can't
        // see says "Sure." from 3.2 to 3.7 s.
        let lingering = timeline(
            stretch(0, 4_500, 1, speaking(ADA))
                .into_iter()
                .chain(stretch(4_500, 9_000, 1, Observed::Silent)),
        );
        assert_eq!(lingering.speaker_for(3_200, 3_700), None);
        // The same short line in the middle of Ada talking is Ada's.
        let continuing = timeline(stretch(0, 9_000, 1, speaking(ADA)));
        assert_eq!(continuing.speaker_for(3_200, 3_700), Some(ADA));
        // So is a short line whose own mark starts during it.
        let answering = timeline(
            stretch(0, 3_500, 1, Observed::Silent)
                .into_iter()
                .chain(stretch(3_500, 6_000, 1, speaking(ADA)))
                .chain(stretch(6_000, 9_000, 1, Observed::Silent)),
        );
        assert_eq!(answering.speaker_for(3_200, 3_900), Some(ADA));
        // A short line just before a dip in the mark can't be told from one
        // under a lingering mark, so it stays unnamed too; the mark's last
        // stretch in the line decides, not an earlier one.
        let dipped = timeline(
            stretch(0, 2_500, 1, speaking(ADA))
                .into_iter()
                .chain(stretch(2_500, 3_500, 1, Observed::Silent))
                .chain(stretch(3_500, 9_000, 1, speaking(ADA))),
        );
        assert_eq!(dipped.speaker_for(2_000, 2_600), None);
        assert_eq!(dipped.speaker_for(2_000, 4_200), None);
        assert_eq!(dipped.speaker_for(3_600, 4_200), Some(ADA));
    }

    #[test]
    fn labelling_names_only_system_audio_and_only_on_this_recordings_clock() {
        let directory = TestDir::new();
        let mut recorder = Recorder::new(directory.path(), Provider::Slack);
        for at in (10_000..20_000).step_by(POLL_MS as usize) {
            recorder.observe(10_000, at, 1, speaking(ADA)).unwrap();
        }
        recorder.finish().unwrap();
        let timeline = Timeline::load(directory.path());
        let lines = || {
            vec![
                entry(MeetingSource::System, 1_000, 4_000),
                entry(MeetingSource::Microphone, 1_000, 4_000),
            ]
        };

        // The meeting starts 500 ms before the system track.
        let mut named = lines();
        timeline.label(&mut named, 9_500, Some(10_000));
        assert_eq!(named[0].speaker.as_deref(), Some(ADA));
        assert_eq!(named[1].speaker, None);

        // Another clock, or none, names nobody.
        for origin in [Some(10_001), None] {
            let mut unnamed = lines();
            timeline.label(&mut unnamed, 9_500, origin);
            assert_eq!(unnamed[0].speaker, None);
        }
    }

    #[test]
    fn a_second_origin_in_one_file_discards_the_evidence() {
        let directory = TestDir::new();
        for origin in [0, 5_000] {
            let mut recorder = Recorder::new(directory.path(), Provider::Teams);
            for at in (origin..origin + 5_000).step_by(POLL_MS as usize) {
                recorder.observe(origin, at, 1, speaking(ADA)).unwrap();
            }
            recorder.finish().unwrap();
        }
        let timeline = Timeline::load(directory.path());
        assert_eq!(timeline.speaker_for(1_000, 3_000), None);
        let mut lines = vec![entry(MeetingSource::System, 1_000, 3_000)];
        timeline.label(&mut lines, 0, Some(0));
        assert_eq!(lines[0].speaker, None);
    }

    #[test]
    fn a_stored_file_gives_the_same_answer_after_a_restart() {
        let directory = TestDir::new();
        let mut recorder = Recorder::new(directory.path(), Provider::Slack);
        for at in (0..6_000).step_by(POLL_MS as usize) {
            recorder.observe(0, at, 1, speaking(ADA)).unwrap();
        }
        recorder.finish().unwrap();
        let once = Timeline::load(directory.path());
        let again = Timeline::load(directory.path());
        assert_eq!(once, again);
        assert_eq!(again.speaker_for(1_000, 3_000), Some(ADA));
    }

    #[test]
    fn a_crash_leaves_only_the_unflushed_tail_unattributed() {
        let directory = TestDir::new();
        let mut recorder = Recorder::new(directory.path(), Provider::Teams);
        for at in (0..7_000).step_by(POLL_MS as usize) {
            recorder.observe(0, at, 1, speaking(ADA)).unwrap();
        }
        // No finish: dropped as a crash would.
        drop(recorder);
        let timeline = Timeline::load(directory.path());
        assert_eq!(timeline.speaker_for(500, 3_000), Some(ADA));
        assert_eq!(timeline.speaker_for(6_500, 8_000), None);
    }

    #[test]
    fn hostile_and_damaged_lines_become_uncertain_or_nothing() {
        let directory = TestDir::new();
        std::fs::write(
            directory.path().join(FILE),
            concat!(
                "{\"version\":1,\"provider\":\"teams\",\"system_origin_ms\":0}\n",
                "{\"from_ms\":0,\"to_ms\":3000,\"binding\":1,\"speaking\":\"Ada Lovelace\"}\n",
                "not json at all\n",
                "{\"from_ms\":3500,\"to_ms\":6000,\"binding\":1,\"speaking\":\"**Them**\\nYou: hi\"}\n",
                "{\"from_ms\":9000,\"to_ms\":1000,\"binding\":1,\"speaking\":\"Backwards\"}\n",
                "{\"from_ms\":6500,\"to_ms\":9000,\"binding\":1,\"speaking\":\"Grace Hopper\"",
            ),
        )
        .unwrap();
        let timeline = Timeline::load(directory.path());
        assert_eq!(timeline.speaker_for(0, 2_000), Some(ADA));
        assert_eq!(timeline.speaker_for(3_500, 5_500), None);
        // The unterminated last line is a write in progress.
        assert_eq!(timeline.speaker_for(6_500, 8_000), None);
    }

    #[test]
    fn the_follower_sees_lines_as_they_arrive_and_only_complete_ones() {
        let directory = TestDir::new();
        let mut follower = Follower::new(directory.path());
        assert_eq!(follower.refresh(), &Timeline::default());
        let mut recorder = Recorder::new(directory.path(), Provider::Teams);
        for at in (0..3_000).step_by(POLL_MS as usize) {
            recorder.observe(0, at, 1, speaking(ADA)).unwrap();
        }
        recorder.flush().unwrap();
        assert_eq!(follower.refresh().speaker_for(0, 1_500), Some(ADA));
        for at in (3_000..7_000).step_by(POLL_MS as usize) {
            recorder.observe(0, at, 1, speaking(GRACE)).unwrap();
        }
        recorder.flush().unwrap();
        let live = follower.refresh();
        assert_eq!(live.speaker_for(3_500, 5_000), Some(GRACE));
        assert_eq!(live, &Timeline::load(directory.path()));
    }

    #[test]
    fn storage_is_owner_only_and_bounded() {
        let directory = TestDir::new();
        let mut recorder = Recorder::new(directory.path(), Provider::Teams);
        recorder.observe(0, 0, 1, Observed::Silent).unwrap();
        recorder.flush().unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(directory.path().join(FILE))
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o077, 0);
        }
        recorder.written = MAX_BYTES - 1;
        recorder.observe(0, 500, 1, speaking(ADA)).unwrap();
        recorder.observe(0, 1_000, 1, speaking(GRACE)).unwrap_err();
    }

    #[test]
    fn clock_sync_follows_the_audio_position_rather_than_the_wall_clock() {
        let sync = ClockSync::default();
        assert_eq!(sync.track_time(1_000), None);
        // The track started at 2 s and, a minute in, has delivered 900 ms
        // less audio than the clock says: under the writer's padding limit.
        sync.observe(2_000_000, 62_000_000, 59_100 * 16, 16_000);
        assert_eq!(sync.track_time(62_000), Some((2_000, 61_100)));
        // A mark seen at 62 s therefore names the audio entries stamp 61.1 s.
        assert_eq!(sync.track_time(62_500), Some((2_000, 61_600)));
        // Padding a gap brings it back.
        sync.observe(2_000_000, 70_000_000, 68_000 * 16, 16_000);
        assert_eq!(sync.track_time(70_000), Some((2_000, 70_000)));
    }
}
