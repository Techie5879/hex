//! Labs › Meeting speaker names: who said a line of the call audio, from the
//! participant the call app itself marks as speaking. Teams (desktop) and
//! Slack huddles, on macOS, through the Accessibility access Hex already has.
//!
//! One worker per recording reads the call app every [`POLL_MS`], bounded in
//! elements and time, and appends what it saw to `speakers.ndjson` on the
//! system track's own timeline (`timeline.rs`). The transcript names a
//! system-audio line only when that evidence shows one person throughout it;
//! everything else stays `Them`, and the microphone stays `You`. Capture,
//! echo suppression, and the paste cursor never read names.
//!
//! The adapters (`teams.rs`, `slack.rs`) read synthetic trees in tests and
//! the real app through `native.rs`. Nothing here clicks, focuses, asks for a
//! permission, reads captions or rosters, or talks to a service.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

mod ax;
mod names;
#[cfg(target_os = "macos")]
mod native;
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
mod slack;
#[cfg(test)]
mod surfaces_tests;
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
mod teams;
mod timeline;

pub use names::is_valid as is_valid_name;
#[cfg(test)]
use timeline::FILE;
pub use timeline::{ClockSync, Follower, Timeline};

use ax::{NoCall, Reading};
use timeline::{Observed, POLL_MS, Recorder};

/// A reading that took longer than this says too little about any moment.
const MAX_READ_MS: u64 = 700;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Provider {
    Teams,
    Slack,
}

impl Provider {
    pub const ALL: [Self; 2] = [Self::Teams, Self::Slack];

    pub fn bundle_id(self) -> &'static str {
        match self {
            Self::Teams => teams::BUNDLE_ID,
            Self::Slack => slack::BUNDLE_ID,
        }
    }

    /// The providers a recording of `application` (a canonical bundle
    /// identifier) may read: that app's, or for a recording started by hand
    /// with no app, either, whichever holds a call.
    pub fn for_application(application: Option<&str>) -> Vec<Self> {
        match application {
            None => Self::ALL.to_vec(),
            Some(id) => Self::ALL
                .into_iter()
                .filter(|provider| provider.bundle_id() == id)
                .collect(),
        }
    }
}

/// What the Meetings pane says about names while a recording runs.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum Status {
    #[default]
    Waiting,
    Reading,
    PermissionRequired,
    Unavailable,
}

impl Status {
    pub const ALL: [Self; 4] = [
        Self::Waiting,
        Self::Reading,
        Self::PermissionRequired,
        Self::Unavailable,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Waiting => "Waiting for a Teams call or Slack huddle to name speakers",
            Self::Reading => "Naming speakers from the call app",
            Self::PermissionRequired => "Allow Accessibility to name speakers",
            Self::Unavailable => "Speaker names unavailable",
        }
    }
}

/// One reading of the call app.
#[derive(Debug)]
pub(crate) enum Sample {
    /// A verified call; `binding` changes whenever its window or region does.
    Call {
        binding: u32,
        reading: Reading,
    },
    NoCall(NoCall),
    PermissionRequired,
}

/// Where readings come from: macOS Accessibility, or a script in tests.
pub(crate) trait Source {
    /// The provider whose call was found, once one was.
    fn provider(&self) -> Option<Provider>;
    fn read(&mut self) -> Sample;
}

type Shared = Arc<Mutex<Status>>;

fn sessions() -> &'static Mutex<HashMap<String, Shared>> {
    static SESSIONS: OnceLock<Mutex<HashMap<String, Shared>>> = OnceLock::new();
    SESSIONS.get_or_init(Default::default)
}

fn lock<T>(value: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    value.lock().unwrap_or_else(|error| error.into_inner())
}

/// The live status of meeting `id` while this process names its speakers.
pub fn active_status(id: &str) -> Option<Status> {
    let state = lock(sessions()).get(id).cloned()?;
    Some(*lock(&state))
}

pub(super) struct Session {
    id: String,
    stop: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
}

