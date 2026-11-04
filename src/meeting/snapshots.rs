//! Local-only meeting images. One session owns capture, selection, its index,
//! and the controls; the UI reads a projection of that same state.

use std::path::PathBuf;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum Status {
    #[default]
    Off,
    Waiting,
    Capturing,
    Paused,
    PermissionRequired,
    LimitReached,
    Failed,
    Complete,
}

impl Status {
    pub fn label(self) -> &'static str {
        match self {
            Self::Off => "Snapshots start with the next recording",
            Self::Waiting => "Waiting for a Teams screen share",
            Self::Capturing => "Taking snapshots",
            Self::Paused => "Snapshots paused",
            Self::PermissionRequired => "Allow Screen Recording to take snapshots",
            Self::LimitReached => "Snapshot storage limit reached",
            Self::Failed => "Snapshots unavailable",
            Self::Complete => "Saved snapshots",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Snapshot {
    pub id: u64,
    pub at_ms: u64,
    pub until_ms: u64,
    pub path: PathBuf,
    pub thumbnail: PathBuf,
    pub width: u32,
    pub height: u32,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct View {
    pub active: bool,
    pub status: Status,
    pub images: Vec<Snapshot>,
    pub has_gaps: bool,
}

mod native;
mod selector;
mod store;

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

struct SessionState {
    store: store::Store,
    status: Status,
    paused: bool,
    generation: u64,
    disabled: bool,
}

impl SessionState {
    fn disable(&mut self) {
        self.store.mark_gap();
        self.disabled = true;
        self.generation += 1;
        self.status = Status::Off;
    }

    fn may_publish(&self, generation: u64) -> bool {
        !self.paused && !self.disabled && self.generation == generation
    }

    fn unavailable(&mut self, reason: native::Unavailable, established: bool) {
        if established && reason.interrupted {
            self.store.mark_gap();
        }
        self.status = reason.status;
    }

    fn rejected_binding(&mut self) {
        self.store.mark_gap();
        self.status = Status::Waiting;
    }
}

type Shared = Arc<Mutex<SessionState>>;
fn sessions() -> &'static Mutex<HashMap<String, Shared>> {
    static SESSIONS: OnceLock<Mutex<HashMap<String, Shared>>> = OnceLock::new();
    SESSIONS.get_or_init(|| Mutex::new(HashMap::new()))
}
fn lock<T>(value: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    value.lock().unwrap_or_else(|e| e.into_inner())
}
fn lookup(id: &str) -> Option<Shared> {
    lock(sessions()).get(id).cloned()
}
fn directory(id: &str) -> Option<PathBuf> {
    if id.is_empty()
        || id.len() > 100
        || !id.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'-')
    {
        return None;
    }
    super::root().ok().map(|root| root.join(id))
}

/// Reads the active manager's projection, or the same durable index after it
/// closes. Called by the Meetings background reader, never the paint thread.
pub fn read(id: &str) -> View {
    if let Some(state) = lookup(id) {
        let state = lock(&state);
        let mut view = state.store.view(state.status);
        view.active = true;
        return view;
    }
    let Some(directory) = directory(id) else {
        return View::default();
    };
    match store::Store::load(&directory) {
        Ok(store) => store.view(Status::Complete),
        Err(_) if !directory.join("snapshots").exists() => View::default(),
        Err(_) => View {
            active: false,
            status: Status::Failed,
            images: Vec::new(),
            has_gaps: true,
        },
    }
}

/// The recording capsule needs only the live state, not image paths or I/O.
pub(crate) fn active_status(id: &str) -> Option<Status> {
    let state = lookup(id)?;
    Some(lock(&state).status)
}

pub fn set_paused(id: &str, paused: bool) -> std::result::Result<(), String> {
    let state = lock(sessions())
        .get(id)
        .cloned()
        .ok_or("This meeting is not taking snapshots")?;
    let mut state = lock(&state);
    if state.disabled {
        return Err("Snapshots are off for this recording".into());
    }
    state.paused = paused;
    if paused {
        state.store.mark_gap();
    }
    state.generation += 1;
    state.status = if paused {
        Status::Paused
    } else {
        Status::Waiting
    };
    Ok(())
}

/// Called synchronously when the saved Labs switch goes off. Taking the same
/// lock as publication ensures no pending image can appear after it returns.
pub(crate) fn disable_all() {
    for state in lock(sessions()).values() {
        let mut state = lock(state);
        state.disable();
    }
}

/// Serializes with the worker. Publishing the index before removing an
/// unreferenced image means an interrupted deletion never leaves a broken row.
pub fn delete(id: &str, image_id: u64) -> std::result::Result<(), String> {
    if let Some(state) = lookup(id) {
        let mut state = lock(&state);
        state.store.delete(image_id).map_err(|e| e.to_string())?;
        state.generation += 1;
        if state.status == Status::LimitReached && !state.disabled {
            state.status = if state.paused {
                Status::Paused
            } else {
                Status::Waiting
            };
        }
        return Ok(());
    }
    static COMPLETED_WRITES: Mutex<()> = Mutex::new(());
    let _guard = lock(&COMPLETED_WRITES);
    let directory = directory(id).ok_or("Invalid meeting")?;
    store::Store::load(&directory)
        .and_then(|mut store| store.delete(image_id))
        .map_err(|e| e.to_string())
}

pub(super) struct Session {
    id: String,
    stop: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
}
impl Session {
    /// The gate is checked here and again before each native capture and write.
    /// Enabling it part-way through a recording takes effect on the next one.
    pub(super) fn start(
        id: &str,
        directory: &std::path::Path,
        clock: Instant,
        application: Option<&str>,
    ) -> Option<Self> {
        if !cfg!(target_os = "macos")
            || !crate::labs::meeting_snapshots()
            || application.is_some_and(|app| app != "com.microsoft.teams2")
        {
            return None;
        }
        let state = match store::Store::new(directory) {
            Ok(store) => Arc::new(Mutex::new(SessionState {
                store,
                status: Status::Waiting,
                paused: false,
                generation: 0,
                disabled: false,
            })),
            Err(error) => {
                tracing::warn!(%error,"could not open meeting snapshots");
                let state = Arc::new(Mutex::new(SessionState {
                    store: store::Store::unavailable(directory),
                    status: Status::Failed,
                    paused: false,
                    generation: 0,
                    disabled: true,
                }));
                lock(sessions()).insert(id.to_owned(), state);
                return Some(Self {
                    id: id.to_owned(),
                    stop: Arc::new(AtomicBool::new(false)),
                    worker: None,
                });
            }
        };
        let stop = Arc::new(AtomicBool::new(false));
        {
            let mut registry = lock(sessions());
            // Pair admission with disable_all's registry lock. A setting
            // changed during directory setup must not admit a late worker.
            if !crate::labs::meeting_snapshots() {
                return None;
            }
            registry.insert(id.to_owned(), state.clone());
        }
        let worker_stop = stop.clone();
        let worker = thread::Builder::new()
            .name("meeting-snapshots".into())
            .spawn(move || run(state, worker_stop, clock));
        match worker {
            Ok(worker) => Some(Self {
                id: id.to_owned(),
                stop,
                worker: Some(worker),
            }),
            Err(error) => {
                lock(sessions()).remove(id);
                tracing::warn!(%error,"could not start meeting snapshots");
                None
            }
        }
    }
}
impl Drop for Session {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
        lock(sessions()).remove(&self.id);
    }
}

