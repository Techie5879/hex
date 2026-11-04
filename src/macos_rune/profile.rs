//! The existing main-thread settings owner handles a profile exchange atomically
//! with other app actions. The network worker never gets a control API token.
use super::*;
use crate::profile_sync::{host, listener};

impl Pump {
    pub(super) fn profile_sync_tick(&mut self) {
        if self.profile_check_at.elapsed() >= Duration::from_secs(1) {
            self.profile_check_at = Instant::now();
            let wanted = AppSettings::read().is_ok_and(|s| s.labs.profile_sync)
                && host::directory()
                    .and_then(|p| host::devices(&p))
                    .is_ok_and(|d| !d.is_empty());
            if !wanted && let Some(listener) = self.profile_listener.as_ref() {
                listener.stop();
            }
            if self
                .profile_listener
                .as_ref()
                .is_some_and(listener::Listener::finished)
            {
                self.profile_listener = None;
            }
            if wanted && self.profile_listener.is_none() {
                self.profile_listener = host::directory()
                    .and_then(|path| listener::Listener::start(path, self.profile_sender.clone()))
                    .inspect_err(|_| {
                        tracing::warn!("profile sync could not bind its loopback port")
                    })
                    .ok();
            }
        }
        // Bounded admission and one exchange per UI tick keep other app work moving.
        if let Ok(call) = self.profile_calls.try_recv() {
            let result = (|| -> Result<host::Reply> {
                if self.shutdown.load(Ordering::Acquire) {
                    return Ok(host::Reply::code(503));
                }
                let mut settings = AppSettings::read()?;
                host::exchange(
                    &host::directory()?,
                    call.exchange,
                    &mut settings,
                    |updated| updated.save(),
                )
            })();
            let response = match result {
                Ok(response) => response,
                Err(_) => host::Reply::code(503),
            };
            let _ = call.reply.try_send(response);
            let _ = self.runtime.send(Request::ReloadSettings);
        }
    }
}
