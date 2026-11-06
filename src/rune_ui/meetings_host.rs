//! The Meetings pane's listing, read from the meetings store on every OS,
//! and the meeting requests the Linux and Windows windows perform in their
//! own process (macOS's go through `macos_host`).

use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, SyncSender, TryRecvError};
use std::time::{Duration, Instant};

use super::meetings::{Chapter, Meeting, Status, Transcript, Turn};
#[cfg(not(target_os = "macos"))]
use super::platform::MeetingRequest;
#[cfg(not(target_os = "macos"))]
use crate::desktop::i18n::tr;

const RESCAN: Duration = Duration::from_secs(1);

/// Linux and Windows: the window and the meeting controller share one
/// process, so the window performs meeting requests itself.
#[cfg(not(target_os = "macos"))]
#[derive(Default)]
pub(super) struct LocalMeetings {
    feed: MeetingFeed,
    /// The transcript folder panel, while it is open.
    folder_choice: Option<crate::desktop::file_dialog::Pending>,
}

#[cfg(not(target_os = "macos"))]
impl LocalMeetings {
    /// `wake` redraws the window once the folder panel answers.
    pub(super) fn send(
        &mut self,
        request: MeetingRequest,
        wake: impl Fn() + Send + 'static,
    ) -> Result<(), String> {
        use crate::meeting::MeetingRequest as Controller;
        let failed = |error: color_eyre::Report| format!("{error:#}");
        match request {
            MeetingRequest::Start => {
                crate::meeting::runtime::request(Controller::Start { title: None })
            }
            MeetingRequest::Stop => crate::meeting::runtime::request(Controller::Stop),
            MeetingRequest::Summarize(id) => {
                crate::meeting::runtime::request(Controller::Summarize(id))
            }
            MeetingRequest::SetMode(change) => crate::meeting::mode::request(change),
            MeetingRequest::Rename { id, title } => crate::meeting::rename(&id, &title)
                .map(|_| ())
                .map_err(failed),
            MeetingRequest::Trash(id) => crate::meeting::trash(&id).map_err(failed),
            MeetingRequest::Reveal(id) => {
                crate::meeting::reveal(&crate::meeting::root().map_err(failed)?.join(id));
                Ok(())
            }
            MeetingRequest::RevealFolder => {
                let folder = crate::app_settings::meeting_output_folder();
                std::fs::create_dir_all(&folder).map_err(|error| error.to_string())?;
                crate::meeting::reveal(&folder);
                Ok(())
            }
            MeetingRequest::ChooseFolder => {
                use crate::desktop::file_dialog::{Dialog, Target};
                if self.folder_choice.is_none() {
                    self.folder_choice = Some(crate::desktop::file_dialog::open(
                        Dialog {
                            title: tr("Transcript folder").into(),
                            target: Target::Folder,
                            directory: Some(crate::app_settings::meeting_output_folder()),
                        },
                        wake,
                    ));
                }
                Ok(())
            }
        }
    }

    /// The folder the panel answered with since the last call.
    pub(super) fn chosen_folder(&mut self) -> Option<Result<PathBuf, String>> {
        let choice = self.folder_choice.as_ref()?.poll()?;
        self.folder_choice = None;
        choice.transpose()
    }

    pub(super) fn meetings(&mut self) -> Option<Result<Vec<Meeting>, String>> {
        self.feed.poll()
    }
}

/// A nonblocking feed. The one worker owns both the query cache and all disk reads.
#[derive(Default)]
pub(super) struct MeetingFeed {
    worker: Option<ScanWorker>,
    root: Option<PathBuf>,
    scan_at: Option<Instant>,
    pending: bool,
    shown: Option<Result<Vec<Meeting>, String>>,
}

struct ScanWorker {
    requests: SyncSender<()>,
    results: Receiver<Result<Vec<Meeting>, String>>,
}

