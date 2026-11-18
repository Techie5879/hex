//! Recording origin and call-presence input shared with controller tests.
use super::*;

/// How a recording began, which decides whether Labs › Stop when the call
/// ends may stop it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Origin {
    /// Always record started it with no click.
    Automatic,
    /// The user accepted a detected call's offer (Record, or … › Always
    /// record): its call app was on the microphone, and it is the one
    /// watched.
    AcceptedOffer,
    /// The menu bar, `hex meetings record`, or a calendar reminder: no call
    /// was seen on the microphone, so none may end it.
    Requested,
}

impl Origin {
    /// A `Record` command's origin: an offered call's candidate names its
    /// source and app; a manual request names neither.
    pub(super) fn of_record(candidate: &MeetingCandidate) -> Self {
        if candidate.source != MeetingSource::Manual && candidate.application.is_some() {
            Self::AcceptedOffer
        } else {
            Self::Requested
        }
    }

    /// Whether the recording may stop once its call ends, with the lab on.
    pub(super) fn stops_when_call_ends(self) -> bool {
        matches!(self, Self::Automatic | Self::AcceptedOffer)
    }
}

/// Starts the recorder thread; tests substitute one that records nothing.
pub(super) type Recorder = fn(
    meeting::MeetingStart,
    &AtomicBool,
    Option<SyncSender<meeting::RecordingUpdate>>,
) -> Result<meeting::MeetingManifest>;

/// Whether a recording's call app is still in the call; tests substitute
/// their own readings.
pub(super) trait CallProbe {
    /// Whether `application` holds the microphone; an error when the read
    /// failed.
    fn on_microphone(&mut self, application: &str) -> Result<bool>;
    /// Whether `application` plays audio; a failed read counts as playing.
    fn playing(&mut self, application: &str) -> bool;
}

/// The Mac's microphone and output activity.
pub(super) struct SystemCallProbe;

impl CallProbe for SystemCallProbe {
    fn on_microphone(&mut self, application: &str) -> Result<bool> {
        Ok(active_microphone_applications()?
            .iter()
            .any(|candidate| candidate.canonical_bundle_id() == Some(application)))
    }

    fn playing(&mut self, application: &str) -> bool {
        call_app_playing(application)
    }
}
