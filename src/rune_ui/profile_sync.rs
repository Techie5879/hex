//! Keep open editors in step with incoming profile changes and accepted saves.
use super::*;

impl Ui {
    pub(super) fn commit(&mut self, mut candidate: AppSettings) -> bool {
        let before_rebase = candidate.clone();
        if cfg!(target_os = "macos") && matches!(self.service, Service::Live(_)) {
            match AppSettings::read() {
                Ok(current) if current.labs.profile_sync || self.settings.labs.profile_sync => {
                    crate::profile_sync::adapter::rebase_editor(
                        &self.settings,
                        &mut candidate,
                        &current,
                    );
                }
                Ok(_) => {}
                Err(error) => {
                    self.error = Some(format!("{error:#}"));
                    return false;
                }
            }
        }
        match &mut self.service {
            Service::Live(backend) => {
                if let Err(error) = backend.save(&candidate) {
                    self.error = Some(format!("{error:#}"));
                    return false;
                }
            }
            Service::Fixture(_) => self.saved += 1,
            Service::Pending(_) => {}
        }
        self.error = None;
        self.sync_rebased_editors(&before_rebase, &candidate);
        self.settings = candidate;
        self.draft_since = None;
        self.settings_changed();
        true
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
            self.modes.reload(after);
        }
    }

    /// Adopts settings the host saved, unless they are the stale copy of an
    /// edit this window just made. An editor with typing not yet saved
    /// keeps it; every other editor and setting is adopted beneath, so the
    /// draft's save never writes back what another writer (`hex settings
    /// set`, a capture's saved rule) changed meanwhile.
    pub(super) fn adopt(&mut self, service: AppSettings) -> bool {
        if self.service_settings.as_ref() == Some(&service) {
            return false;
        }
        let changed = service != self.settings;
        if changed {
            let drafts = self.drafts();
            if (service.labs.profile_sync || self.settings.labs.profile_sync)
                && (drafts.modes || drafts.vocabulary)
            {
                // Retained drafts still have the OLD settings as their baseline.
                // Carry the person's edits over the incoming profile before moving
                // that baseline, or missing new terms look like deliberate deletes.
                let mut rebased = self.drafted();
                crate::profile_sync::adapter::rebase_editor(&self.settings, &mut rebased, &service);
                if drafts.modes {
                    self.modes.reload(&rebased);
                }
                if drafts.vocabulary {
                    self.vocabulary.reload(&rebased.vocabulary);
                }
            }
            if !drafts.modes && service.dictation_processing != self.settings.dictation_processing {
                self.modes.reload(&service);
            }
            if !drafts.vocabulary && service.vocabulary != self.settings.vocabulary {
                self.vocabulary.reload(&service.vocabulary);
            }
            self.settings = service.clone();
            self.settings_changed();
        }
        self.service_settings = Some(service);
        changed
    }
}