impl ScanWorker {
    fn start(root: Option<PathBuf>) -> Result<Self, String> {
        let (requests, jobs) = mpsc::sync_channel(1);
        let (completed, results) = mpsc::sync_channel(1);
        std::thread::Builder::new()
            .name("hex-meeting-feed".into())
            .spawn(move || {
                let mut library = None;
                while jobs.recv().is_ok() {
                    let listed = scan(&mut library, root.as_deref());
                    if completed.send(listed).is_err() {
                        break;
                    }
                }
            })
            .map_err(|error| error.to_string())?;
        Ok(Self { requests, results })
    }
}

impl MeetingFeed {
    pub(super) fn poll(&mut self) -> Option<Result<Vec<Meeting>, String>> {
        let mut completed = None;
        if let Some(worker) = &self.worker {
            match worker.results.try_recv() {
                Ok(listed) => {
                    self.pending = false;
                    completed = Some(listed);
                }
                Err(TryRecvError::Empty) => {}
                Err(TryRecvError::Disconnected) => {
                    self.worker = None;
                    self.pending = false;
                    completed = Some(Err("Meeting listing worker stopped".into()));
                }
            }
        }
        let now = Instant::now();
        if !self.pending && self.scan_at.is_none_or(|at| now >= at) {
            self.scan_at = Some(now + RESCAN);
            if self.worker.is_none() {
                match ScanWorker::start(self.root.clone()) {
                    Ok(worker) => self.worker = Some(worker),
                    Err(error) => completed = Some(Err(error)),
                }
            }
            if let Some(worker) = &self.worker {
                match worker.requests.try_send(()) {
                    Ok(()) => self.pending = true,
                    Err(error) => completed = Some(Err(error.to_string())),
                }
            }
        }
        let listed = completed?;
        if self.shown.as_ref() == Some(&listed) {
            return None;
        }
        self.shown = Some(listed.clone());
        Some(listed)
    }
}

/// Runs only on the feed worker, including snapshot indexes and image metadata.
fn scan(
    library: &mut Option<crate::meeting::query::Library>,
    root: Option<&std::path::Path>,
) -> Result<Vec<Meeting>, String> {
    let library = match library {
        Some(library) => library,
        None => {
            let root = match root {
                Some(root) => root.to_path_buf(),
                None => crate::meeting::root().map_err(|error| format!("{error:#}"))?,
            };
            library.insert(crate::meeting::query::Library::new(root))
        }
    };
    let scan = library.scan().map_err(|error| format!("{error:#}"))?;
    let now_ms = crate::desktop::events::now_ms();
    Ok(scan
        .meetings
        .iter()
        .map(|meeting| pane_meeting(meeting, now_ms))
        .collect())
}

