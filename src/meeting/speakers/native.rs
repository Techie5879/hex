//! The macOS reader: finds the call app's one process, reads its windows
//! through Accessibility within a budget, and binds the call it finds.

use std::time::Duration;

use objc2_app_kit::NSRunningApplication;
use objc2_foundation::NSString;

use super::ax::{AxNode, Binder, Budget, Call, NoCall};
use super::{Provider, Sample, Source, slack, teams};
use crate::accessibility::{self, AxElement};

/// Elements and time one reading may spend in the call app. Published
/// measurements put a Slack huddle walk at 70–140 ms and a Teams meeting
/// window at under 800 elements.
const NODES: usize = 6_000;
const TIME: Duration = Duration::from_millis(600);

impl AxNode for AxElement {
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
        // A window list that can't be read whole could hide the call.
        let windows = accessibility::application_windows(pid, provider == Provider::Teams)
            .ok_or(NoCall::Unknown)?;
        let mut budget = Budget::new(NODES, TIME);
        let call = match provider {
            Provider::Teams => teams::read(windows, &mut budget),
            Provider::Slack => slack::read(windows, self.binder.region(pid), &mut budget),
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
            let candidates = match self.found {
                Some(provider) => vec![provider],
                // A recording started by hand looks only at an app holding
                // the microphone, so an idle Slack or Teams is never walked.
                None if self.providers.len() > 1 => self
                    .providers
                    .iter()
                    .copied()
                    .filter(|provider| holds_microphone(*provider))
                    .collect(),
                None => self.providers.clone(),
            };
            let mut calls = candidates
                .into_iter()
                .map(|provider| (provider, self.read_provider(provider)))
                .collect::<Vec<_>>();
            let found = calls.iter().filter(|(_, call)| call.is_ok()).count();
            if found > 1 {
                return Sample::NoCall(NoCall::Unknown);
            }
            match calls.iter().position(|(_, call)| call.is_ok()) {
                Some(index) => {
                    let (provider, call) = calls.swap_remove(index);
                    let Ok((pid, call)) = call else {
                        unreachable!()
                    };
                    self.found = Some(provider);
                    Sample::Call {
                        binding: self.binder.bind(pid, call.region),
                        reading: call.reading,
                    }
                }
                None => {
                    self.binder.lose();
                    let unknown = calls
                        .into_iter()
                        .any(|(_, call)| call.err() == Some(NoCall::Unknown));
                    Sample::NoCall(if unknown {
                        NoCall::Unknown
                    } else {
                        NoCall::Absent
                    })
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

fn holds_microphone(provider: Provider) -> bool {
    crate::microphone_activity::application_input_devices(provider.bundle_id())
        .is_ok_and(|devices| !devices.is_empty())
}
