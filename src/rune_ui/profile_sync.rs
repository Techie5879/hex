//! Keep open editors in step with incoming profile changes and accepted saves.
use super::*;

impl Ui {
    pub(super) fn commit(&mut self, mut candidate: AppSettings) -> bool {
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
        // The accepted save may include incoming fields the open editors never
        // saw. Advance their projections too, or the next autosave deletes them.
        if self.vocabulary.project() != candidate.vocabulary {
            self.vocabulary.reload(&candidate.vocabulary);
        }
        let mut projected = candidate.clone();
        self.modes.project(&mut projected);
        if projected.dictation_processing != candidate.dictation_processing {
            self.modes.reload(&candidate);
        }
        self.settings = candidate;
        self.draft_since = None;
        self.settings_changed();
        true
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
