//! The active recording's mode, never a preference for the next recording.
use super::*;
use crate::meeting::{MeetingMode, mode::Change};

impl MeetingsPane {
    pub(in crate::rune_ui) fn poll_mode(&mut self) -> bool {
        poll(self)
    }
}

fn poll(pane: &mut MeetingsPane) -> bool {
    let Some(change) = pane.mode_change.as_ref() else {
        return false;
    };
    match change.result() {
        Some(Err(error)) => {
            pane.mode_error = Some((change.id.clone(), error));
            pane.mode_change = None;
            true
        }
        Some(Ok(()))
            if pane.meeting(&change.id).is_none_or(|meeting| {
                meeting.shared_room == (change.mode == MeetingMode::SharedRoom)
                    || meeting.status != Status::Recording
            }) =>
        {
            pane.mode_change = None;
            true
        }
        _ => false,
    }
}

fn toggle(ui: &mut Ui, id: &str) {
    if ui.meetings.mode_change.is_some() || ui.meetings.stop_pending {
        return;
    }
    let Some(meeting) = ui
        .meetings
        .meeting(id)
        .filter(|meeting| meeting.status == Status::Recording)
    else {
        return;
    };
    let mode = if meeting.shared_room {
        MeetingMode::Standard
    } else {
        MeetingMode::SharedRoom
    };
    let change = Change::new(id.into(), mode);
    ui.meetings.action_error = None;
    ui.meetings.mode_error = None;
    if ask(ui, MeetingRequest::SetMode(change.clone())) {
        ui.meetings.mode_change = Some(change);
    }
}

pub(super) fn view(ui: &Ui, p: Paint, meeting: &Meeting) -> Option<El> {
    if meeting.status != Status::Recording {
        return None;
    }
    let pending = ui
        .meetings
        .mode_change
        .as_ref()
        .is_some_and(|change| change.id == meeting.id);
    let id = meeting.id.clone();
    Some(
        super::super::controls::toggle_row(
            p,
            "meeting-shared-room",
            tr("Shared room"),
            Some(if pending {
                tr("Saving meeting mode…")
            } else {
                tr("Only this recording. Use Room mic and Call audio for people sharing this room.")
            }),
            meeting.shared_room,
            move |ui| toggle(ui, &id),
        )
        .disabled(pending || ui.meetings.stop_pending),
    )
}

pub(super) fn error<'a>(pane: &'a MeetingsPane, id: &str) -> Option<&'a str> {
    pane.mode_error
        .as_ref()
        .filter(|(meeting, _)| meeting == id)
        .map(|(_, error)| error.as_str())
}

pub(super) fn explanation(p: Paint, meeting: &Meeting) -> Option<El> {
    meeting.shared_room.then(|| text(tr("Shared room: Room mic is everyone near this computer and Call audio is the call. Hex doesn't tell the people apart."))
        .key("shared-room")
        .font(DESCRIPTION)
        .color(p.faint())
        .wrap())
}
