use std::cell::Cell;
use std::ffi::{CStr, c_char, c_void};
use std::ptr;
use std::rc::Rc;

type CfTypeRef = *const c_void;
type CfStringRef = *const c_void;
type AxUiElementRef = *const c_void;

const CF_STRING_ENCODING_UTF8: u32 = 0x0800_0100;
const MESSAGING_TIMEOUT_SECS: f32 = 0.25;

#[link(name = "ApplicationServices", kind = "framework")]
unsafe extern "C" {
    fn AXUIElementCreateApplication(pid: i32) -> AxUiElementRef;
    fn AXUIElementCopyAttributeValue(
        element: AxUiElementRef,
        attribute: CfStringRef,
        value: *mut CfTypeRef,
    ) -> i32;
    fn AXUIElementSetMessagingTimeout(element: AxUiElementRef, timeout_in_seconds: f32) -> i32;
    fn AXUIElementSetAttributeValue(
        element: AxUiElementRef,
        attribute: CfStringRef,
        value: CfTypeRef,
    ) -> i32;
    fn AXUIElementCopyParameterizedAttributeValue(
        element: AxUiElementRef,
        parameterized_attribute: CfStringRef,
        parameter: CfTypeRef,
        result: *mut CfTypeRef,
    ) -> i32;
    fn AXValueGetTypeID() -> usize;
    fn AXValueCreate(value_type: u32, value: *const c_void) -> CfTypeRef;
    fn AXValueGetValue(value: CfTypeRef, value_type: u32, value_out: *mut c_void) -> bool;
    safe fn AXIsProcessTrusted() -> bool;
}

/// `kAXErrorNoValue` and `kAXErrorAttributeUnsupported`: the element has no
/// such attribute, rather than a failed read.
const AX_ERROR_NO_VALUE: i32 = -25212;
const AX_ERROR_ATTRIBUTE_UNSUPPORTED: i32 = -25205;

/// `kAXValueTypeCFRange`.
const AX_VALUE_CF_RANGE: u32 = 4;

/// Core Foundation's `CFRange`, in UTF-16 code units.
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct CfRange {
    location: isize,
    length: isize,
}

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn CFGetTypeID(value: CfTypeRef) -> usize;
    fn CFStringGetTypeID() -> usize;
    fn CFStringGetLength(value: CfStringRef) -> isize;
    fn CFStringGetMaximumSizeForEncoding(length: isize, encoding: u32) -> isize;
    fn CFStringGetCString(
        value: CfStringRef,
        buffer: *mut c_char,
        buffer_size: isize,
        encoding: u32,
    ) -> bool;
    fn CFStringCreateWithCString(
        allocator: *const c_void,
        string: *const c_char,
        encoding: u32,
    ) -> CfStringRef;
    fn CFURLGetTypeID() -> usize;
    fn CFURLGetString(url: CfTypeRef) -> CfStringRef;
    fn CFRelease(value: CfTypeRef);
    fn CFRetain(value: CfTypeRef) -> CfTypeRef;
    fn CFEqual(left: CfTypeRef, right: CfTypeRef) -> u8;
    fn CFArrayGetTypeID() -> usize;
    fn CFArrayGetCount(array: CfTypeRef) -> isize;
    fn CFArrayGetValueAtIndex(array: CfTypeRef, index: isize) -> CfTypeRef;
    static kCFBooleanTrue: CfTypeRef;
}

pub fn focused_window_title(pid: i32) -> Option<String> {
    let application = external_application(pid)?;
    let window = copy_element(&application, c"AXFocusedWindow")?;
    copy_string(&window, c"AXTitle").filter(|title| !title.is_empty())
}

/// The page address of the web area holding the application's focused
/// element, for browsers without a scripting dictionary (Firefox, Zen).
/// The outermost web area wins, so focus inside an iframe still reports
/// the page itself.
pub fn focused_web_url(pid: i32) -> Option<String> {
    const MAX_DEPTH: usize = 64;
    let application = external_application(pid)?;
    let mut element = copy_element(&application, c"AXFocusedUIElement")?;
    let mut url = None;
    for _ in 0..MAX_DEPTH {
        if copy_string(&element, c"AXRole").as_deref() == Some("AXWebArea") {
            url = copy_url(&element, c"AXURL").or(url);
        }
        match copy_element(&element, c"AXParent") {
            Some(parent) => element = parent,
            None => break,
        }
    }
    url
}