impl Session {
    /// Starts naming speakers for a recording of `application`, when the Labs
    /// switch is on at the start and this OS has a reader for that app.
    /// Turning the switch on later takes effect from the next recording.
    #[cfg_attr(not(target_os = "macos"), allow(unused_variables))]
    pub(super) fn start(
        id: &str,
        directory: &Path,
        clock: Instant,
        sync: Arc<ClockSync>,
        application: Option<&str>,
    ) -> Option<Self> {
        let providers = Provider::for_application(application);
        if !crate::labs::meeting_speaker_names() || providers.is_empty() {
            return None;
        }
        #[cfg(target_os = "macos")]
        {
            let state = Shared::default();
            lock(sessions()).insert(id.to_owned(), state.clone());
            let stop = Arc::new(AtomicBool::new(false));
            let worker_stop = stop.clone();
            let directory = directory.to_path_buf();
            let worker = thread::Builder::new()
                .name("meeting-speakers".into())
                .spawn(move || {
                    crate::transcription::background_priority::deprioritize_background_thread();
                    let source = native::Reader::new(providers);
                    run(
                        source,
                        &state,
                        &worker_stop,
                        &crate::labs::meeting_speaker_names,
                        &|| clock.elapsed().as_millis() as u64,
                        &sync,
                        directory,
                    );
                });
            match worker {
                Ok(worker) => Some(Self {
                    id: id.to_owned(),
                    stop,
                    worker: Some(worker),
                }),
                Err(error) => {
                    lock(sessions()).remove(id);
                    tracing::warn!(%error, "could not start naming meeting speakers");
                    None
                }
            }
        }
        #[cfg(not(target_os = "macos"))]
        None
    }

    /// Signals at the audio stop boundary; [`Drop`] waits for the last write.
    pub(super) fn request_stop(&self) {
        self.stop.store(true, Ordering::Release);
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        self.request_stop();
        if let Some(worker) = self.worker.take() {
            // A reading is bounded, but quitting never waits for one.
            while !worker.is_finished() && !crate::desktop::process_exit::quitting() {
                thread::sleep(Duration::from_millis(10));
            }
            if worker.is_finished() {
                let _ = worker.join();
            }
        }
        lock(sessions()).remove(&self.id);
    }
}

/// Reads until stopped or switched off, storing what it sees.
fn run(
    mut source: impl Source,
    status: &Mutex<Status>,
    stop: &AtomicBool,
    enabled: &dyn Fn() -> bool,
    now_ms: &dyn Fn() -> u64,
    sync: &ClockSync,
    directory: PathBuf,
) {
    let mut writer = Writer::new(directory);
    let running = || !stop.load(Ordering::Acquire) && enabled();
    while running() {
        let started = now_ms();
        let sample = source.read();
        let finished = now_ms();
        // A reading that finishes after Stop or the switch going off is
        // dropped, so nothing is stored for time after either.
        if !running() {
            break;
        }
        match writer.accept(source.provider(), sample, started, finished, sync) {
            Ok(next) => *lock(status) = next,
            Err(error) => {
                tracing::warn!(%error, "stopped naming meeting speakers");
                *lock(status) = Status::Unavailable;
                break;
            }
        }
        while running() && now_ms() < started + POLL_MS {
            thread::sleep(Duration::from_millis(25));
        }
    }
    if let Err(error) = writer.finish() {
        tracing::warn!(%error, "could not save the last meeting speaker readings");
    }
}

/// Turns readings into stored observations.
struct Writer {
    directory: PathBuf,
    recorder: Option<Recorder>,
}

impl Writer {
    fn new(directory: PathBuf) -> Self {
        Self {
            directory,
            recorder: None,
        }
    }

    fn accept(
        &mut self,
        provider: Option<Provider>,
        sample: Sample,
        started: u64,
        finished: u64,
        sync: &ClockSync,
    ) -> std::io::Result<Status> {
        let (binding, observed, status) = match sample {
            Sample::Call { binding, reading } => {
                let observed = match reading {
                    _ if finished.saturating_sub(started) > MAX_READ_MS => Observed::Uncertain,
                    Reading::Speaking(name) => Observed::Speaking(name),
                    Reading::Silent => Observed::Silent,
                    Reading::Uncertain => Observed::Uncertain,
                };
                (binding, observed, Status::Reading)
            }
            // Binding 0 is no call: a stretch never continues across it.
            Sample::NoCall(_) => (0, Observed::Uncertain, Status::Waiting),
            Sample::PermissionRequired => (0, Observed::Uncertain, Status::PermissionRequired),
        };
        // Before the call audio starts there is nothing to name.
        let Some((origin, at)) = sync.track_time(started + (finished - started) / 2) else {
            return Ok(status);
        };
        if self.recorder.is_none()
            && binding != 0
            && let Some(provider) = provider
        {
            self.recorder = Some(Recorder::new(&self.directory, provider));
        }
        if let Some(recorder) = &mut self.recorder {
            recorder.observe(origin, at, binding, observed)?;
            recorder.flush()?;
        }
        Ok(status)
    }

