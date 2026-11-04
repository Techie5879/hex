//! The only platform adapter; the manager owns all storage and selection.
use super::Status;
use super::selector::Frame;

pub(super) struct Unavailable {
    pub status: Status,
    pub interrupted: bool,
}

fn unavailable(code: i32) -> Unavailable {
    Unavailable {
        status: match code {
            1 => Status::PermissionRequired,
            2..=5 => Status::Waiting,
            _ => Status::Failed,
        },
        interrupted: code != 2,
    }
}

#[cfg(any(target_os = "macos", test))]
#[repr(C)]
#[derive(Default)]
struct NativeFrame {
    status: i32,
    width: u32,
    height: u32,
    window: u32,
    source: u64,
    rgba: *mut u8,
    length: usize,
    window_identity: u64,
}

#[cfg(target_os = "macos")]
unsafe extern "C" {
    fn hex_teams_snapshot_capture(frame: *mut NativeFrame);
    fn hex_teams_snapshot_free(bytes: *mut u8);
}

#[cfg(target_os = "macos")]
pub(super) fn capture() -> Result<Frame, Unavailable> {
    let mut native = NativeFrame::default();
    // SAFETY: the Swift bridge fills the repr(C) record synchronously. Its
    // returned malloc allocation stays valid until the matching free call.
    unsafe { hex_teams_snapshot_capture(&mut native) };
    let valid = native.width > 0
        && native.height > 0
        && native.width <= 1600
        && native.height <= 1600
        && native.source != 0
        && native.window != 0
        && native.window_identity != 0
        && native.length == native.width as usize * native.height as usize * 4
        && !native.rgba.is_null();
    let pixels = if native.status == 0 && valid {
        // SAFETY: validated bounds match the bridge allocation contract.
        unsafe { std::slice::from_raw_parts(native.rgba, native.length).to_vec() }
    } else {
        Vec::new()
    };
    // SAFETY: only the bridge's allocation is released, including error paths.
    unsafe { hex_teams_snapshot_free(native.rgba) };
    match native.status {
        0 if valid => Ok(Frame {
            width: native.width,
            height: native.height,
            pixels,
            source: native.source,
            window: native.window,
            binding: native.window_identity,
        }),
        code => Err(unavailable(if code == 0 { 6 } else { code })),
    }
}

#[cfg(not(target_os = "macos"))]
pub(super) fn capture() -> Result<Frame, Unavailable> {
    Err(unavailable(2))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_frame_matches_the_c_bridge_layout() {
        assert_eq!(std::mem::size_of::<NativeFrame>(), 48);
        assert_eq!(std::mem::offset_of!(NativeFrame, source), 16);
        assert_eq!(std::mem::offset_of!(NativeFrame, rgba), 24);
        assert_eq!(std::mem::offset_of!(NativeFrame, length), 32);
        assert_eq!(std::mem::offset_of!(NativeFrame, window_identity), 40);
    }
    #[test]
    fn minimization_and_ambiguity_remain_distinct_from_no_share() {
        assert!(!unavailable(2).interrupted);
        for code in [1, 3, 4, 5, 6] {
            assert!(unavailable(code).interrupted);
        }
    }
}