/// Every page a browser's windows show, for a call's address or tab title
/// when its scripting dictionary can't be asked (#831). Chromium (Arc,
/// Chrome, Brave, Edge) builds its web accessibility tree only for an
/// assistive app that asks, which `AXManualAccessibility` does, once per
/// process; without it a browser behind another app shows windows but no
/// web areas.
pub fn browser_pages(pid: i32) -> crate::dictation::browser_page::BrowserPages {
    let Some(application) = external_application(pid) else {
        return Default::default();
    };
    request_web_accessibility(pid, &application);
    let mut windows: Vec<CfObject> = Vec::new();
    let candidates = [
        copy_element(&application, c"AXFocusedWindow"),
        copy_element(&application, c"AXMainWindow"),
    ]
    .into_iter()
    .flatten()
    .chain(copy_elements(&application, c"AXWindows"));
    for window in candidates {
        // SAFETY: Both are live AXUIElements.
        if !windows
            .iter()
            .any(|known| unsafe { CFEqual(known.0, window.0) } != 0)
        {
            windows.push(window);
        }
    }
    let mut pages = crate::dictation::browser_page::pages(windows.into_iter().map(Element));
    // The focused web area, when there is one, is the likeliest page.
    if let Some(url) = focused_web_url(pid) {
        pages.urls.retain(|known| known != &url);
        pages.urls.insert(0, url);
    }
    pages
}

/// Asks a Chromium browser for its web accessibility tree, once per
/// process. Other apps ignore the attribute.
fn request_web_accessibility(pid: i32, application: &CfObject) {
    static ASKED: std::sync::Mutex<Vec<i32>> = std::sync::Mutex::new(Vec::new());
    let mut asked = ASKED.lock().unwrap_or_else(|error| error.into_inner());
    if asked.contains(&pid) {
        return;
    }
    asked.push(pid);
    let Some(attribute) = cf_string_literal(c"AXManualAccessibility").map(CfObject) else {
        return;
    };
    // SAFETY: A live application element, a CFString name, and the shared
    // kCFBooleanTrue constant.
    let status =
        unsafe { AXUIElementSetAttributeValue(application.0, attribute.0, kCFBooleanTrue) };
    if status != 0 {
        tracing::debug!(
            status,
            pid,
            "the browser has no manual accessibility switch"
        );
    }
}

/// Whether Hex may read other apps. Only asks; never prompts.
pub fn is_trusted() -> bool {
    AXIsProcessTrusted()
}

/// Another app's element for a bounded reader on one background thread: it
/// never crosses threads and is never Hex's own (see `external_application`).
#[derive(Clone)]
pub struct AxElement(CfObject, Rc<Cell<bool>>);

/// All windows of process `pid` (`None` when they can't all be read), after asking its Chromium web content for an
/// accessibility tree. `enhanced` also sets `AXEnhancedUserInterface`, which
/// the new Teams needs where `AXManualAccessibility` alone does nothing.
pub fn application_windows(pid: i32, enhanced: bool) -> Option<Vec<AxElement>> {
    let application = external_application(pid)?;
    request_web_accessibility(pid, &application);
    if enhanced {
        request_enhanced_interface(pid, &application);
    }
    copy_all_elements(&application, c"AXWindows", 64, &Rc::default())
}

/// An attribute's elements, all of them or `None`: unlike `copy_elements`, a
/// failed request, a dropped element, or more than `limit` is not an empty or
/// shorter list. A missing attribute is no elements.
fn copy_all_elements(
    element: &CfObject,
    name: &CStr,
    limit: usize,
    failed: &Rc<Cell<bool>>,
) -> Option<Vec<AxElement>> {
    let attribute = CfObject(cf_string_literal(name)?);
    let mut value = ptr::null();
    // SAFETY: A live element and a CFString name; a returned value is owned
    // by the caller.
    let status = unsafe { AXUIElementCopyAttributeValue(element.0, attribute.0, &mut value) };
    if status == AX_ERROR_NO_VALUE || status == AX_ERROR_ATTRIBUTE_UNSUPPORTED {
        return Some(Vec::new());
    }
    if status != 0 || value.is_null() {
        return None;
    }
    let array = CfObject(value);
    if unsafe { CFGetTypeID(array.0) } != unsafe { CFArrayGetTypeID() } {
        return None;
    }
    let count = unsafe { CFArrayGetCount(array.0) };
    if count < 0 || count as usize > limit {
        return None;
    }
    (0..count)
        .map(|index| {
            // SAFETY: Get rule: the array owns the item, so it is retained
            // before the array goes.
            let item = unsafe { CFArrayGetValueAtIndex(array.0, index) };
            if item.is_null() {
                return None;
            }
            let item = CfObject(unsafe { CFRetain(item) });
            (unsafe { AXUIElementSetMessagingTimeout(item.0, MESSAGING_TIMEOUT_SECS) } == 0)
                .then_some(AxElement(item, failed.clone()))
        })
        .collect()
}

