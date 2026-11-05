//! The macOS reader: finds the call app's one process, reads its windows
//! through Accessibility within a budget, and binds the call it finds.

use std::time::Duration;

use objc2_app_kit::NSRunningApplication;
use objc2_foundation::NSString;

use super::ax::{AxNode, Binder, Budget, Call, NoCall, unique_call};
use super::{Provider, Sample, Source, slack, teams};
use crate::accessibility::{self, AxElement};

/// Elements and time one reading may spend in the call app. Published
/// measurements put a Slack huddle walk at 70–140 ms and a Teams meeting
/// window at under 800 elements.
const NODES: usize = 6_000;
const TIME: Duration = Duration::from_millis(600);

impl AxNode for AxElement {
    fn read_failed(&self) -> bool {
        AxElement::read_failed(self)
    }
    fn role(&self) -> Option<String> {
        AxElement::role(self)
    }
    fn classes(&self) -> Vec<String> {
        AxElement::classes(self)
    }
    fn dom_id(&self) -> Option<String> {
        AxElement::dom_id(self)
    }
    fn title(&self) -> Option<String> {
        AxElement::title(self)
    }
    fn description(&self) -> Option<String> {
        AxElement::description(self)
    }
    fn value(&self) -> Option<String> {
        AxElement::value(self)
    }
    fn children(&self) -> Option<Vec<Self>> {
        AxElement::children(self, Budget::MAX_CHILDREN)
    }
    fn same_element(&self, other: &Self) -> bool {
        self.same(other)
    }
}

pub(super) struct Reader {
    providers: Vec<Provider>,
    /// The provider whose call this recording found; it stays for the rest
    /// of the recording.
    found: Option<Provider>,
    binder: Binder<AxElement>,
}

impl Reader {
    pub(super) fn new(providers: Vec<Provider>) -> Self {
        Self {
            providers,
            found: None,
            binder: Binder::default(),
        }
    }

    fn read_provider(&self, provider: Provider) -> Result<(i32, Call<AxElement>), NoCall> {
        let pid = only_process(provider.bundle_id())?;
        if self.providers.len() > 1 && self.found != Some(provider) {
            // A manual recording must resolve every competing app, even
            // after one provider was selected on a previous poll.
            match crate::microphone_activity::application_input_devices(provider.bundle_id()) {
                Ok(devices) if devices.is_empty() => return Err(NoCall::Absent),
                Ok(_) => {}
                Err(_) => return Err(NoCall::Unknown),
            }
        }
        // A window list that can't be read whole could hide the call.
        let windows = accessibility::application_windows(pid, provider == Provider::Teams)
            .ok_or(NoCall::Unknown)?;
        let mut budget = Budget::new(NODES, TIME);
        let call = match provider {
            Provider::Teams => teams::read(windows, &mut budget),
            Provider::Slack => slack::read(windows, &mut budget),
        }?;
        Ok((pid, call))
    }
}

impl Source for Reader {
    fn provider(&self) -> Option<Provider> {
        self.found
    }

    fn read(&mut self) -> Sample {
        if !accessibility::is_trusted() {
            return Sample::PermissionRequired;
        }
        objc2::rc::autoreleasepool(|_| {
            let calls = self
                .providers
                .iter()
                .copied()
                .map(|provider| self.read_provider(provider).map(|call| (provider, call)));
            match unique_call(calls) {
                Ok((provider, _)) if self.found.is_some_and(|found| found != provider) => {
                    self.binder.lose();
                    Sample::NoCall(NoCall::Unknown)
                }
                Ok((provider, (pid, call))) => {
                    self.found = Some(provider);
                    Sample::Call {
                        binding: self.binder.bind(pid, call.region),
                        reading: call.reading,
                    }
                }
                Err(reason) => {
                    self.binder.lose();
                    Sample::NoCall(reason)
                }
            }
        })
    }
}

/// The app's one running process; two copies are ambiguous.
fn only_process(bundle_id: &str) -> Result<i32, NoCall> {
    let running = NSRunningApplication::runningApplicationsWithBundleIdentifier(
        &NSString::from_str(bundle_id),
    );
    let pids = running
        .iter()
        .filter(|application| !application.isTerminated())
        .map(|application| application.processIdentifier())
        .collect::<Vec<_>>();
    match pids.as_slice() {
        [] => Err(NoCall::Absent),
        [pid] => Ok(*pid),
        _ => Err(NoCall::Unknown),
    }
}
