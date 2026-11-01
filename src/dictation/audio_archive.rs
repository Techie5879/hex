//! Local compressed copies of intentional dictations, independent of text history.
//! PCM is durably staged at capture completion; one background worker encodes it.
//! A failed encoder leaves the staged WAV for a later retry instead of losing audio.
use std::fs::{self, OpenOptions};
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{OnceLock, mpsc};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use color_eyre::eyre::{Result, bail, eyre};
use hound::{SampleFormat, WavSpec, WavWriter};

const RATE: u32 = 16_000;
const ENCODE_LIMIT: Duration = Duration::from_secs(60);
// 0 means forever, u64::MAX means stop new saves without deleting existing ones.
static MAX_AGE_MS: AtomicU64 = AtomicU64::new(0);

pub(crate) fn set_retention(retention: crate::history::HistoryRetention) {
    let age = if retention.is_off() {
        u64::MAX
    } else {
        retention.max_age_ms().unwrap_or(0)
    };
    MAX_AGE_MS.store(age, Ordering::Release);
    resume();
}

static WAKE: OnceLock<mpsc::SyncSender<()>> = OnceLock::new();

/// Called only for completed or cancelled intentional captures, before ASR.
/// Turning this off stops new saves but keeps existing recordings.
pub(crate) fn retain(samples: &[f32], kind: &str) {
    if !crate::labs::save_dictation_audio()
        || MAX_AGE_MS.load(Ordering::Acquire) == u64::MAX
        || samples.is_empty()
    {
        return;
    }
    let result = root().and_then(|root| stage(&root, samples, kind));
    match result {
        Ok(path) => {
            tracing::info!(path = %path.display(), "staged dictation audio for compression");
            wake();
        }
        Err(error) => tracing::error!(%error, "could not save dictation audio"),
    }
}

/// Coalesced notifications keep the queue bounded; the disk is the durable queue.
/// On restart, resume only this feature's unfinished encodes, never legacy history.
pub(crate) fn resume() {
    if !cfg!(test) && crate::labs::save_dictation_audio() {
        wake();
    }
}

fn root() -> Result<PathBuf> {
    Ok(crate::app_paths::support_dir()?.join("dictation-audio"))
}

fn wake() {
    let sender = WAKE.get_or_init(|| {
        let (sender, receiver) = mpsc::sync_channel(1);
        std::thread::Builder::new()
            .name("dictation-audio-archive".into())
            .spawn(move || {
                loop {
                    match receiver.recv_timeout(Duration::from_secs(60)) {
                        Err(mpsc::RecvTimeoutError::Disconnected) => break,
                        Ok(()) | Err(mpsc::RecvTimeoutError::Timeout) => {}
                    }
                    let result = root().and_then(|root| {
                        private_dir(&root)?;
                        compress_pending(&root, &encoder())?;
                        let age = MAX_AGE_MS.load(Ordering::Acquire);
                        if crate::labs::save_dictation_audio() && age != 0 && age != u64::MAX {
                            let now = SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis() as u64;
                            prune_before(&root, now.saturating_sub(age))?;
                        }
                        Ok(())
                    });
                    if let Err(error) = result {
                        tracing::error!(%error, "dictation audio compression failed; staged audio retained");
                    }
                }
            })
            .expect("start dictation audio archive worker");
        sender
    });
    let _ = sender.try_send(());
}

fn encoder() -> PathBuf {
    ["/opt/homebrew/bin/ffmpeg", "/usr/local/bin/ffmpeg"]
        .into_iter()
        .map(PathBuf::from)
        .find(|path| path.is_file())
        .unwrap_or_else(|| PathBuf::from("ffmpeg"))
}

fn private_dir(path: &Path) -> Result<()> {
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(path)?;
    Ok(())
}

fn stage(root: &Path, samples: &[f32], kind: &str) -> Result<PathBuf> {
    let pending = root.join(".pending");
    private_dir(&pending)?;
    // Milliseconds aid matching a clip to history; randomness prevents collisions.
    let stamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis();
    let stem = format!("dictation-{stamp}-{:016x}-{kind}", getrandom::u64()?);
    let partial = pending.join(format!("{stem}.partial"));
    let path = pending.join(format!("{stem}.wav"));
    let file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&partial)?;
    let mut wav = WavWriter::new(
        std::io::BufWriter::new(file),
        WavSpec {
            channels: 1,
            sample_rate: RATE,
            bits_per_sample: 16,
            sample_format: SampleFormat::Int,
        },
    )?;
    for &sample in samples {
        let sample = if sample.is_finite() {
            sample.clamp(-1.0, 1.0)
        } else {
            0.0
        };
        wav.write_sample((sample * f32::from(i16::MAX)).round() as i16)?;
    }
    wav.finalize()?;
    OpenOptions::new().write(true).open(&partial)?.sync_all()?;
    fs::rename(&partial, &path)?;
    fs::File::open(&pending)?.sync_all()?;
    Ok(path)
}

fn compress_pending(root: &Path, encoder: &Path) -> Result<()> {
    let pending = root.join(".pending");
    if !pending.is_dir() {
        return Ok(());
    }
    let mut paths = fs::read_dir(&pending)?
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| path.extension().is_some_and(|extension| extension == "wav"))
        .collect::<Vec<_>>();
    paths.sort();
    for path in paths {
        if let Err(error) = compress_one(root, &path, encoder) {
            // Continue with other recordings even if one file is damaged.
            tracing::error!(path = %path.display(), %error, "kept uncompressed dictation for retry");
        }
    }
    Ok(())
}