impl AxElement {
    /// A failure on any attribute in this poll invalidates its whole tree.
    pub fn read_failed(&self) -> bool {
        self.1.get()
    }

    fn attribute(&self, name: &CStr) -> Option<CfObject> {
        let Some(attribute) = cf_string_literal(name).map(CfObject) else {
            self.1.set(true);
            return None;
        };
        let mut value = ptr::null();
        // SAFETY: Live element and attribute; a successful value is retained.
        let status = unsafe { AXUIElementCopyAttributeValue(self.0.0, attribute.0, &mut value) };
        let result = (!value.is_null()).then(|| CfObject(value));
        match status {
            0 if result.is_some() => result,
            AX_ERROR_NO_VALUE | AX_ERROR_ATTRIBUTE_UNSUPPORTED => None,
            _ => {
                self.1.set(true);
                None
            }
        }
    }

    fn string(&self, name: &CStr) -> Option<String> {
        let value = self.attribute(name)?;
        let text = cf_string(value.0);
        if text.is_none() {
            self.1.set(true);
        }
        text
    }

    pub fn role(&self) -> Option<String> {
        self.string(c"AXRole")
    }

    pub fn classes(&self) -> Vec<String> {
        let Some(array) = self.attribute(c"AXDOMClassList") else {
            return Vec::new();
        };
        let strings = strings_from_array(&array);
        if strings.is_none() {
            self.1.set(true);
        }
        strings.unwrap_or_default()
    }

    pub fn dom_id(&self) -> Option<String> {
        self.string(c"AXDOMIdentifier")
    }

    pub fn title(&self) -> Option<String> {
        self.string(c"AXTitle")
    }

    pub fn description(&self) -> Option<String> {
        self.string(c"AXDescription")
    }

    /// Every child, or a failed poll if the list is incomplete.
    pub fn children(&self, limit: usize) -> Option<Vec<Self>> {
        let children = copy_all_elements(&self.0, c"AXChildren", limit, &self.1);
        if children.is_none() {
            self.1.set(true);
        }
        children
    }

    pub fn same(&self, other: &Self) -> bool {
        // SAFETY: Both are live AXUIElements.
        unsafe { CFEqual(self.0.0, other.0.0) != 0 }
    }
}

/// Sets `AXEnhancedUserInterface`, once per process. Chromium then builds
/// its full web tree as it does for VoiceOver; the call reports an error on
/// some builds yet takes effect.
fn request_enhanced_interface(pid: i32, application: &CfObject) {
    static ASKED: std::sync::Mutex<Vec<i32>> = std::sync::Mutex::new(Vec::new());
    let mut asked = ASKED.lock().unwrap_or_else(|error| error.into_inner());
    if asked.contains(&pid) {
        return;
    }
    asked.push(pid);
    let Some(attribute) = cf_string_literal(c"AXEnhancedUserInterface").map(CfObject) else {
        return;
    };
    // SAFETY: A live application element, a CFString name, and the shared
    // kCFBooleanTrue constant.
    let status =
        unsafe { AXUIElementSetAttributeValue(application.0, attribute.0, kCFBooleanTrue) };
    if status != 0 {
        tracing::debug!(status, pid, "the app reported an error enabling its tree");
    }
}

/// An entire bounded class list, never a partial list after a failed read.
fn strings_from_array(array: &CfObject) -> Option<Vec<String>> {
    if unsafe { CFGetTypeID(array.0) } != unsafe { CFArrayGetTypeID() } {
        return None;
    }
    let count = unsafe { CFArrayGetCount(array.0) };
    if !(0..=64).contains(&count) {
        return None;
    }
    (0..count)
        .map(|index| {
            // SAFETY: Get rule: `array` keeps each item alive while read.
            let item = unsafe { CFArrayGetValueAtIndex(array.0, index) };
            (!item.is_null()).then(|| cf_string(item)).flatten()
        })
        .collect()
}

