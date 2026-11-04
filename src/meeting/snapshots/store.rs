//! Durable, bounded, owner-only image storage inside one meeting directory.
use super::selector::Frame;
use super::{Snapshot, Status, View};
use color_eyre::eyre::{Result, eyre};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

pub(super) const MAX_IMAGES: usize = 400;
const MAX_BYTES: u64 = 200 * 1024 * 1024;
const MAX_INDEX: u64 = 1024 * 1024;

#[derive(Clone, Serialize, Deserialize)]
struct Entry {
    id: u64,
    at_ms: u64,
    until_ms: u64,
    asset: String,
    width: u32,
    height: u32,
}
#[derive(Clone, Default, Serialize, Deserialize)]
struct Index {
    version: u8,
    next: u64,
    entries: Vec<Entry>,
    /// Deletion must not be undone by an in-flight sample of the same image.
    #[serde(default)]
    deleted_assets: Vec<String>,
    #[serde(default)]
    has_gaps: bool,
}

pub(super) struct Store {
    directory: PathBuf,
    index: Index,
}
impl Store {
    pub fn unavailable(meeting: &Path) -> Self {
        Self {
            directory: meeting.join("snapshots"),
            index: Index {
                version: 1,
                next: 1,
                ..Index::default()
            },
        }
    }
    pub fn new(meeting: &Path) -> Result<Self> {
        let directory = meeting.join("snapshots");
        if directory.exists() && fs::symlink_metadata(&directory)?.file_type().is_symlink() {
            return Err(eyre!("symlinked snapshot directory"));
        }
        fs::create_dir_all(&directory)?;
        super::super::set_owner_only(&directory, true)?;
        let store = Self::unavailable(meeting);
        store.persist(&store.index)?;
        Ok(store)
    }
    pub fn view(&self, status: Status) -> View {
        project(&self.directory, &self.index, status)
    }
    pub fn mark_gap(&mut self) {
        if !self.index.has_gaps {
            self.index.has_gaps = true;
            if let Err(error) = self.persist(&self.index) {
                tracing::warn!(%error, "could not retain snapshot gap state");
            }
        }
    }
    fn persist(&self, index: &Index) -> Result<()> {
        let bytes = serde_json::to_vec(index)?;
        if bytes.len() as u64 > MAX_INDEX {
            return Err(eyre!("snapshot index limit"));
        }
        let path = self.directory.join("index.json");
        if path.exists() && fs::symlink_metadata(&path)?.file_type().is_symlink() {
            return Err(eyre!("symlinked snapshot index"));
        }
        crate::durable::write(&path, &bytes)?;
        super::super::set_owner_only(&path, false)?;
        Ok(())
    }
    pub fn add(&mut self, frame: &Frame, at_ms: u64, until_ms: u64) -> Result<bool> {
        self.add_with_writer(frame, at_ms, until_ms, write_asset)
    }