/// A stored meeting as the pane shows it.
fn pane_meeting(meeting: &crate::meeting::query::Meeting, now_ms: u64) -> Meeting {
    use crate::meeting::MeetingStatus;
    let manifest = &meeting.manifest;
    let transcript = if meeting.entries.is_empty() && !meeting.warnings.is_empty() {
        Transcript::Unavailable(meeting.warnings.join("\n"))
    } else {
        Transcript::Loaded(turns(&meeting.entries))
    };
    Meeting {
        snapshots: crate::meeting::snapshots::read(&meeting.id),
        id: meeting.id.clone(),
        title: meeting.title(),
        status: match meeting.status {
            MeetingStatus::Starting => Status::Starting,
            MeetingStatus::Recording => Status::Recording,
            MeetingStatus::Transcribing => Status::Transcribing,
            MeetingStatus::Complete => Status::Complete,
            MeetingStatus::Interrupted => Status::Interrupted,
            MeetingStatus::Failed => Status::Failed,
        },
        day: crate::meeting::day_label(meeting.start_ms(), now_ms),
        clock: crate::meeting::clock(meeting.start_ms()),
        duration_ms: meeting.duration_ms(now_ms),
        app: manifest.app.clone(),
        summary: manifest
            .notes
            .as_ref()
            .map(|notes| notes.summary.clone())
            .filter(|summary| !summary.is_empty()),
        notes_running: crate::meeting::notes_in_progress(&meeting.id),
        shared_room: manifest.mode == crate::meeting::MeetingMode::SharedRoom,
        has_markdown: manifest
            .export_path
            .as_ref()
            .is_some_and(|path| path.exists()),
        error: manifest.error.clone(),
        export_error: manifest.export_error.clone(),
        notes_error: manifest.notes_error.clone(),
        live_error: manifest.live_transcription_error.clone(),
        transcript,
        // Sections come from the transcript lines, not the merged turns, as
        // the menu bar and `hex meetings show --chapter` read them.
        kept_for: (!meeting.status.is_active())
            .then(|| {
                crate::meeting::retention_for(manifest, crate::app_settings::meeting_retention())
            })
            .filter(|retention| retention.max_age_ms().is_some())
            .map(crate::app_settings::MeetingRetention::label),
        chapters: meeting
            .chapter_views(now_ms)
            .iter()
            .map(|view| Chapter {
                title: view.chapter.title.clone(),
                summary: view.chapter.summary.clone(),
                start_ms: view.chapter.start_ms,
                markdown: view.markdown(),
            })
            .collect(),
    }
}

