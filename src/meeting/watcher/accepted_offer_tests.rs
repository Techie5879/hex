//! Synthetic calls verify accepted-offer auto-stop without recording audio.
use super::*;
use crate::microphone_activity::ActiveMicrophoneApplication;

fn meeting(id: &str, start_minute: i64, minutes: i64) -> CalendarMeeting {
    CalendarMeeting {
        id: id.into(),
        title: id.into(),
        start_ms: start_minute * 60_000,
        end_ms: (start_minute + minutes) * 60_000,
        attendees: vec!["Ana Ruiz".into()],
        link: None,
    }
}
const TEAMS: &str = "com.microsoft.teams2";
const SLACK: &str = "com.tinyspeck.slackmacgap";
const ZOOM: &str = "us.zoom.xos";

/// Records nothing: reports a start, then waits for Stop.
fn fake_record(
    start: meeting::MeetingStart,
    stop: &AtomicBool,
    updates: Option<SyncSender<meeting::RecordingUpdate>>,
) -> Result<meeting::MeetingManifest> {
    if let Some(updates) = updates {
        let _ = updates.send(meeting::RecordingUpdate::Started {
            id: "synthetic".into(),
            title: start.title.unwrap_or_else(|| "Synthetic call".into()),
        });
    }
    while !stop.load(Ordering::Relaxed) {
        thread::sleep(Duration::from_millis(2));
    }
    Err(eyre!("nothing was recorded"))
}

/// Synthetic call activity: the apps on the microphone (`None` when the
/// read fails) and whether the call app plays audio.
#[derive(Default)]
struct Probe {
    microphone: Option<Vec<&'static str>>,
    playing: bool,
    asked: Vec<String>,
}

impl CallProbe for Probe {
    fn on_microphone(&mut self, application: &str) -> Result<bool> {
        self.asked.push(application.into());
        let microphone = self
            .microphone
            .as_ref()
            .ok_or_else(|| eyre!("no microphone reading"))?;
        Ok(microphone.contains(&application))
    }

    fn playing(&mut self, _application: &str) -> bool {
        self.playing
    }
}

/// The meeting controller with a fake recorder and synthetic call
/// activity, on a clock the test moves.
struct Controller {
    recording: Option<ActiveRecording>,
    detector: MeetingDetector,
    reminders: Reminders,
    events: SyncSender<ControllerEvent>,
    received: Receiver<ControllerEvent>,
    probe: Probe,
    lab_on: bool,
    epoch: Instant,
    now_ms: u64,
}

impl Controller {
    fn new(lab_on: bool, microphone: &[&'static str]) -> Self {
        let (events, received) = mpsc::sync_channel(1024);
        Self {
            recording: None,
            detector: MeetingDetector::default(),
            reminders: Reminders::default(),
            events,
            received,
            probe: Probe {
                microphone: Some(microphone.to_vec()),
                ..Probe::default()
            },
            lab_on,
            epoch: Instant::now(),
            now_ms: 0,
        }
    }

    fn command(&mut self, command: ControllerCommand) {
        handle_command(
            &mut self.recording,
            &mut None,
            &mut self.detector,
            &mut self.reminders,
            command,
            true,
            &self.events,
            fake_record,
        );
    }

    /// Polls until the recorder reports its start.
    fn started(&mut self) -> bool {
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            self.poll();
            if let Some(active) = &self.recording
                && active.reported_started
            {
                // Only Always record says it started automatically.
                let expected = active.origin == Origin::Automatic;
                return self.received.try_iter().any(|event| {
                    matches!(event, ControllerEvent::RecordingStarted { automatic, .. }
                            if automatic == expected)
                });
            }
            thread::sleep(Duration::from_millis(2));
        }
        false
    }

    fn poll(&mut self) {
        poll_recording(
            &mut self.recording,
            &mut self.detector,
            &mut BrowserWatch::new(|_| Ok(ContextSnapshot::default())),
            &self.events,
            &mut self.probe,
            self.lab_on,
            self.epoch + Duration::from_millis(self.now_ms),
        )
        .unwrap();
    }

    /// One pass a second, as the loop's 500 ms passes would, until
    /// `until_ms`.
    fn run_until(&mut self, until_ms: u64) {
        while self.now_ms < until_ms {
            self.now_ms = (self.now_ms + 1_000).min(until_ms);
            self.poll();
        }
    }