    fn finish(self) -> std::io::Result<()> {
        self.recorder.map_or(Ok(()), Recorder::finish)
    }
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::sync::atomic::{AtomicU32, AtomicU64};

    use super::timeline::tests::TestDir;
    use super::*;
    use crate::meeting::{MeetingSource, TranscriptEntry};

    /// Each reading takes 500 ms of a fake clock, one poll.
    struct Script<'a> {
        provider: Option<Provider>,
        samples: VecDeque<Sample>,
        stop: &'a AtomicBool,
        clock: &'a AtomicU64,
    }

    impl Source for Script<'_> {
        fn provider(&self) -> Option<Provider> {
            self.provider
        }
        fn read(&mut self) -> Sample {
            self.clock.fetch_add(POLL_MS, Ordering::SeqCst);
            match self.samples.pop_front() {
                Some(sample) => sample,
                None => {
                    self.stop.store(true, Ordering::Release);
                    Sample::NoCall(NoCall::Absent)
                }
            }
        }
    }

    fn speaking(binding: u32, name: &str) -> Sample {
        Sample::Call {
            binding,
            reading: Reading::Speaking(name.into()),
        }
    }

    /// The system track started at capture time 0.
    fn synced() -> ClockSync {
        let sync = ClockSync::default();
        sync.observe(0, 1_000_000, 16_000, 16_000);
        sync
    }

    /// Runs `samples` through the worker, one per poll; returns the meeting
    /// folder. Reading `k` is stored at `500 k + 250` ms.
    fn record(samples: Vec<Sample>, provider: Option<Provider>) -> (TestDir, Status) {
        let directory = TestDir::new();
        let clock = AtomicU64::new(0);
        let stop = AtomicBool::new(false);
        let status = Mutex::new(Status::Waiting);
        let now = || clock.load(Ordering::SeqCst);
        run(
            Script {
                provider,
                samples: samples.into(),
                stop: &stop,
                clock: &clock,
            },
            &status,
            &stop,
            &|| true,
            &now,
            &synced(),
            directory.path().to_path_buf(),
        );
        let status = *lock(&status);
        (directory, status)
    }

    #[test]
    fn readings_become_names_on_the_recordings_timeline() {
        let samples = (0..20).map(|_| speaking(1, "Ada Lovelace")).collect();
        let (directory, status) = record(samples, Some(Provider::Teams));
        assert_eq!(status, Status::Reading);
        let mut entries = vec![TranscriptEntry {
            source: MeetingSource::System,
            start_ms: 1_000,
            end_ms: 4_000,
            text: "Shall we start?".into(),
            speaker: None,
        }];
        Timeline::load(directory.path()).label(&mut entries, 0, Some(0));
        assert_eq!(entries[0].speaker.as_deref(), Some("Ada Lovelace"));
    }

    #[test]
    fn nothing_is_stored_before_a_call_is_found_or_when_none_ever_is() {
        let (directory, status) = record(
            vec![Sample::NoCall(NoCall::Absent), Sample::PermissionRequired],
            Some(Provider::Slack),
        );
        assert_eq!(status, Status::PermissionRequired);
        assert!(!directory.path().join(FILE).exists());
    }

    #[test]
    fn losing_the_call_or_the_permission_breaks_the_stretch() {
        let mut samples = (0..8)
            .map(|_| speaking(1, "Ada Lovelace"))
            .collect::<Vec<_>>();
        samples.push(Sample::PermissionRequired);
        samples.extend((0..8).map(|_| speaking(1, "Ada Lovelace")));
        let (directory, _) = record(samples, Some(Provider::Slack));
        let timeline = Timeline::load(directory.path());
        assert_eq!(timeline.speaker_for(0, 1_500), Some("Ada Lovelace"));
        // The permission was missing at 4.25 s.
        assert_eq!(timeline.speaker_for(2_500, 5_000), None);
        assert_eq!(timeline.speaker_for(5_500, 7_500), Some("Ada Lovelace"));
    }

    #[test]
    fn the_switch_going_off_stops_reading_and_storing() {
        let directory = TestDir::new();
        let stop = AtomicBool::new(false);
        let enabled = AtomicBool::new(true);
        let reads = AtomicU32::new(0);
        let clock = AtomicU64::new(0);
        struct Switching<'a> {
            reads: &'a AtomicU32,
            enabled: &'a AtomicBool,
            clock: &'a AtomicU64,
        }
        impl Source for Switching<'_> {
            fn provider(&self) -> Option<Provider> {
                Some(Provider::Teams)
            }
            fn read(&mut self) -> Sample {
                self.clock.fetch_add(POLL_MS, Ordering::SeqCst);
                if self.reads.fetch_add(1, Ordering::SeqCst) == 5 {
                    self.enabled.store(false, Ordering::SeqCst);
                }
                speaking(1, "Ada Lovelace")
            }
        }
        run(
            Switching {
                reads: &reads,
                enabled: &enabled,
                clock: &clock,
            },
            &Mutex::new(Status::Waiting),
            &stop,
            &|| enabled.load(Ordering::SeqCst),
            &|| clock.load(Ordering::SeqCst),
            &synced(),
            directory.path().to_path_buf(),
        );
        assert_eq!(
            reads.load(Ordering::SeqCst),
            6,
            "no reading after the switch"
        );
        // Five readings stored; the sixth finished after the switch went off.
        let timeline = Timeline::load(directory.path());
        assert_eq!(timeline.speaker_for(0, 400), Some("Ada Lovelace"));
        assert_eq!(timeline.speaker_for(2_000, 4_000), None);
    }

    #[test]
    fn a_slow_reading_is_stored_as_uncertain() {
        let directory = TestDir::new();
        let sync = synced();
        let mut writer = Writer::new(directory.path().to_path_buf());
        for at in (0..4_000).step_by(500) {
            writer
                .accept(
                    Some(Provider::Teams),
                    speaking(1, "Ada Lovelace"),
                    at,
                    at + 900,
                    &sync,
                )
                .unwrap();
        }
        writer.finish().unwrap();
        assert_eq!(
            Timeline::load(directory.path()).speaker_for(500, 2_500),
            None
        );
    }

    #[test]
    fn nothing_is_stored_before_the_call_audio_starts() {
        let directory = TestDir::new();
        let mut writer = Writer::new(directory.path().to_path_buf());
        let status = writer
            .accept(
                Some(Provider::Slack),
                speaking(1, "Ada Lovelace"),
                0,
                100,
                &ClockSync::default(),
            )
            .unwrap();
        assert_eq!(status, Status::Reading);
        writer.finish().unwrap();
        assert!(!directory.path().join(FILE).exists());
    }

    #[test]
    fn providers_follow_the_recorded_app() {
        assert_eq!(Provider::for_application(None), Provider::ALL.to_vec());
        assert_eq!(
            Provider::for_application(Some("com.microsoft.teams2")),
            vec![Provider::Teams]
        );
        assert_eq!(
            Provider::for_application(Some("com.tinyspeck.slackmacgap")),
            vec![Provider::Slack]
        );
        assert!(Provider::for_application(Some("us.zoom.xos")).is_empty());
    }

    #[test]
    fn sessions_never_start_where_there_is_no_reader_or_the_switch_is_off() {
        let directory = TestDir::new();
        let sync = Arc::new(ClockSync::default());
        let start = |application| {
            Session::start(
                "synthetic-speakers",
                directory.path(),
                Instant::now(),
                sync.clone(),
                application,
            )
        };
        assert!(start(Some("us.zoom.xos")).is_none());
        if !crate::labs::meeting_speaker_names() {
            assert!(start(Some("com.microsoft.teams2")).is_none());
            assert!(start(None).is_none());
            assert_eq!(active_status("synthetic-speakers"), None);
        }
    }
}
