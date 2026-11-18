//! Keep open editors in step with incoming profile changes and accepted saves.
use super::*;

impl Ui {
    pub(super) fn rebase_profile_candidate(
        &self,
        candidate: &mut AppSettings,
    ) -> Result<(), String> {
        if cfg!(target_os = "macos") && matches!(self.service, Service::Live(_)) {
            let current = AppSettings::read().map_err(|error| format!("{error:#}"))?;
            if current.labs.profile_sync || self.settings.labs.profile_sync {
                crate::profile_sync::adapter::rebase_editor(&self.settings, candidate, &current);
            }
        }
        Ok(())
    }

    /// Refresh only fields changed by rebasing, not ordinary UI edits whose
    /// callbacks still own their editor mutations (such as adding a row).
    pub(super) fn sync_rebased_editors(&mut self, before: &AppSettings, after: &AppSettings) {
        if before.vocabulary != after.vocabulary {
            self.vocabulary.reload(&after.vocabulary);
        }
        if before.dictation_processing.default_mode.post_processing
            != after.dictation_processing.default_mode.post_processing
            && self.modes.selected == modes::Selection::Default
        {
            self.modes.adopt(after, false);
        }
    }
}
