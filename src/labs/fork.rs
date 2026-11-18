//! Runtime switches for fork features. Kept separate from upstream's packed flags.
use super::Feature;
#[cfg(not(test))]
use crate::app_settings::AppSettings;
use crate::app_settings::LabsSettings;
use std::sync::atomic::{AtomicU64, Ordering};
static STATE: AtomicU64 = AtomicU64::new(0);
fn encode(settings: &LabsSettings) -> u64 {
    let policy = crate::policy::Policy::current();
    let mut value = 1;
    if settings.local_polish
        && Feature::LocalPolish.shipped()
        && policy.labs_lock(Feature::LocalPolish).is_none()
    {
        value |= 1 << 1;
    }
    if settings.provider_fallback
        && Feature::ProviderFallback.shipped()
        && policy.labs_lock(Feature::ProviderFallback).is_none()
    {
        value |= 1 << 2;
    }
    if settings.save_dictation_audio
        && Feature::SaveDictationAudio.shipped()
        && policy.labs_lock(Feature::SaveDictationAudio).is_none()
    {
        value |= 1 << 3;
    }
    if settings.profile_sync
        && Feature::ProfileSync.shipped()
        && policy.labs_lock(Feature::ProfileSync).is_none()
    {
        value |= 1 << 4;
    }
    if settings.meeting_snapshots
        && Feature::MeetingSnapshots.shipped()
        && policy.labs_lock(Feature::MeetingSnapshots).is_none()
    {
        value |= 1 << 5;
    }
    if settings.meeting_speaker_names
        && Feature::MeetingSpeakerNames.shipped()
        && policy.labs_lock(Feature::MeetingSpeakerNames).is_none()
    {
        value |= 1 << 6;
    }
    value
}
pub(super) fn apply(settings: &LabsSettings) {
    if !settings.meeting_snapshots {
        crate::meeting::snapshots::disable_all();
    }
    STATE.store(encode(settings), Ordering::Release);
}
fn on(bit: u64) -> bool {
    let mut state = STATE.load(Ordering::Acquire);
    if state == 0 {
        #[cfg(not(test))]
        let settings = AppSettings::saved_labs();
        #[cfg(test)]
        let settings = LabsSettings::default();
        let initial = encode(&settings);
        state = match STATE.compare_exchange(0, initial, Ordering::AcqRel, Ordering::Acquire) {
            Ok(_) => initial,
            Err(current) => current,
        };
    }
    state & bit != 0
}
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub(crate) fn local_polish() -> bool {
    Feature::LocalPolish.on_this_os() && on(1 << 1)
}
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub(crate) fn provider_fallback() -> bool {
    Feature::ProviderFallback.on_this_os() && on(1 << 2)
}
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub(crate) fn save_dictation_audio() -> bool {
    Feature::SaveDictationAudio.on_this_os() && on(1 << 3)
}
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub(crate) fn profile_sync() -> bool {
    Feature::ProfileSync.on_this_os() && on(1 << 4)
}
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub(crate) fn meeting_snapshots() -> bool {
    Feature::MeetingSnapshots.on_this_os() && on(1 << 5)
}
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub(crate) fn meeting_speaker_names() -> bool {
    Feature::MeetingSpeakerNames.on_this_os() && on(1 << 6)
}
