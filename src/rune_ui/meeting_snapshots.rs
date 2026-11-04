//! Local meeting images, with only the visible page decoded off the UI thread.
use std::cell::RefCell;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, TryRecvError};

use rune::{El, col, row, text};

use super::controls::{ButtonKind, DESCRIPTION, Paint, button};
use super::lists::{error_panel, section_label};
use super::{Ui, meetings::Meeting};
use crate::desktop::i18n::{tr, tr_fill};
use crate::meeting::snapshots::Status;

const PAGE_SIZE: usize = 6;

#[derive(Default)]
pub(super) struct Images {
    pub(super) cache: Option<rune::ImageCache>,
    pub(super) waker: Option<rune::Waker>,
    thumbnails: RefCell<HashMap<PathBuf, rune::Image>>,
    page: usize,
    expanded: bool,
    meeting: String,
    ready: usize,
    fixture: bool,
    pending: Option<Receiver<Result<(), String>>>,
    error: Option<String>,
}

impl Images {
    pub(super) fn poll(&mut self, selected: Option<&str>) -> bool {
        let mut changed = false;
        if selected.unwrap_or_default() != self.meeting {
            self.meeting = selected.unwrap_or_default().into();
            self.page = 0;
            self.expanded = false;
            self.thumbnails.get_mut().clear();
            self.error = None;
            changed = true;
        }
        let ready = self
            .thumbnails
            .get_mut()
            .values()
            .filter(|image| image.is_ready())
            .count();
        if self.ready != ready {
            self.ready = ready;
            changed = true;
        }
        let finished = self
            .pending
            .as_ref()
            .and_then(|pending| match pending.try_recv() {
                Ok(result) => Some(result),
                Err(TryRecvError::Empty) => None,
                Err(TryRecvError::Disconnected) => Some(Err(tr("Try again").into())),
            });
        if let Some(result) = finished {
            self.pending = None;
            self.error = result.err();
            changed = true;
        }
        changed
    }
}