/// Only our timestamped recordings expire; unrelated files are never touched.
fn prune_before(root: &Path, cutoff_ms: u64) -> Result<()> {
    for directory in [root.to_path_buf(), root.join(".pending")] {
        if !directory.is_dir() {
            continue;
        }
        for entry in fs::read_dir(directory)? {
            let path = entry?.path();
            let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
                continue;
            };
            let Some(stamp) = name
                .strip_prefix("dictation-")
                .and_then(|tail| tail.split('-').next())
                .and_then(|stamp| stamp.parse::<u64>().ok())
            else {
                continue;
            };
            if stamp < cutoff_ms
                && path
                    .extension()
                    .is_some_and(|ext| ext == "ogg" || ext == "wav")
            {
                fs::remove_file(path)?;
            }
        }
    }
    Ok(())
}

fn compress_one(root: &Path, source: &Path, encoder: &Path) -> Result<PathBuf> {
    let stem = source
        .file_stem()
        .ok_or_else(|| eyre!("audio filename has no stem"))?;
    let output = root.join(stem).with_extension("ogg");
    let partial = source.with_extension("ogg.partial");
    // Pre-create privately, before ffmpeg opens it. Everything stays on this Mac.
    let _ = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(&partial)?;
    let mut child = crate::desktop::background_process::command(encoder)
        .args(["-nostdin", "-hide_banner", "-loglevel", "error", "-y", "-i"])
        .arg(source)
        .args([
            "-map",
            "0:a:0",
            "-vn",
            "-c:a",
            "libopus",
            "-b:a",
            "24k",
            "-vbr",
            "on",
            "-application",
            "voip",
            "-f",
            "ogg",
        ])
        .arg(&partial)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;
    let until = Instant::now() + ENCODE_LIMIT;
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        if Instant::now() >= until {
            let _ = child.kill();
            let _ = child.wait();
            bail!("audio encoder exceeded 60 seconds");
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    if !status.success() || fs::metadata(&partial)?.len() == 0 {
        bail!("audio encoder failed: {status}");
    }
    OpenOptions::new().write(true).open(&partial)?.sync_all()?;
    fs::rename(&partial, &output)?;
    fs::File::open(root)?.sync_all()?;
    fs::remove_file(source)?;
    tracing::info!(path = %output.display(), "saved compressed dictation audio");
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn retention_expires_only_timestamped_recordings_before_cutoff() {
        let root =
            std::env::temp_dir().join(format!("hex-audio-retention-{}", getrandom::u64().unwrap()));
        private_dir(&root.join(".pending")).unwrap();
        for name in [
            "dictation-10-abc-completed.ogg",
            "dictation-20-abc-completed.ogg",
            "notes.ogg",
            "dictation-10-not-audio.txt",
            ".pending/dictation-10-abc-cancelled.wav",
        ] {
            fs::write(root.join(name), b"test").unwrap();
        }
        prune_before(&root, 20).unwrap();
        assert!(!root.join("dictation-10-abc-completed.ogg").exists());
        assert!(
            !root
                .join(".pending/dictation-10-abc-cancelled.wav")
                .exists()
        );
        assert!(root.join("dictation-20-abc-completed.ogg").exists());
        assert!(root.join("notes.ogg").exists());
        assert!(root.join("dictation-10-not-audio.txt").exists());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn staged_audio_is_private_exact_length_and_unique() {
        let root =
            std::env::temp_dir().join(format!("hex-audio-stage-{}", getrandom::u64().unwrap()));
        let a = stage(&root, &[0.25; 16_000], "completed").unwrap();
        let b = stage(&root, &[0.25; 16_000], "completed").unwrap();
        assert_ne!(a, b);
        let reader = hound::WavReader::open(&a).unwrap();
        assert_eq!(reader.duration(), 16_000);
        assert_eq!(reader.spec().sample_rate, RATE);
        assert_eq!(
            fs::metadata(&a).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert_eq!(
            fs::metadata(root.join(".pending"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn encoder_failure_keeps_recoverable_audio() {
        let root =
            std::env::temp_dir().join(format!("hex-audio-failure-{}", getrandom::u64().unwrap()));
        let source = stage(&root, &[0.25; 16_000], "cancelled").unwrap();
        assert!(compress_one(&root, &source, Path::new("/usr/bin/false")).is_err());
        assert!(source.exists());
        assert_eq!(hound::WavReader::open(&source).unwrap().duration(), 16_000);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    #[ignore = "requires installed ffmpeg with libopus; exercises real compressed audio"]
    fn installed_encoder_round_trip() {
        let root =
            std::env::temp_dir().join(format!("hex-audio-roundtrip-{}", getrandom::u64().unwrap()));
        let samples = (0..RATE * 3)
            .map(|i| (i as f32 * 440.0 * std::f32::consts::TAU / RATE as f32).sin() * 0.4)
            .collect::<Vec<_>>();
        let source = stage(&root, &samples, "completed").unwrap();
        let output = compress_one(&root, &source, &encoder()).unwrap();
        assert!(!source.exists());
        assert!(fs::metadata(&output).unwrap().len() < 30_000);
        assert_eq!(
            fs::metadata(&output).unwrap().permissions().mode() & 0o777,
            0o600
        );
        let decoded = root.join("roundtrip.wav");
        assert!(
            std::process::Command::new(encoder())
                .args(["-nostdin", "-v", "error", "-i"])
                .arg(&output)
                .args(["-ar", "16000", "-c:a", "pcm_s16le"])
                .arg(&decoded)
                .status()
                .unwrap()
                .success()
        );
        let reader = hound::WavReader::open(decoded).unwrap();
        assert_eq!(reader.duration(), samples.len() as u32);
        let values = reader
            .into_samples::<i16>()
            .map(|sample| sample.unwrap())
            .collect::<Vec<_>>();
        assert!(values.iter().any(|sample| sample.unsigned_abs() > 8_000));
        fs::remove_dir_all(root).unwrap();
    }
}