/// Speaker turns, marking silences of 30 s or more.
fn turns(entries: &[crate::meeting::TranscriptEntry]) -> Vec<Turn> {
    let mut previous_end: Option<u64> = None;
    crate::meeting::coalesce_transcript(entries.iter().cloned())
        .into_iter()
        .map(|turn| {
            let pause_before_ms = previous_end
                .map(|end| turn.start_ms.saturating_sub(end))
                .filter(|pause| *pause >= 30_000);
            previous_end = Some(previous_end.map_or(turn.end_ms, |end| end.max(turn.end_ms)));
            Turn {
                mine: turn.source == crate::meeting::MeetingSource::Microphone,
                room: turn.mode == crate::meeting::MeetingMode::SharedRoom,
                speaker: turn.speaker_name().map(str::to_owned),
                start_ms: turn.start_ms,
                text: turn.text,
                pause_before_ms,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_meeting_feed_lists_stored_meetings_once_until_they_change() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/meetings");
        let mut feed = MeetingFeed {
            root: Some(root),
            ..MeetingFeed::default()
        };
        // Poll never waits for the filesystem worker.
        assert!(feed.poll().is_none());
        let deadline = Instant::now() + Duration::from_secs(5);
        let meetings = loop {
            if let Some(listed) = feed.poll() {
                break listed.unwrap();
            }
            assert!(Instant::now() < deadline, "listing worker did not finish");
            std::thread::sleep(Duration::from_millis(1));
        };
        let ids: Vec<_> = meetings.iter().map(|meeting| meeting.id.as_str()).collect();
        assert_eq!(
            ids,
            [
                "1790001000000-1003",
                "1790000500000-1002",
                "1790000000000-1001"
            ],
            "newest first, without the unreadable folder"
        );
        let standup = &meetings[0];
        assert_eq!(standup.status, Status::Interrupted, "an unlocked recording");
        let review = &meetings[1];
        assert_eq!(review.title, "Roadmap review");
        let Transcript::Loaded(turns) = &review.transcript else {
            panic!("the roadmap review has a transcript");
        };
        assert!(!turns[0].mine && turns[1].mine);
        assert_eq!(turns.last().unwrap().pause_before_ms, Some(42_000));
        feed.scan_at = None;
        assert!(feed.poll().is_none());
        while feed.pending {
            assert!(feed.poll().is_none(), "unchanged listing is not sent again");
            assert!(Instant::now() < deadline, "rescan worker did not finish");
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    #[test]
    fn polling_does_not_wait_or_queue_overlapping_scans() {
        let (requests, jobs) = mpsc::sync_channel(1);
        let (completed, results) = mpsc::sync_channel(1);
        let mut feed = MeetingFeed {
            worker: Some(ScanWorker { requests, results }),
            ..Default::default()
        };
        assert!(feed.poll().is_none());
        jobs.try_recv().expect("one job starts");
        assert!(feed.pending);
        // Even when another scan is due, a slow worker receives no second job.
        feed.scan_at = None;
        for _ in 0..100 {
            assert!(feed.poll().is_none());
        }
        assert!(matches!(jobs.try_recv(), Err(TryRecvError::Empty)));
        completed.send(Ok(Vec::new())).unwrap();
        assert_eq!(feed.poll(), Some(Ok(Vec::new())));
        jobs.try_recv()
            .expect("next due job starts only after completion");
        assert!(feed.pending);
    }

    #[test]
    fn copy_section_matches_the_menu_bar_and_the_cli() {
        let root = std::env::temp_dir().join(format!(
            "hex-copy-section-{}-{}",
            std::process::id(),
            crate::desktop::events::now_ms()
        ));
        let id = "1790003000000-1005";
        let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/meetings-chapters")
            .join(id);
        let directory = root.join(id);
        std::fs::create_dir_all(&directory).unwrap();
        std::fs::copy(
            fixture.join("manifest.json"),
            directory.join("manifest.json"),
        )
        .unwrap();
        // One speaker's two lines join into one turn across the boundary
        // at 06:30; the second line belongs to the chapter after.
        let line = |start_ms: u64, end_ms: u64, text: &str| crate::meeting::TranscriptEntry {
            mode: Default::default(),
            source: crate::meeting::MeetingSource::Microphone,
            start_ms,
            end_ms,
            text: text.into(),
            speaker: None,
        };
        let entries = [
            line(181_000, 188_000, "Next, the motion lint."),
            line(386_000, 389_000, "That settles the lint."),
            line(390_500, 397_000, "Last thing: a name for the chat app."),
        ];
        let ndjson = entries
            .iter()
            .map(|entry| serde_json::to_string(entry).unwrap() + "\n")
            .collect::<String>();
        std::fs::write(directory.join("transcript.ndjson"), ndjson).unwrap();

        let library = crate::meeting::query::Library::new(&root);
        let meeting = library.meeting(id).unwrap();
        let now_ms = crate::desktop::events::now_ms();
        let pane = pane_meeting(&meeting, now_ms);
        let chapters = &meeting.manifest.chapters;
        for (index, chapter) in pane.chapters.iter().enumerate() {
            // The menu bar's Copy Section (`meeting::section_for_id`).
            let menu = crate::meeting::section_markdown(
                &chapters[index],
                &crate::meeting::chapter_entries(&entries, chapters, index),
            );
            assert_eq!(chapter.markdown, menu);
            let cli = library.chapter_markdown(id, index + 1, now_ms).unwrap();
            assert!(cli.ends_with(&format!("\n{menu}\n")), "{cli}");
        }
        assert!(pane.chapters[1].markdown.contains("That settles the lint."));
        assert!(!pane.chapters[1].markdown.contains("Last thing"));
        assert!(pane.chapters[2].markdown.contains("Last thing"));
        std::fs::remove_dir_all(root).unwrap();
    }

    /// A shared-room meeting through every reader the pane, the CLI, MCP,
    /// and Option-Control-V use: source labels, no names, no `You`.
    #[test]
    fn a_shared_room_meeting_reads_as_room_mic_and_call_audio_everywhere() {
        use crate::meeting::{MeetingMode, MeetingSource, TranscriptEntry};
        let root = std::env::temp_dir().join(format!(
            "hex-shared-room-readers-{}-{}",
            std::process::id(),
            crate::desktop::events::now_ms()
        ));
        let id = "1790000000000-77";
        let directory = root.join(id);
        std::fs::create_dir_all(&directory).unwrap();
        std::fs::write(
            directory.join("manifest.json"),
            serde_json::json!({
                "schema_version": 1,
                "id": id,
                "title": "Planning",
                "status": "complete",
                "created_at_ms": 1_790_000_000_000_u64,
                "ended_at_ms": 1_790_000_060_000_u64,
                "duration_ms": 60_000,
                "system": {"file": "system.wav", "samples": 0, "dropped_packets": 0},
                "microphone": {"file": "microphone.wav", "samples": 0, "dropped_packets": 0},
                "error": null,
                "app": "Slack",
                "mode": "shared_room",
            })
            .to_string(),
        )
        .unwrap();
        let line = |source, start_ms: u64, text: &str, speaker: Option<&str>| TranscriptEntry {
            source,
            start_ms,
            end_ms: start_ms + 2_000,
            text: text.into(),
            speaker: speaker.map(str::to_owned),
            mode: MeetingMode::SharedRoom,
        };
        let entries = [
            line(
                MeetingSource::Microphone,
                1_000,
                "Can someone dim the lights?",
                None,
            ),
            // A name stored for the call account is not who spoke.
            line(
                MeetingSource::System,
                9_000,
                "The demo is ready.",
                Some("Ada Lovelace"),
            ),
        ];
        let ndjson = entries
            .iter()
            .map(|entry| serde_json::to_string(entry).unwrap() + "\n")
            .collect::<String>();
        std::fs::write(directory.join("transcript.ndjson"), ndjson).unwrap();
        std::fs::write(directory.join("transcript.md"), "# Planning\n").unwrap();

        let library = crate::meeting::query::Library::new(&root);
        let meeting = library.meeting(id).unwrap();
        let now_ms = crate::desktop::events::now_ms();
        let pane = pane_meeting(&meeting, now_ms);
        assert!(pane.shared_room);
        let Transcript::Loaded(turns) = &pane.transcript else {
            panic!("transcript");
        };
        assert_eq!(
            turns
                .iter()
                .map(|turn| (turn.mine, turn.room, turn.speaker.clone()))
                .collect::<Vec<_>>(),
            [(true, true, None), (false, true, None)]
        );

        let list = library
            .list(
                &crate::meeting::query::ListQuery {
                    since: None,
                    until: None,
                    query: None,
                    limit: 20,
                    include_in_progress: true,
                },
                now_ms,
            )
            .unwrap();
        assert!(
            list.contains("participants: Room mic, Call audio (Slack)"),
            "{list}"
        );
        let markdown = library
            .markdown(id, crate::meeting::query::OffsetRange::default(), now_ms)
            .unwrap();
        assert!(markdown.contains("**Room mic** · 00:01"), "{markdown}");
        assert!(markdown.contains("**Call audio** · 00:09"), "{markdown}");
        for hidden in ["**You**", "**Them**", "Ada Lovelace"] {
            assert!(!markdown.contains(hidden), "{markdown}");
        }
        let room_only = library
            .transcript(
                &crate::meeting::query::TranscriptQuery {
                    meeting_id: id.into(),
                    start: None,
                    end: None,
                    speaker: Some("room mic".into()),
                    max_chars: 20_000,
                    cursor: None,
                },
                now_ms,
            )
            .unwrap();
        assert!(room_only.contains("(Room mic only)"), "{room_only}");
        assert!(room_only.contains("Room mic: Can someone dim the lights?"));
        assert!(!room_only.contains("The demo is ready."));

        let mut cursor = crate::meeting::MeetingPasteCursor::default();
        let pasted = cursor.consume_entries(id, &entries, |_| Ok(())).unwrap();
        assert_eq!(
            pasted,
            "Room mic: Can someone dim the lights?\n\nCall audio: The demo is ready."
        );
        assert_eq!(cursor.pending_turn_count(id, &entries), 0);
        std::fs::remove_dir_all(root).unwrap();
    }
}