pub(super) fn view(ui: &Ui, p: Paint, meeting: &Meeting) -> Option<El> {
    let snapshots = &meeting.snapshots;
    let supported = ui.platform.caps.os == super::platform::Os::Macos;
    if snapshots.images.is_empty() && !snapshots.active {
        return None;
    }
    let state = &ui.meetings.snapshots;
    let mut header = row()
        .key("snapshot-header")
        .gap(8.0)
        .center_y()
        .child(section_label(p, "snapshot-label", tr("Meeting snapshots")));
    let controllable = supported
        && ui.settings.labs.meeting_snapshots
        && meeting.status.is_active()
        && matches!(
            snapshots.status,
            Status::Waiting | Status::Capturing | Status::Paused
        );
    if controllable {
        let paused = snapshots.status == Status::Paused;
        let id = meeting.id.clone();
        header = header.child(
            button(
                p,
                "snapshots-pause",
                if paused {
                    tr("Resume snapshots")
                } else {
                    tr("Pause snapshots")
                },
                ButtonKind::Secondary,
                move |ui: &mut Ui, _| {
                    action(ui, id.clone(), SnapshotAction::Pause(!paused));
                },
            )
            .disabled(state.pending.is_some()),
        );
    }
    if supported
        && ui.settings.labs.meeting_snapshots
        && snapshots.status == Status::PermissionRequired
        && meeting.status.is_active()
    {
        header = header.child(button(
            p,
            "snapshots-permission",
            tr("Allow Screen Recording"),
            ButtonKind::Secondary,
            |ui: &mut Ui, _| {
                #[cfg(target_os = "macos")]
                if ui.live() {
                    crate::meeting::request_screen_recording();
                }
                #[cfg(not(target_os = "macos"))]
                let _ = ui;
            },
        ));
    }
    let mut body = col().key("meeting-snapshots").gap(10.0)
        .child(header)
        .child(text(tr(snapshots.status.label())).key("snapshots-status").font(DESCRIPTION).color(p.faint()).wrap())
        .child(text(tr("Images stay on this computer and follow this meeting's retention. They are not sent to OpenCode.")).key("snapshots-local").font(DESCRIPTION).color(p.faint()).wrap());
    if snapshots.has_gaps {
        body = body.child(
            text(tr(
                "Some content may be missing because snapshot capture was paused or interrupted.",
            ))
            .key("snapshots-gaps")
            .font(DESCRIPTION)
            .color(p.faint())
            .wrap(),
        );
    }
    if let Some(error) = &state.error {
        body = body.child(error_panel(
            p,
            "snapshots-error",
            tr("Snapshot could not be updated."),
            error,
        ));
    }
    if snapshots.images.is_empty() {
        state.thumbnails.borrow_mut().clear();
        return Some(body);
    }
    body = body.child(button(
        p,
        "snapshots-toggle-images",
        if state.expanded {
            tr("Hide images")
        } else {
            tr("Show images")
        },
        ButtonKind::Secondary,
        |ui: &mut Ui, _| {
            ui.meetings.snapshots.expanded = !ui.meetings.snapshots.expanded;
            fixture_page(ui);
        },
    ));
    if !state.expanded {
        state.thumbnails.borrow_mut().clear();
        return Some(body);
    }
    let page_count = snapshots.images.len().div_ceil(PAGE_SIZE).max(1);
    let page = state.page.min(page_count - 1);
    let visible = snapshots
        .images
        .iter()
        .skip(page * PAGE_SIZE)
        .take(PAGE_SIZE);
    state.thumbnails.borrow_mut().retain(|path, _| {
        snapshots
            .images
            .iter()
            .skip(page * PAGE_SIZE)
            .take(PAGE_SIZE)
            .any(|image| &image.thumbnail == path)
    });
    for snapshot in visible {
        let key = format!("snapshot-{}", snapshot.id);
        let mut preview = col().key(format!("{key}-image")).size(160.0, 90.0).center();
        if let Some(cache) = &state.cache {
            let mut thumbnails = state.thumbnails.borrow_mut();
            let image = thumbnails
                .entry(snapshot.thumbnail.clone())
                .or_insert_with(|| cache.file(&snapshot.thumbnail, 160.0, 2.0));
            if image.is_ready() && image.error().is_none() {
                let (width, height) = preview_size(snapshot.width, snapshot.height);
                preview = preview.child(
                    col()
                        .key(format!("{key}-pixels"))
                        .size(width, height)
                        .pixels(image.pixels()),
                );
            } else if image.error().is_some() {
                preview = preview.child(
                    text(tr("Preview unavailable"))
                        .font(DESCRIPTION)
                        .color(p.faint())
                        .wrap(),
                );
            }
        }
        let timestamp = super::meetings::clock_offset(snapshot.at_ms);
        let image_id = snapshot.id;
        let meeting_id = meeting.id.clone();
        let path = snapshot.path.clone();
        body = body.child(
            row()
                .key(key.clone())
                .gap(12.0)
                .center_y()
                .child(preview)
                .child(
                    col()
                        .gap(8.0)
                        .child(text(timestamp.clone()).font(DESCRIPTION).color(p.faint()))
                        .child(button(
                            p,
                            &format!("{key}-open"),
                            &tr_fill("Show image at {}", &timestamp),
                            ButtonKind::Secondary,
                            move |ui: &mut Ui, _| {
                                if ui.live() {
                                    crate::meeting::reveal(&path);
                                }
                            },
                        ))
                        .child(
                            button(
                                p,
                                &format!("{key}-delete"),
                                &tr_fill("Delete image at {}", &timestamp),
                                ButtonKind::Secondary,
                                move |ui: &mut Ui, _| {
                                    action(
                                        ui,
                                        meeting_id.clone(),
                                        SnapshotAction::Delete(image_id),
                                    );
                                },
                            )
                            .disabled(state.pending.is_some()),
                        ),
                ),
        );
    }
    if page_count > 1 {
        body = body.child(
            row()
                .key("snapshot-pages")
                .gap(8.0)
                .center_y()
                .child(
                    button(
                        p,
                        "snapshots-previous",
                        tr("Previous images"),
                        ButtonKind::Secondary,
                        move |ui: &mut Ui, _| {
                            ui.meetings.snapshots.page = page.saturating_sub(1);
                            fixture_page(ui);
                        },
                    )
                    .disabled(page == 0),
                )
                .child(
                    text(format!("{} / {}", page + 1, page_count))
                        .font(DESCRIPTION)
                        .color(p.faint()),
                )
                .child(
                    button(
                        p,
                        "snapshots-next",
                        tr("Next images"),
                        ButtonKind::Secondary,
                        move |ui: &mut Ui, _| {
                            ui.meetings.snapshots.page = (page + 1).min(page_count - 1);
                            fixture_page(ui);
                        },
                    )
                    .disabled(page + 1 == page_count),
                ),
        );
    }
    Some(body)
}

enum SnapshotAction {
    Delete(u64),
    Pause(bool),
}

fn action(ui: &mut Ui, id: String, action: SnapshotAction) {
    if ui.meetings.snapshots.pending.is_some() {
        return;
    }
    if !ui.live() {
        if let Some(meeting) = ui
            .meetings
            .meetings
            .iter_mut()
            .find(|meeting| meeting.id == id)
        {
            match action {
                SnapshotAction::Delete(image) => meeting
                    .snapshots
                    .images
                    .retain(|snapshot| snapshot.id != image),
                SnapshotAction::Pause(paused) => {
                    meeting.snapshots.status = if paused {
                        Status::Paused
                    } else {
                        Status::Waiting
                    }
                }
            }
        }
        fixture_page(ui);
        return;
    }
    let (sender, receiver) = mpsc::channel();
    let wake = ui.meetings.snapshots.waker.clone();
    ui.meetings.snapshots.pending = Some(receiver);
    ui.meetings.snapshots.error = None;
    std::thread::spawn(move || {
        let result = match action {
            SnapshotAction::Delete(image) => crate::meeting::snapshots::delete(&id, image),
            SnapshotAction::Pause(paused) => crate::meeting::snapshots::set_paused(&id, paused),
        };
        let _ = sender.send(result);
        if let Some(wake) = wake {
            wake();
        }
    });
}

