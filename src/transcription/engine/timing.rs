//! Native timed transcription on the engine-owned model.
use super::*;

/// Transcribes a complete 16 kHz clip with a held model, padded as a
/// dictation's clip is.
pub(crate) fn transcribe_clip(model: &mut HeldModel, samples: Vec<f32>) -> Result<String> {
    #[cfg(target_os = "macos")]
    return model.transcribe(samples);
    #[cfg(not(target_os = "macos"))]
    model.transcribe_clip(samples)
}

/// Transcribes a complete 16 kHz clip with a held model and its own native
/// timings, never estimated ones.
pub(crate) fn transcribe_clip_timed(
    model: &mut HeldModel,
    samples: Vec<f32>,
) -> Result<crate::transcription::transcript::Transcript, String> {
    model
        .transcribe_timed(samples)
        .map_err(|error| format!("{error:#}"))
}

/// The finest native timings a held model gives. Dictation never asks; the
/// local API answers `422` for a level the model cannot produce.
pub(crate) fn timestamp_level(
    model: &HeldModel,
) -> crate::transcription::transcript::TimestampLevel {
    model.timestamps()
}
