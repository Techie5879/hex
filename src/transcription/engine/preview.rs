//! Bounded, replaceable Whisper previews. They never become final output.
use super::*;

const RATE: usize = 16_000;
const WINDOW: usize = RATE * 12;
const MIN_AUDIO: usize = RATE * 2;
const INTERVAL: Duration = Duration::from_secs(1);

pub(super) fn supported(selection: &crate::transcription::models::TranscriptionSelection) -> bool {
    use crate::transcription::models::{ModelRuntime, validate};
    validate(selection).is_ok_and(|model| {
        matches!(model.runtime,
        ModelRuntime::Gguf(artifact) if artifact.architecture == "whisper")
    })
}

pub(crate) struct Input {
    enabled: fn() -> bool,
    audio: VecDeque<f32>,
    version: u64,
    wake_pending: bool,
}

impl Input {
    pub(super) fn new(enabled: fn() -> bool) -> Self {
        Self {
            enabled,
            audio: VecDeque::new(),
            version: 0,
            wake_pending: false,
        }
    }

    /// Coalesce all feeds into one bounded buffer and at most one queued wake.
    pub(super) fn push(&mut self, samples: &[f32]) -> bool {
        let samples = &samples[samples.len().saturating_sub(WINDOW)..];
        let remove = self
            .audio
            .len()
            .saturating_add(samples.len())
            .saturating_sub(WINDOW);
        self.audio.drain(..remove);
        self.audio.extend(samples);
        self.version += 1;
        !std::mem::replace(&mut self.wake_pending, true)
    }

    fn snapshot(&mut self) -> (u64, Vec<f32>) {
        self.wake_pending = false;
        (self.version, self.audio.iter().copied().collect())
    }
}

fn audible(samples: &[f32]) -> bool {
    // Same conservative level floor as meeting chunk admission. No preview
    // inference for pure silence; final transcription retains its own gates.
    samples
        .chunks(320)
        .any(|frame| frame.iter().map(|x| x * x).sum::<f32>() / frame.len() as f32 >= 0.000001)
}

fn terminal(command: Command, id: u64, deferred: &mut VecDeque<Command>) -> Option<Command> {
    match command {
        Command::Cancel { stream } if stream == id => Some(Command::Cancel { stream }),
        Command::Feed { .. } | Command::Cancel { .. } => None,
        Command::Background { work, .. } => {
            work(None);
            None
        }
        foreground @ Command::Foreground { .. } => {
            deferred.push_back(foreground);
            None
        }
        Command::BeginYielding { yielding, .. } => {
            yielding.close();
            None
        }
        Command::FinishYielding { reply, .. } => {
            let _ = reply.send(Err(PREEMPTED.into()));
            None
        }
        command => Some(command),
    }
}