/// Artificial diagram pixels, never a real meeting or private screen.
pub(super) fn fixture(ui: &mut Ui) {
    let cache = rune::ImageCache::new(std::sync::Arc::new(|| {}));
    let meeting = ui
        .meetings
        .meetings
        .iter_mut()
        .find(|meeting| meeting.id == "preview-live")
        .unwrap();
    meeting.snapshots.active = true;
    meeting.snapshots.status = Status::Capturing;
    meeting.snapshots.has_gaps = true;
    meeting.title = "Teams design review".into();
    meeting.app = Some("Microsoft Teams".into());
    for id in 0..3 {
        let path = PathBuf::from(format!("fixture-snapshot-{id}.png"));
        let (width, height) = fixture_dimensions(id);
        meeting
            .snapshots
            .images
            .push(crate::meeting::snapshots::Snapshot {
                id: id as u64,
                at_ms: id as u64 * 30_000,
                until_ms: (id as u64 + 1) * 30_000,
                path: path.clone(),
                thumbnail: path,
                width,
                height,
            });
    }
    ui.meetings.snapshots.meeting = meeting.id.clone();
    ui.meetings.snapshots.cache = Some(cache);
    ui.meetings.snapshots.fixture = true;
    ui.meetings.snapshots.expanded = true;
    ui.meetings.select("preview-live".into());
    ui.settings.labs.meeting_snapshots = true;
    fixture_page(ui);
}

/// Seed the synthetic scene's current page before it draws. The drawing path
/// always uses the regular image cache and never generates fixture pixels.
pub(super) fn fixture_page(ui: &mut Ui) {
    if !ui.meetings.snapshots.fixture || !ui.meetings.snapshots.expanded {
        return;
    }
    let state = &ui.meetings.snapshots;
    let Some(cache) = &state.cache else { return };
    let Some(meeting) = ui
        .meetings
        .meetings
        .iter()
        .find(|meeting| Some(&meeting.id) == ui.meetings.selected.as_ref())
    else {
        return;
    };
    let page_count = meeting.snapshots.images.len().div_ceil(PAGE_SIZE).max(1);
    let page = state.page.min(page_count - 1);
    let mut thumbnails = state.thumbnails.borrow_mut();
    thumbnails.clear();
    for snapshot in meeting
        .snapshots
        .images
        .iter()
        .skip(page * PAGE_SIZE)
        .take(PAGE_SIZE)
    {
        let image = cache.rgba(
            snapshot.width,
            snapshot.height,
            fixture_pixels(snapshot.id as u32),
        );
        let _ = image.wait();
        thumbnails.insert(snapshot.thumbnail.clone(), image);
    }
}

fn preview_size(width: u32, height: u32) -> (f32, f32) {
    let width = width.max(1) as f32;
    let height = height.max(1) as f32;
    let scale = (160.0 / width).min(90.0 / height);
    (width * scale, height * scale)
}

fn fixture_dimensions(id: u32) -> (u32, u32) {
    match id {
        1 => (90, 160),
        2 => (128, 128),
        _ => (160, 90),
    }
}

fn fixture_pixels(id: u32) -> Vec<u8> {
    let (width, height) = fixture_dimensions(id);
    let mut pixels = Vec::with_capacity((width * height * 4) as usize);
    for y in 0..height {
        for x in 0..width {
            let rgb = if x > 15 && x < 145 && y > 15 && y < 26 {
                [64, 128, 208]
            } else if y > 40 && y < 70 && x > 15 && x < 35 + id * 7 {
                [80, 170, 130]
            } else {
                [235, 238, 242]
            };
            pixels.extend_from_slice(&[rgb[0], rgb[1], rgb[2], 255]);
        }
    }
    pixels
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn image_list_starts_collapsed() {
        assert!(!Images::default().expanded);
        assert_eq!(PAGE_SIZE, 6);
    }

    #[test]
    fn landscape_and_portrait_previews_preserve_aspect_ratio() {
        assert_eq!(preview_size(1920, 1080), (160.0, 90.0));
        assert_eq!(preview_size(1080, 1920), (50.625, 90.0));
        assert_eq!(preview_size(100, 100), (90.0, 90.0));
    }

    #[test]
    fn disconnected_actions_clear_pending_and_show_retry() {
        let (sender, receiver) = mpsc::channel();
        drop(sender);
        let mut images = Images {
            pending: Some(receiver),
            ..Default::default()
        };
        assert!(images.poll(None));
        assert!(images.pending.is_none());
        assert_eq!(images.error.as_deref(), Some("Try again"));
        assert!(!images.poll(None), "no idle redraw loop");
    }
}