fn run(shared: Shared, stop: Arc<AtomicBool>, clock: Instant) {
    crate::transcription::background_priority::deprioritize_background_thread();
    let mut selector = selector::Selector::default();
    let mut observed_generation = 0;
    let mut pinned_window = None;
    while !stop.load(Ordering::Acquire) {
        if !crate::labs::meeting_snapshots() {
            lock(&shared).status = Status::Off;
            break;
        }
        let generation = {
            let state = lock(&shared);
            if state.disabled {
                break;
            }
            if state.paused || state.status == Status::LimitReached {
                drop(state);
                selector.reset();
                wait(&stop);
                continue;
            }
            state.generation
        };
        if observed_generation != generation {
            selector.reset();
            observed_generation = generation;
        }
        // Dictation always wins. Skipped screenshot work is not queued.
        if crate::transcription::engine::dictation_busy() {
            selector.reset();
            if pinned_window.is_some() {
                lock(&shared).store.mark_gap();
            }
            wait(&stop);
            continue;
        }
        let result = native::capture();
        let at = clock.elapsed().as_millis() as u64;
        let mut state = lock(&shared);
        let dictating = crate::transcription::engine::dictation_busy();
        if dictating && pinned_window.is_some() {
            state.store.mark_gap();
        }
        if stop.load(Ordering::Acquire)
            || !state.may_publish(generation)
            || !crate::labs::meeting_snapshots()
            || dictating
        {
            selector.reset();
            drop(state);
            wait(&stop);
            continue;
        }
        match result {
            Ok(frame)
                if pinned_window.is_none_or(|window| window == (frame.window, frame.binding)) =>
            {
                pinned_window = Some((frame.window, frame.binding));
                state.status = if state.store.has_images() {
                    Status::Capturing
                } else {
                    Status::Waiting
                };
                // Selection and PNG encoding are CPU work, not state mutation.
                // Release the state mutex and revalidate publication afterwards.
                drop(state);
                let chosen = selector.observe(frame, at);
                let prepared = chosen
                    .as_ref()
                    .map(|chosen| store::Prepared::new(&chosen.frame));
                state = lock(&shared);
                if stop.load(Ordering::Acquire)
                    || !state.may_publish(generation)
                    || !crate::labs::meeting_snapshots()
                    || crate::transcription::engine::dictation_busy()
                {
                    selector.reset();
                } else if let Some(chosen) = chosen {
                    let result = prepared
                        .expect("chosen frame was prepared")
                        .and_then(|prepared| state.store.add_prepared(&prepared, chosen.since, at));
                    match result {
                        Ok(true) => {
                            if chosen.sparse {
                                state.store.mark_gap();
                            }
                            selector.remember(chosen.frame, at);
                            state.status = if state.store.has_images() {
                                Status::Capturing
                            } else {
                                Status::Waiting
                            };
                        }
                        Ok(false) => {
                            state.store.mark_gap();
                            state.status = Status::LimitReached;
                            selector.reset();
                        }
                        Err(error) => {
                            state.store.mark_gap();
                            state.status = Status::Failed;
                            selector.reset();
                            tracing::warn!(%error,"could not save meeting snapshot");
                        }
                    }
                }
            }
            Ok(_) => {
                state.rejected_binding();
                selector.reset();
            }
            Err(reason) => {
                state.unavailable(reason, pinned_window.is_some());
                selector.reset();
            }
        }
        drop(state);
        wait(&stop);
    }
}
fn wait(stop: &AtomicBool) {
    for _ in 0..10 {
        if stop.load(Ordering::Acquire) {
            break;
        }
        thread::sleep(Duration::from_millis(50));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn a_disabled_session_never_publishes_a_pending_frame_after_reenable() {
        let directory =
            std::env::temp_dir().join(format!("hex-snapshot-generation-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        let mut state = SessionState {
            store: store::Store::new(&directory).unwrap(),
            status: Status::Capturing,
            paused: false,
            generation: 4,
            disabled: false,
        };
        assert!(state.may_publish(4));
        state.disable();
        assert!(!state.may_publish(4));
        assert!(!state.may_publish(5));
        assert_eq!(state.status, Status::Off);
        std::fs::remove_dir_all(directory).unwrap();
    }
    #[test]
    fn a_pause_and_resume_invalidates_the_prior_capture() {
        let directory =
            std::env::temp_dir().join(format!("hex-snapshot-pause-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        let mut state = SessionState {
            store: store::Store::new(&directory).unwrap(),
            status: Status::Paused,
            paused: true,
            generation: 1,
            disabled: false,
        };
        assert!(!state.may_publish(1));
        state.paused = false;
        state.generation += 1;
        assert!(!state.may_publish(1));
        assert!(state.may_publish(2));
        std::fs::remove_dir_all(directory).unwrap();
    }
    #[test]
    fn interruptions_after_capture_and_rejected_bindings_retain_a_gap() {
        for rejected_binding in [false, true] {
            let directory = std::env::temp_dir().join(format!(
                "hex-snapshot-interruption-{}-{rejected_binding}",
                std::process::id()
            ));
            std::fs::create_dir_all(&directory).unwrap();
            let mut state = SessionState {
                store: store::Store::new(&directory).unwrap(),
                status: Status::Waiting,
                paused: false,
                generation: 0,
                disabled: false,
            };
            // Waiting before a share has ever been established is ordinary.
            state.unavailable(
                native::Unavailable {
                    status: Status::Waiting,
                    interrupted: false,
                },
                false,
            );
            assert!(!state.store.view(state.status).has_gaps);
            state.status = Status::Capturing;
            if rejected_binding {
                state.rejected_binding();
            } else {
                state.unavailable(
                    native::Unavailable {
                        status: Status::Waiting,
                        interrupted: true,
                    },
                    true,
                );
            }
            state.status = Status::Capturing;
            assert!(
                store::Store::load(&directory)
                    .unwrap()
                    .view(Status::Complete)
                    .has_gaps
            );
            std::fs::remove_dir_all(directory).unwrap();
        }
    }
}