    fn stopped(&self) -> bool {
        self.recording
            .as_ref()
            .is_some_and(|active| active.stop.load(Ordering::Relaxed))
    }

    /// What the controller reported since last asked.
    fn reported(&self) -> Vec<&'static str> {
        self.received
            .try_iter()
            .filter_map(|event| match event {
                ControllerEvent::CallEnded {
                    auto_stop: true, ..
                } => Some("call ended, stops"),
                ControllerEvent::CallEnded {
                    auto_stop: false, ..
                } => Some("call ended"),
                ControllerEvent::CallEndedWithdrawn => Some("stop cancelled"),
                ControllerEvent::Transcribing => Some("transcribing"),
                ControllerEvent::RecordingContinued => Some("recording continues"),
                _ => None,
            })
            .collect()
    }
}

impl Drop for Controller {
    fn drop(&mut self) {
        if let Some(active) = self.recording.take() {
            active.stop.store(true, Ordering::Relaxed);
            let _ = active.worker.join();
        }
    }
}

fn offered(bundle_id: &str) -> MeetingCandidate {
    let application = ActiveMicrophoneApplication {
        pid: 7,
        bundle_id: bundle_id.into(),
        name: bundle_id.into(),
    };
    candidate_from_microphone(&application, &ContextSnapshot::default()).unwrap()
}

fn teams_meeting() -> CalendarMeeting {
    CalendarMeeting {
        link: Some(crate::calendar::CallLink {
            provider: crate::calendar::CallProvider::Teams,
            url: "https://teams.microsoft.com/l/meetup-join/synthetic".into(),
        }),
        ..meeting("standup", 0, 30)
    }
}

/// Teams leaves the microphone at 2 s while Zoom keeps it.
fn leave_at_2_s(controller: &mut Controller) {
    controller.run_until(2_000);
    controller.probe.microphone = Some(vec![ZOOM]);
}

#[test]
fn an_accepted_teams_or_slack_offer_stops_30_s_after_its_call_ends() {
    for (bundle_id, source) in [
        (TEAMS, MeetingSource::Teams),
        (SLACK, MeetingSource::SlackHuddle),
    ] {
        let mut controller = Controller::new(true, &[bundle_id, ZOOM]);
        let candidate = offered(bundle_id);
        assert_eq!(candidate.source, source);
        controller.command(ControllerCommand::Record(candidate, None));
        // Clicked, so never "Started automatically".
        assert!(controller.started(), "{bundle_id}");
        assert_eq!(
            controller.recording.as_ref().unwrap().origin,
            Origin::AcceptedOffer
        );

        controller.run_until(2_000);
        // Only the offered call's app counts: Zoom on the microphone
        // never keeps this call alive.
        controller.probe.microphone = Some(vec![ZOOM]);
        // Away from 3 s; the call ends 3 s later and stops 30 s after.
        controller.run_until(6_000);
        assert_eq!(controller.reported(), ["call ended, stops"]);
        controller.run_until(35_000);
        assert!(!controller.stopped(), "{bundle_id} stopped before 30 s");
        controller.run_until(36_000);
        assert!(controller.stopped(), "{bundle_id}");
        assert_eq!(controller.reported(), ["transcribing"]);
        assert!(
            controller
                .probe
                .asked
                .iter()
                .all(|asked| asked == bundle_id)
        );
    }
}

#[test]
fn always_record_still_stops_and_still_says_it_started_automatically() {
    let mut controller = Controller::new(true, &[TEAMS]);
    let start = Start::from_candidate(&offered(TEAMS));
    start_recording(
        &mut controller.recording,
        &mut controller.reminders,
        start,
        Origin::Automatic,
        &controller.events,
        fake_record,
    );
    assert!(controller.started());
    leave_at_2_s(&mut controller);
    controller.run_until(36_000);
    assert!(controller.stopped());
    assert_eq!(controller.reported(), ["call ended, stops", "transcribing"]);
}