pub(super) fn drive(
    id: u64,
    cancel: &CancelToken,
    input: &Mutex<Input>,
    partials: Option<&Partials>,
    commands: &Receiver<Command>,
    deferred: &mut VecDeque<Command>,
    mut decode: impl FnMut(Vec<f32>) -> Result<String, String>,
) -> Option<Command> {
    let mut decoded = 0;
    let mut next_decode = Instant::now();
    let mut failed = false;
    loop {
        if cancel.is_cancelled() {
            return None;
        }
        // Commands always take precedence over starting another inference.
        match commands.recv_timeout(Duration::from_millis(40)) {
            Ok(command) => {
                if let Some(command) = terminal(command, id, deferred) {
                    return Some(command);
                }
            }
            Err(RecvTimeoutError::Disconnected) => return None,
            Err(RecvTimeoutError::Timeout) => {}
        }
        for _ in 0..128 {
            match commands.try_recv() {
                Ok(command) => {
                    if let Some(command) = terminal(command, id, deferred) {
                        return Some(command);
                    }
                }
                Err(_) => break,
            }
        }
        let enabled = input.lock().unwrap_or_else(|e| e.into_inner()).enabled;
        if !enabled() {
            if let Some(partials) = partials {
                partials.publish(Partial::default());
            }
            continue;
        }
        if failed || Instant::now() < next_decode {
            continue;
        }
        let (version, samples) = input.lock().unwrap_or_else(|e| e.into_inner()).snapshot();
        if version == decoded || samples.len() < MIN_AUDIO {
            continue;
        }
        decoded = version;
        if !audible(&samples) {
            continue;
        }
        let started = Instant::now();
        let result = decode(samples);
        next_decode = Instant::now() + INTERVAL;
        if cancel.is_cancelled() {
            return None;
        }
        if !enabled() {
            continue;
        }
        match result {
            Ok(text) => {
                if let Some(partials) = partials {
                    partials.publish(Partial {
                        stable: String::new(),
                        tentative: text.trim().into(),
                    });
                }
                tracing::debug!(
                    elapsed_ms = started.elapsed().as_millis(),
                    "dictation preview updated"
                );
            }
            Err(error) => {
                // A preview error must never fail or repeatedly retry a final
                // dictation. Keep capturing and wait for its ordinary finish.
                tracing::warn!(%error, "dictation preview unavailable; final transcription remains active");
                if let Some(partials) = partials {
                    partials.publish(Partial::default());
                }
                failed = true;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn whisper_engine(live_text: bool) -> (TranscriptionEngine, Receiver<Command>) {
        let (mut engine, receiver) = TranscriptionEngine::for_test_without_streaming();
        *engine.held.lock().unwrap() = Some(crate::transcription::models::TranscriptionSelection {
            model: crate::transcription::models::TranscriptionModelId::WhisperLargeV3Turbo,
            language: "en".into(),
        });
        if live_text {
            engine = engine.with_live_text();
        }
        (engine, receiver)
    }

    #[test]
    fn whisper_preview_requires_live_text_and_a_supported_model() {
        let (engine, _receiver) = whisper_engine(false);
        assert!(engine.begin().is_none());
        let (engine, _receiver) = TranscriptionEngine::for_test_without_streaming();
        assert!(engine.with_live_text().begin().is_none());
        let (engine, _receiver) = whisper_engine(true);
        assert!(engine.begin().is_some());
    }

    #[test]
    fn preview_finish_cancels_decoding_and_sends_the_entire_clip_offline() {
        let (engine, receiver) = whisper_engine(true);
        let mut stream = engine.begin().unwrap();
        let cancel = stream.cancel.clone();
        stream.feed(&vec![0.1; WINDOW * 2]);
        let complete = vec![0.2; WINDOW * 3];
        let pending = engine
            .finish_with(
                Some(stream),
                complete.clone(),
                Box::new(|_, _| Ok("final cleaned separately".into())),
            )
            .unwrap();
        assert!(cancel.is_cancelled());
        assert!(pending.stream_cancel.is_none());
        assert!(matches!(
            receiver.recv().unwrap(),
            Command::Begin {
                preview: Some(_),
                ..
            }
        ));
        assert!(
            matches!(receiver.recv().unwrap(), Command::Feed { samples, .. } if samples.is_empty())
        );
        assert!(
            matches!(receiver.recv().unwrap(), Command::Finish { stream: None, clip, .. } if clip == complete)
        );
    }

    #[test]
    fn dropping_a_preview_cancels_its_inflight_decode() {
        let (engine, _receiver) = whisper_engine(true);
        let stream = engine.begin().unwrap();
        let cancel = stream.cancel.clone();
        drop(stream);
        assert!(cancel.is_cancelled());
    }

    #[test]
    fn preview_publishes_tentative_words_without_finalizing() {
        let (sender, receiver) = mpsc::channel();
        let input = Mutex::new(Input::new(|| true));
        input.lock().unwrap().push(&vec![0.1; MIN_AUDIO]);
        let partials = Partials::default();
        sender
            .send(Command::Feed {
                stream: 1,
                samples: vec![],
            })
            .unwrap();
        let result = drive(
            1,
            &CancelToken::new(),
            &input,
            Some(&partials),
            &receiver,
            &mut VecDeque::new(),
            |samples| {
                assert_eq!(samples.len(), MIN_AUDIO);
                sender.send(Command::Shutdown).unwrap();
                Ok("  The budget is 7300, not 4300.  ".into())
            },
        );
        assert!(matches!(result, Some(Command::Shutdown)));
        let partial = partials.latest().unwrap();
        assert!(partial.stable.is_empty());
        assert_eq!(partial.tentative, "The budget is 7300, not 4300.");
    }

    #[test]
    fn a_preview_error_does_not_consume_the_final_command() {
        let (sender, receiver) = mpsc::channel();
        let input = Mutex::new(Input::new(|| true));
        input.lock().unwrap().push(&vec![0.1; MIN_AUDIO]);
        let partials = Partials::default();
        let (reply, _) = mpsc::channel();
        sender
            .send(Command::Feed {
                stream: 1,
                samples: vec![],
            })
            .unwrap();
        let result = drive(
            1,
            &CancelToken::new(),
            &input,
            Some(&partials),
            &receiver,
            &mut VecDeque::new(),
            |_| {
                sender
                    .send(Command::Finish {
                        stream: None,
                        clip: vec![0.3; 10],
                        offline: Box::new(|_, _| Ok("final".into())),
                        job: Job {
                            cancel: CancelToken::new(),
                            reply: reply.clone(),
                        },
                    })
                    .unwrap();
                Err("preview failed".into())
            },
        );
        assert!(matches!(result, Some(Command::Finish { clip, .. }) if clip.len() == 10));
        assert!(
            partials
                .latest()
                .is_none_or(|partial| partial.stable.is_empty() && partial.tentative.is_empty())
        );
    }

    #[test]
    fn feeds_are_bounded_and_coalesced() {
        let mut input = Input::new(|| true);
        assert!(input.push(&vec![0.1; WINDOW]));
        for _ in 0..100 {
            assert!(!input.push(&[0.2; 160]));
        }
        let (_, audio) = input.snapshot();
        assert_eq!(audio.len(), WINDOW);
        assert_eq!(*audio.last().unwrap(), 0.2);
        assert!(input.push(&[0.3; 160]));
    }
    #[test]
    fn silence_does_not_decode() {
        assert!(!audible(&vec![0.; MIN_AUDIO]));
        assert!(audible(&vec![0.01; MIN_AUDIO]));
    }
    #[test]
    fn cancellation_during_decode_publishes_no_late_words() {
        let (sender, receiver) = mpsc::channel();
        let input = Mutex::new(Input::new(|| true));
        input.lock().unwrap().push(&vec![0.1; MIN_AUDIO]);
        let cancel = CancelToken::new();
        let partials = Partials::default();
        sender
            .send(Command::Feed {
                stream: 1,
                samples: vec![],
            })
            .unwrap();
        let result = drive(
            1,
            &cancel,
            &input,
            Some(&partials),
            &receiver,
            &mut VecDeque::new(),
            |_| {
                cancel.cancel();
                Ok("late words".into())
            },
        );
        assert!(result.is_none());
        assert_eq!(partials.latest(), None);
    }
    #[test]
    fn finish_preempts_queued_preview() {
        let (sender, receiver) = mpsc::channel();
        let input = Mutex::new(Input::new(|| true));
        input.lock().unwrap().push(&vec![0.1; MIN_AUDIO]);
        sender
            .send(Command::Feed {
                stream: 1,
                samples: vec![],
            })
            .unwrap();
        sender.send(Command::Shutdown).unwrap();
        let result = drive(
            1,
            &CancelToken::new(),
            &input,
            None,
            &receiver,
            &mut VecDeque::new(),
            |_| panic!("must not decode"),
        );
        assert!(matches!(result, Some(Command::Shutdown)));
    }
}
