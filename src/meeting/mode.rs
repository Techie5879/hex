//! Changes to the active recording, serialized with its manifest owner.
//! No default is saved: a new recording always starts in Standard mode.

use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, SyncSender, TryRecvError};
use std::sync::{Arc, Mutex};

use color_eyre::eyre::{Result, eyre};

use super::{
    MeetingManifest, MeetingMode, MeetingStatus, read_manifest, transcript, write_manifest,
};

type Completion = Result<(), String>;

/// A UI action and its completion. Clones refer to the same action, so the
/// pane can show pending until both the manifest and live view are updated.
#[derive(Clone, Debug)]
pub(crate) struct Change {
    serial: u64,
    pub(crate) id: String,
    pub(crate) mode: MeetingMode,
    result: Arc<Mutex<Option<Completion>>>,
}

impl PartialEq for Change {
    fn eq(&self, other: &Self) -> bool {
        self.serial == other.serial
    }
}
impl Eq for Change {}

impl Change {
    pub(crate) fn new(id: String, mode: MeetingMode) -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        Self {
            serial: NEXT.fetch_add(1, Ordering::Relaxed),
            id,
            mode,
            result: Arc::default(),
        }
    }

    pub(crate) fn result(&self) -> Option<Completion> {
        self.result
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    #[cfg(test)]
    pub(crate) fn fixture_complete(&self, result: Completion) {
        self.complete(result);
    }

    fn complete(&self, result: Completion) {
        *self.result.lock().unwrap_or_else(|e| e.into_inner()) = Some(result);
    }
}

struct Active {
    id: String,
    sender: SyncSender<Change>,
}
static ACTIVE: Mutex<Option<Active>> = Mutex::new(None);

/// Called by the existing UI action hosts. Enqueuing is not completion.
/// The ID is checked again by the recording owner before any file changes.
pub(crate) fn request(change: Change) -> Completion {
    let active = ACTIVE.lock().unwrap_or_else(|e| e.into_inner());
    let result = active
        .as_ref()
        .filter(|active| active.id == change.id)
        .ok_or_else(|| "That meeting is no longer recording.".to_string())
        .and_then(|active| {
            active
                .sender
                .try_send(change.clone())
                .map_err(|_| "The recording is busy changing its mode. Try again.".to_string())
        });
    if let Err(error) = &result {
        change.complete(Err(error.clone()));
    }
    result
}

struct Pending {
    change: Change,
    previous: MeetingMode,
    updated: Receiver<Completion>,
}

/// Lives only while the recorder accepts changes, before finalization.
/// Only this owner writes the manifest, including provisional titles.
pub(super) struct Owner {
    id: String,
    requests: Receiver<Change>,
    pending: Option<Pending>,
}

impl Owner {
    pub(super) fn register(id: &str) -> Self {
        let (sender, requests) = mpsc::sync_channel(1);
        *ACTIVE.lock().unwrap_or_else(|e| e.into_inner()) = Some(Active {
            id: id.into(),
            sender,
        });
        Self {
            id: id.into(),
            requests,
            pending: None,
        }
    }

    pub(super) fn poll(
        &mut self,
        directory: &Path,
        manifest: &mut MeetingManifest,
        rolling: &transcript::Rolling,
    ) -> Result<()> {
        self.settle(directory, manifest, false)?;
        if self.pending.is_none()
            && let Ok(change) = self.requests.try_recv()
        {
            self.begin(directory, manifest, change, |mode| {
                rolling.change_mode(mode)
            })?;
        }
        Ok(())
    }

    fn begin(
        &mut self,
        directory: &Path,
        manifest: &mut MeetingManifest,
        change: Change,
        update: impl FnOnce(MeetingMode) -> Result<Receiver<Completion>>,
    ) -> Result<()> {
        if change.id != self.id
            || manifest.id != self.id
            || manifest.status != MeetingStatus::Recording
        {
            change.complete(Err("That meeting is no longer recording.".into()));
            return Ok(());
        }
        let previous = manifest.mode;
        let mut saved = manifest.clone();
        saved.mode = change.mode;
        // Do not notify the live worker until the durable choice exists.
        if let Err(error) = write_manifest(directory, &saved) {
            let message = format!("Could not save the meeting mode: {error}");
            // A directory sync can fail after the atomic rename. Confirm
            // the old choice or restore it before continuing capture.
            return match read_manifest(directory) {
                Ok(current) if current.mode == previous => {
                    change.complete(Err(message));
                    Ok(())
                }
                Ok(_) => rollback(directory, manifest, previous, change, message),
                Err(read) => {
                    let message = format!(
                        "{message}. Could not verify the saved mode ({read}); recording stopped."
                    );
                    change.complete(Err(message.clone()));
                    Err(eyre!(message))
                }
            };
        }
        *manifest = saved;
        let updated = match update(change.mode) {
            Ok(updated) => updated,
            Err(error) => {
                return rollback(directory, manifest, previous, change, error.to_string());
            }
        };
        self.pending = Some(Pending {
            change,
            previous,
            updated,
        });
        Ok(())
    }

    fn settle(
        &mut self,
        directory: &Path,
        manifest: &mut MeetingManifest,
        wait: bool,
    ) -> Result<()> {
        let Some(pending) = self.pending.as_ref() else {
            return Ok(());
        };
        let result = if wait {
            Some(pending.updated.recv().unwrap_or_else(|_| {
                Err("The live transcript stopped before applying the mode.".into())
            }))
        } else {
            match pending.updated.try_recv() {
                Ok(result) => Some(result),
                Err(TryRecvError::Disconnected) => Some(Err(
                    "The live transcript stopped before applying the mode.".into(),
                )),
                Err(TryRecvError::Empty) => None,
            }
        };
        if let Some(result) = result {
            let pending = self.pending.take().expect("a pending change was checked");
            match result {
                Ok(()) => pending.change.complete(Ok(())),
                Err(error) => {
                    rollback(directory, manifest, pending.previous, pending.change, error)?
                }
            }
        }
        Ok(())
    }

    /// Unregister before capture stops. Queued clicks cannot edit a finished
    /// meeting or the next recording. A change already saved is settled
    /// after the rolling worker drains its ordered queue.
    pub(super) fn close(&mut self) {
        let mut active = ACTIVE.lock().unwrap_or_else(|e| e.into_inner());
        if active.as_ref().is_some_and(|active| active.id == self.id) {
            *active = None;
        }
        for change in self.requests.try_iter() {
            change.complete(Err("That meeting is no longer recording.".into()));
        }
    }

    pub(super) fn finish(
        &mut self,
        directory: &Path,
        manifest: &mut MeetingManifest,
    ) -> Result<()> {
        self.settle(directory, manifest, true)
    }
}

impl Drop for Owner {
    fn drop(&mut self) {
        self.close();
        if let Some(pending) = self.pending.take() {
            pending.change.complete(Err(
                "Recording ended before the mode change completed.".into()
            ));
        }
    }
}

fn rollback(
    directory: &Path,
    manifest: &mut MeetingManifest,
    previous: MeetingMode,
    change: Change,
    error: String,
) -> Result<()> {
    let mut restored = manifest.clone();
    restored.mode = previous;
    if let Err(rollback) = write_manifest(directory, &restored) {
        let message = format!(
            "Could not update the live transcript ({error}) or restore the previous mode ({rollback}). Recording stopped; the saved mode needs review."
        );
        change.complete(Err(message.clone()));
        return Err(eyre!(message));
    }
    *manifest = restored;
    change.complete(Err(format!("Could not change the meeting mode: {error}")));
    Ok(())
}

#[cfg(test)]
mod tests;