#[test]
fn manual_and_calendar_recordings_never_stop_when_a_call_ends() {
    // A manual recording is tied to no call, so nothing is watched.
    let mut controller = Controller::new(true, &[TEAMS]);
    controller.command(ControllerCommand::Record(
        MeetingCandidate::manual_titled(Some("Design sync".into())),
        None,
    ));
    assert!(controller.started());
    assert_eq!(
        controller.recording.as_ref().unwrap().origin,
        Origin::Requested
    );
    leave_at_2_s(&mut controller);
    controller.run_until(120_000);
    assert!(!controller.stopped());
    assert!(controller.probe.asked.is_empty());

    // A calendar recording watches its Teams link's app but, started
    // before any call was seen, only says the call ended.
    let mut controller = Controller::new(true, &[TEAMS]);
    controller.command(ControllerCommand::RecordMeeting {
        meeting: teams_meeting(),
        join: false,
    });
    assert!(controller.started());
    leave_at_2_s(&mut controller);
    controller.run_until(120_000);
    assert!(!controller.stopped());
    assert_eq!(controller.reported(), ["call ended"]);
}

#[test]
fn with_the_lab_off_an_accepted_offer_records_until_stop() {
    let mut controller = Controller::new(false, &[TEAMS]);
    controller.command(ControllerCommand::Record(offered(TEAMS), None));
    assert!(controller.started());
    // Muted but still playing would count only with the lab on.
    controller.probe.playing = true;
    leave_at_2_s(&mut controller);
    controller.run_until(120_000);
    assert!(!controller.stopped());
    assert_eq!(controller.reported(), ["call ended"]);
}

#[test]
fn a_muted_call_or_a_failed_read_never_stops_an_accepted_offer() {
    // Muted: off the microphone but still playing the call.
    let mut controller = Controller::new(true, &[SLACK]);
    controller.command(ControllerCommand::Record(offered(SLACK), None));
    assert!(controller.started());
    controller.probe.playing = true;
    leave_at_2_s(&mut controller);
    controller.run_until(120_000);
    assert!(!controller.stopped());
    assert!(controller.reported().is_empty());

    // The read fails mid-countdown: counted as present, so the stop is
    // dropped and the absence starts over once reads work again.
    let mut controller = Controller::new(true, &[TEAMS]);
    controller.command(ControllerCommand::Record(offered(TEAMS), None));
    assert!(controller.started());
    leave_at_2_s(&mut controller);
    controller.run_until(10_000);
    controller.probe.microphone = None;
    controller.run_until(60_000);
    assert!(!controller.stopped());
    assert_eq!(
        controller.reported(),
        ["call ended, stops", "stop cancelled"]
    );
    controller.probe.microphone = Some(vec![]);
    controller.run_until(93_000);
    assert!(!controller.stopped(), "a new absence gets its full grace");
    controller.run_until(94_000);
    assert!(controller.stopped());
}

#[test]
fn a_returning_call_keep_recording_or_the_lab_off_cancels_the_stop() {
    // Teams comes back within the grace: nothing stops, and leaving
    // again arms a new stop.
    let mut controller = Controller::new(true, &[TEAMS]);
    controller.command(ControllerCommand::Record(offered(TEAMS), None));
    assert!(controller.started());
    leave_at_2_s(&mut controller);
    controller.run_until(10_000);
    controller.probe.microphone = Some(vec![TEAMS]);
    controller.run_until(60_000);
    assert!(!controller.stopped());
    assert_eq!(
        controller.reported(),
        ["call ended, stops", "stop cancelled"]
    );
    controller.probe.microphone = Some(vec![]);
    controller.run_until(93_000);
    assert!(!controller.stopped());
    controller.run_until(94_000);
    assert!(controller.stopped());

    // Keep recording: this absence never stops it; the capsule closes
    // itself, and the controller restores its recording indicator.
    let mut controller = Controller::new(true, &[TEAMS]);
    controller.command(ControllerCommand::Record(offered(TEAMS), None));
    assert!(controller.started());
    leave_at_2_s(&mut controller);
    controller.run_until(10_000);
    controller.command(ControllerCommand::KeepRecording);
    controller.run_until(120_000);
    assert!(!controller.stopped());
    assert_eq!(
        controller.reported(),
        ["call ended, stops", "recording continues"]
    );

    // The lab turned off mid-countdown: the call is still away.
    let mut controller = Controller::new(true, &[TEAMS]);
    controller.command(ControllerCommand::Record(offered(TEAMS), None));
    assert!(controller.started());
    leave_at_2_s(&mut controller);
    controller.run_until(10_000);
    controller.lab_on = false;
    controller.run_until(120_000);
    assert!(!controller.stopped());
    assert_eq!(
        controller.reported(),
        ["call ended, stops", "stop cancelled"]
    );
}