/// An element as `browser_page` walks it.
struct Element(CfObject);

impl crate::dictation::browser_page::AxNode for Element {
    fn role(&self) -> Option<String> {
        copy_string(&self.0, c"AXRole")
    }

    fn title(&self) -> Option<String> {
        copy_string(&self.0, c"AXTitle")
    }

    fn url(&self) -> Option<String> {
        copy_url(&self.0, c"AXURL")
    }

    fn children(&self) -> Vec<Self> {
        copy_elements(&self.0, c"AXChildren")
            .into_iter()
            .map(Element)
            .collect()
    }
}

/// An attribute holding elements (`AXWindows`, `AXChildren`), each bounded.
fn copy_elements(element: &CfObject, name: &CStr) -> Vec<CfObject> {
    let Some(array) = copy_attribute(element, name) else {
        return Vec::new();
    };
    if unsafe { CFGetTypeID(array.0) } != unsafe { CFArrayGetTypeID() } {
        return Vec::new();
    }
    let count = unsafe { CFArrayGetCount(array.0) };
    (0..count)
        .filter_map(|index| {
            // SAFETY: Get rule: the array owns the item, so it is retained
            // before the array goes.
            let item = unsafe { CFArrayGetValueAtIndex(array.0, index) };
            if item.is_null() {
                return None;
            }
            let item = CfObject(unsafe { CFRetain(item) });
            (unsafe { AXUIElementSetMessagingTimeout(item.0, MESSAGING_TIMEOUT_SECS) } == 0)
                .then_some(item)
        })
        .collect()
}

/// The character just before the insertion point of the application's
/// focused text element, for Labs › Smart spacing. `None` when text is
/// selected, the cursor is at the start, or the app doesn't say.
pub fn char_before_caret(pid: i32) -> Option<char> {
    let application = external_application(pid)?;
    let focused = copy_element(&application, c"AXFocusedUIElement")?;
    let selection = copy_range(&focused, c"AXSelectedTextRange")?;
    if selection.length != 0 || selection.location <= 0 {
        return None;
    }
    // One UTF-16 unit, or two when it is half of a surrogate pair.
    (1..=2.min(selection.location)).find_map(|units| {
        let range = CfRange {
            location: selection.location - units,
            length: units,
        };
        copy_string_for_range(&focused, range)?.chars().next_back()
    })
}

fn copy_range(element: &CfObject, name: &CStr) -> Option<CfRange> {
    let value = copy_attribute(element, name)?;
    if unsafe { CFGetTypeID(value.0) } != unsafe { AXValueGetTypeID() } {
        return None;
    }
    let mut range = CfRange::default();
    // SAFETY: `value` is an AXValue and `range` has CFRange's layout.
    unsafe {
        AXValueGetValue(
            value.0,
            AX_VALUE_CF_RANGE,
            (&mut range as *mut CfRange).cast(),
        )
    }
    .then_some(range)
}

fn copy_string_for_range(element: &CfObject, range: CfRange) -> Option<String> {
    // SAFETY: AXValueCreate copies the CFRange and returns a retained object.
    let parameter = unsafe { AXValueCreate(AX_VALUE_CF_RANGE, (&range as *const CfRange).cast()) };
    if parameter.is_null() {
        return None;
    }
    let parameter = CfObject(parameter);
    let attribute = CfObject(cf_string_literal(c"AXStringForRange")?);
    let mut value = ptr::null();
    let status = unsafe {
        AXUIElementCopyParameterizedAttributeValue(element.0, attribute.0, parameter.0, &mut value)
    };
    if status != 0 || value.is_null() {
        return None;
    }
    let value = CfObject(value);
    cf_string(value.0).filter(|text| !text.is_empty())
}

struct CfObject(CfTypeRef);

impl Clone for CfObject {
    fn clone(&self) -> Self {
        // SAFETY: A live, non-null Core Foundation object; the clone owns
        // the new reference.
        Self(unsafe { CFRetain(self.0) })
    }
}

impl Drop for CfObject {
    fn drop(&mut self) {
        unsafe { CFRelease(self.0) };
    }
}