    fn add_with_writer(
        &mut self,
        frame: &Frame,
        at_ms: u64,
        until_ms: u64,
        mut write: impl FnMut(&Path, &[u8]) -> Result<()>,
    ) -> Result<bool> {
        if !frame.valid() {
            return Err(eyre!("invalid snapshot dimensions"));
        }
        if self.index.entries.len() >= MAX_IMAGES {
            return Ok(false);
        }
        let mut hash = Sha256::new();
        hash.update(frame.width.to_le_bytes());
        hash.update(frame.height.to_le_bytes());
        hash.update(&frame.pixels);
        let asset: String = hash
            .finalize()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        if self.index.deleted_assets.contains(&asset) {
            return Ok(true);
        }
        if self.index.deleted_assets.len() >= MAX_IMAGES {
            return Ok(false);
        }
        let path = self.directory.join(format!("{asset}.png"));
        let thumbnail = self.directory.join(format!("{asset}-thumb.png"));
        let (w, h, pixels) = thumbnail_pixels(frame);
        if !complete_png(&path, frame.width, frame.height) || !complete_png(&thumbnail, w, h) {
            let encoded = encode(frame.width, frame.height, &frame.pixels)?;
            let thumb = encode(w, h, &pixels)?;
            let total = fs::read_dir(&self.directory)?.try_fold(
                0u64,
                |total, entry| -> std::io::Result<u64> {
                    let entry = entry?;
                    // Existing incomplete versions are replaced, not added.
                    let bytes = if entry.path() == path || entry.path() == thumbnail {
                        0
                    } else {
                        entry.metadata()?.len()
                    };
                    Ok(total.saturating_add(bytes))
                },
            )?;
            if total
                .saturating_add(encoded.len() as u64)
                .saturating_add(thumb.len() as u64)
                > MAX_BYTES
            {
                return Ok(false);
            }
            write(&path, &encoded)?;
            write(&thumbnail, &thumb)?;
        }
        let mut next = self.index.clone();
        next.entries.push(Entry {
            id: next.next,
            at_ms,
            until_ms,
            asset,
            width: frame.width,
            height: frame.height,
        });
        next.next += 1;
        self.persist(&next)?;
        self.index = next;
        Ok(true)
    }
    pub fn delete(&mut self, id: u64) -> Result<()> {
        let Some(entry) = self
            .index
            .entries
            .iter()
            .find(|entry| entry.id == id)
            .cloned()
        else {
            return Err(eyre!("snapshot not found"));
        };
        let mut next = self.index.clone();
        next.entries.retain(|entry| entry.id != id);
        if !next.deleted_assets.contains(&entry.asset) {
            next.deleted_assets.push(entry.asset.clone());
        }
        self.persist(&next)?;
        self.index = next;
        if !self
            .index
            .entries
            .iter()
            .any(|other| other.asset == entry.asset)
        {
            for name in [
                format!("{}.png", entry.asset),
                format!("{}-thumb.png", entry.asset),
            ] {
                match fs::remove_file(self.directory.join(name)) {
                    Ok(()) => {}
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                    Err(e) => return Err(e.into()),
                }
            }
            crate::durable::sync_directory(&self.directory)?;
        }
        Ok(())
    }
    pub fn load(meeting: &Path) -> Result<Self> {
        let directory = meeting.join("snapshots");
        if fs::symlink_metadata(&directory)?.file_type().is_symlink() {
            return Err(eyre!("symlinked snapshots"));
        }
        let path = directory.join("index.json");
        let meta = fs::symlink_metadata(&path)?;
        if !meta.is_file() || meta.len() > MAX_INDEX {
            return Err(eyre!("invalid snapshot index"));
        }
        let index: Index = serde_json::from_slice(&fs::read(path)?)?;
        let mut ids = HashSet::new();
        if index.version != 1
            || index.entries.len() > MAX_IMAGES
            || index.deleted_assets.len() > MAX_IMAGES
            || index
                .deleted_assets
                .iter()
                .any(|asset| asset.len() != 64 || !asset.bytes().all(|c| c.is_ascii_hexdigit()))
            || index.entries.iter().any(|e| {
                !ids.insert(e.id)
                    || e.id >= index.next
                    || e.at_ms > e.until_ms
                    || e.width == 0
                    || e.height == 0
                    || e.width > 1600
                    || e.height > 1600
                    || e.asset.len() != 64
                    || !e.asset.bytes().all(|c| c.is_ascii_hexdigit())
            })
        {
            return Err(eyre!("invalid snapshot entries"));
        }
        Ok(Self { directory, index })
    }
}
fn write_asset(path: &Path, bytes: &[u8]) -> Result<()> {
    if let Ok(metadata) = fs::symlink_metadata(path)
        && !metadata.is_file()
    {
        return Err(eyre!("invalid snapshot file"));
    }
    crate::durable::write(path, bytes)?;
    super::super::set_owner_only(path, false)?;
    Ok(())
}
fn complete_png(path: &Path, width: u32, height: u32) -> bool {
    let Ok(metadata) = fs::symlink_metadata(path) else {
        return false;
    };
    let expected = width as usize * height as usize * 4;
    if !metadata.is_file() || metadata.len() > expected as u64 + 1024 * 1024 {
        return false;
    }
    let Ok(file) = fs::File::open(path) else {
        return false;
    };
    let decoder = png::Decoder::new(std::io::BufReader::new(file));
    let Ok(mut reader) = decoder.read_info() else {
        return false;
    };
    if reader.output_buffer_size() != Some(expected) {
        return false;
    }
    let mut bytes = vec![0; expected];
    reader.next_frame(&mut bytes).is_ok_and(|info| {
        info.width == width
            && info.height == height
            && info.color_type == png::ColorType::Rgba
            && info.bit_depth == png::BitDepth::Eight
    })
}
fn regular(path: &Path) -> bool {
    fs::symlink_metadata(path).is_ok_and(|m| m.is_file())
}
fn project(directory: &Path, index: &Index, status: Status) -> View {
    View {
        status,
        has_gaps: index.has_gaps,
        images: index
            .entries
            .iter()
            .filter_map(|e| {
                let path = directory.join(format!("{}.png", e.asset));
                let thumbnail = directory.join(format!("{}-thumb.png", e.asset));
                (regular(&path) && regular(&thumbnail)).then_some(Snapshot {
                    id: e.id,
                    at_ms: e.at_ms,
                    until_ms: e.until_ms,
                    path,
                    thumbnail,
                    width: e.width,
                    height: e.height,
                })
            })
            .collect(),
    }
}
fn encode(width: u32, height: u32, pixels: &[u8]) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut bytes, width, height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header()?;
        writer.write_image_data(pixels)?;
    }
    Ok(bytes)
}
fn thumbnail_pixels(frame: &Frame) -> (u32, u32, Vec<u8>) {
    let scale = (320.0 / f64::from(frame.width))
        .min(200.0 / f64::from(frame.height))
        .min(1.0);
    let w = (f64::from(frame.width) * scale).round().max(1.0) as u32;
    let h = (f64::from(frame.height) * scale).round().max(1.0) as u32;
    let mut out = vec![0; (w * h * 4) as usize];
    for y in 0..h {
        for x in 0..w {
            let from = (((y * frame.height / h) * frame.width + x * frame.width / w) * 4) as usize;
            let to = ((y * w + x) * 4) as usize;
            out[to..to + 4].copy_from_slice(&frame.pixels[from..from + 4]);
        }
    }
    (w, h, out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};
    fn directory() -> PathBuf {
        static N: AtomicU64 = AtomicU64::new(0);
        let p = std::env::temp_dir().join(format!(
            "hex-snapshots-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&p).unwrap();
        p
    }
    fn frame() -> Frame {
        Frame {
            width: 4,
            height: 4,
            pixels: vec![100; 64],
            source: 1,
            window: 1,
            binding: 1,
        }
    }
    #[test]
    fn exact_revisits_share_files_but_keep_separate_timestamps() {
        let p = directory();
        let mut store = Store::new(&p).unwrap();
        assert!(store.add(&frame(), 1, 2).unwrap());
        assert!(store.add(&frame(), 10, 11).unwrap());
        let restored = Store::load(&p).unwrap().view(Status::Complete);
        assert_eq!(restored.images.len(), 2);
        assert_eq!(restored.images[0].path, restored.images[1].path);
        assert_eq!(restored.images[1].at_ms, 10);
        assert_eq!(fs::read_dir(p.join("snapshots")).unwrap().count(), 3);
        store.delete(1).unwrap();
        assert!(restored.images[0].path.exists());
        store.delete(2).unwrap();
        assert!(!restored.images[0].path.exists());
        assert!(store.add(&frame(), 20, 21).unwrap());
        assert!(
            store.index.entries.is_empty(),
            "a deleted visible image must not immediately return"
        );
        assert!(Store::load(&p).unwrap().index.entries.is_empty());
        fs::remove_dir_all(p).unwrap();
    }
    #[test]
    fn malformed_or_escaping_indexes_are_refused() {
        let p = directory();
        let mut store = Store::new(&p).unwrap();
        store.add(&frame(), 1, 2).unwrap();
        store.index.entries[0].asset = "../../elsewhere".into();
        store.persist(&store.index).unwrap();
        assert!(Store::load(&p).is_err());
        fs::remove_dir_all(p).unwrap();
    }
    #[test]
    fn capture_gaps_survive_a_restart() {
        let p = directory();
        let mut store = Store::new(&p).unwrap();
        store.mark_gap();
        assert!(Store::load(&p).unwrap().view(Status::Complete).has_gaps);
        fs::remove_dir_all(p).unwrap();
    }
    #[test]
    fn a_failed_image_or_thumbnail_write_can_be_retried_and_deleted() {
        for failure_at in [0, 1] {
            for partial in [false, true] {
                let p = directory();
                let mut store = Store::new(&p).unwrap();
                let mut call = 0;
                let result = store.add_with_writer(&frame(), 1, 2, |path, bytes| {
                    let fail = call == failure_at;
                    call += 1;
                    if fail {
                        if partial {
                            fs::write(path, &bytes[..bytes.len() / 2])?;
                        } else {
                            write_asset(path, bytes)?;
                        }
                        return Err(eyre!("injected write or sync failure"));
                    }
                    write_asset(path, bytes)
                });
                assert!(result.is_err());
                assert!(
                    Store::load(&p)
                        .unwrap()
                        .view(Status::Complete)
                        .images
                        .is_empty()
                );
                assert!(store.add(&frame(), 1, 2).unwrap());
                let restored = Store::load(&p).unwrap().view(Status::Complete);
                assert_eq!(restored.images.len(), 1);
                assert!(complete_png(&restored.images[0].path, 4, 4));
                store.delete(restored.images[0].id).unwrap();
                assert!(
                    Store::load(&p)
                        .unwrap()
                        .view(Status::Complete)
                        .images
                        .is_empty()
                );
                fs::remove_dir_all(p).unwrap();
            }
        }
    }
    #[test]
    fn image_limit_does_not_evict_previous_content() {
        let p = directory();
        let mut store = Store::new(&p).unwrap();
        store.add(&frame(), 1, 2).unwrap();
        let mut entries = Vec::new();
        for id in 1..=MAX_IMAGES as u64 {
            let mut e = store.index.entries[0].clone();
            e.id = id;
            entries.push(e);
        }
        store.index.entries = entries;
        store.index.next = MAX_IMAGES as u64 + 1;
        assert!(!store.add(&frame(), 20, 21).unwrap());
        assert_eq!(store.index.entries.len(), MAX_IMAGES);
        fs::remove_dir_all(p).unwrap();
    }
    #[cfg(unix)]
    #[test]
    fn stored_content_is_private_and_symlinked_images_are_not_projected() {
        use std::os::unix::fs::{PermissionsExt, symlink};
        let p = directory();
        let mut store = Store::new(&p).unwrap();
        store.add(&frame(), 1, 2).unwrap();
        let view = store.view(Status::Complete);
        let path = &view.images[0].path;
        assert_eq!(
            fs::metadata(path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert_eq!(
            fs::metadata(p.join("snapshots"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
        fs::remove_file(path).unwrap();
        symlink("/etc/passwd", path).unwrap();
        assert!(store.view(Status::Complete).images.is_empty());
        fs::remove_dir_all(p).unwrap();
    }
}