/// Self queries run Hex's AccessKit callbacks synchronously on the caller's
/// thread. AccessKit's Rc and RefCell state is confined to the main thread, so
/// a background read races it and can abort across the Objective-C callback
/// boundary. Only query other processes; application identity still comes from
/// NSWorkspace.
fn external_application(pid: i32) -> Option<CfObject> {
    if pid <= 0 || pid as u32 == std::process::id() {
        return None;
    }
    // SAFETY: AXUIElementCreateApplication returns a retained Core Foundation object.
    bounded_element(unsafe { AXUIElementCreateApplication(pid) })
}

fn bounded_element(element: AxUiElementRef) -> Option<CfObject> {
    if element.is_null() {
        return None;
    }
    let element = CfObject(element);
    (unsafe { AXUIElementSetMessagingTimeout(element.0, MESSAGING_TIMEOUT_SECS) } == 0)
        .then_some(element)
}

fn copy_attribute(element: &CfObject, name: &CStr) -> Option<CfObject> {
    let attribute = CfObject(cf_string_literal(name)?);
    let mut value = ptr::null();
    let status = unsafe { AXUIElementCopyAttributeValue(element.0, attribute.0, &mut value) };
    // `then`, not `then_some`: dropping a `CfObject(null)` would CFRelease(NULL).
    (status == 0 && !value.is_null()).then(|| CfObject(value))
}

fn copy_element(element: &CfObject, name: &CStr) -> Option<CfObject> {
    let child = copy_attribute(element, name)?;
    (unsafe { AXUIElementSetMessagingTimeout(child.0, MESSAGING_TIMEOUT_SECS) } == 0)
        .then_some(child)
}

fn copy_string(element: &CfObject, name: &CStr) -> Option<String> {
    let value = copy_attribute(element, name)?;
    cf_string(value.0)
}

fn copy_url(element: &CfObject, name: &CStr) -> Option<String> {
    let value = copy_attribute(element, name)?;
    if unsafe { CFGetTypeID(value.0) } != unsafe { CFURLGetTypeID() } {
        return cf_string(value.0);
    }
    // SAFETY: CFURLGetString follows the Get rule; `value` keeps it alive.
    let string = unsafe { CFURLGetString(value.0) };
    (!string.is_null()).then(|| cf_string(string)).flatten()
}

fn cf_string_literal(value: &CStr) -> Option<CfStringRef> {
    let string =
        unsafe { CFStringCreateWithCString(ptr::null(), value.as_ptr(), CF_STRING_ENCODING_UTF8) };
    (!string.is_null()).then_some(string)
}

fn cf_string(value: CfTypeRef) -> Option<String> {
    if unsafe { CFGetTypeID(value) } != unsafe { CFStringGetTypeID() } {
        return None;
    }
    let value = value.cast();
    let length = unsafe { CFStringGetLength(value) };
    let capacity = unsafe { CFStringGetMaximumSizeForEncoding(length, CF_STRING_ENCODING_UTF8) }
        .saturating_add(1);
    let mut bytes = vec![0_u8; capacity as usize];
    if !unsafe {
        CFStringGetCString(
            value,
            bytes.as_mut_ptr().cast(),
            capacity,
            CF_STRING_ENCODING_UTF8,
        )
    } {
        return None;
    }
    let length = bytes
        .iter()
        .position(|byte| *byte == 0)
        .unwrap_or(bytes.len());
    bytes.truncate(length);
    String::from_utf8(bytes).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn self_queries_never_create_an_accessibility_application() {
        let pid = i32::try_from(std::process::id()).unwrap();
        assert!(external_application(pid).is_none());
        assert_eq!(focused_window_title(pid), None);
        assert_eq!(focused_web_url(pid), None);
        assert_eq!(char_before_caret(pid), None);
        assert_eq!(browser_pages(pid), Default::default());
        assert!(application_windows(pid, true).is_none());
    }

    #[test]
    fn invalid_processes_are_rejected_before_native_queries() {
        for pid in [0, -1, i32::MIN] {
            assert!(external_application(pid).is_none());
            assert!(application_windows(pid, false).is_none());
        }
    }

    #[test]
    fn a_missing_attribute_is_none() {
        assert_eq!(focused_window_title(i32::MAX), None);
        assert_eq!(focused_web_url(i32::MAX), None);
        assert_eq!(char_before_caret(i32::MAX), None);
    }
}
