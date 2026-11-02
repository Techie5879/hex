//! The profile store owned by the running desktop app. It never serves control,
//! audio, or credentials. A persisted pending profile survives interrupted apply.
use std::fs::{self, OpenOptions};
use std::path::{Path, PathBuf};

use color_eyre::eyre::{Result, eyre};
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

use super::{adapter, decode, empty, encode};
use crate::app_settings::AppSettings;

#[derive(Clone, Debug)]
pub enum Exchange {
    Get,
    Put {
        text: String,
        if_match: Option<String>,
        if_none_match: Option<String>,
    },
}

#[derive(Clone, Debug)]
pub struct Reply {
    pub status: u16,
    pub body: String,
    pub etag: Option<String>,
}

impl Reply {
    pub fn code(status: u16) -> Self {
        Self {
            status,
            body: String::new(),
            etag: None,
        }
    }
}

#[derive(Deserialize, Serialize)]
struct State {
    device: String,
    profile: Value,
    applied: Value,
    #[serde(default)]
    import_pending: bool,
}

pub fn directory() -> Result<PathBuf> {
    Ok(crate::app_paths::support_dir()?.join("profile-sync/host"))
}

pub fn private_directory(path: &Path) -> Result<()> {
    fs::create_dir_all(path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

pub fn write_private(path: &Path, bytes: &[u8]) -> Result<()> {
    crate::durable::write(path, bytes)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}

fn identifier() -> Result<String> {
    let mut bytes = [0u8; 16];
    getrandom::fill(&mut bytes).map_err(|e| eyre!("{e}"))?;
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}

pub fn hash(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

pub fn etag(profile: &Value) -> String {
    format!("\"{}\"", hash(encode(profile).as_bytes()))
}

/// The file lock also fences the CLI's token changes and migration against the app.
pub fn locked<T>(path: &Path, action: impl FnOnce() -> Result<T>) -> Result<T> {
    private_directory(path)?;
    let lock = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(path.join("lock"))?;
    lock.try_lock_exclusive()
        .map_err(|_| eyre!("Profile sync is busy. Try again."))?;
    let result = action();
    FileExt::unlock(&lock)?;
    result
}

fn load(path: &Path) -> Result<State> {
    match fs::read(path.join("state.json")) {
        Ok(bytes) => {
            let state: State = serde_json::from_slice(&bytes)?;
            decode(&encode(&state.profile)).map_err(|e| eyre!(e))?;
            decode(&encode(&state.applied)).map_err(|e| eyre!(e))?;
            Ok(state)
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(State {
            device: identifier()?,
            profile: empty(),
            applied: empty(),
            import_pending: false,
        }),
        Err(e) => Err(e.into()),
    }
}

fn save(path: &Path, state: &State) -> Result<()> {
    decode(&encode(&state.profile)).map_err(|e| eyre!(e))?;
    write_private(&path.join("state.json"), &serde_json::to_vec(state)?)
}

fn apply(
    state: &mut State,
    settings: &mut AppSettings,
    persist: &mut impl FnMut(&AppSettings) -> Result<()>,
    path: &Path,
) -> Result<()> {
    let mut updated = settings.clone();
    updated.vocabulary = adapter::vocabulary(&state.profile);
    updated.dictation_processing.default_mode.post_processing = adapter::apply(
        &state.profile,
        &settings.dictation_processing.default_mode.post_processing,
    );
    if updated != *settings {
        persist(&updated)?;
        *settings = updated;
    }
    state.import_pending = false;
    state.applied = state.profile.clone();
    save(path, state)
}

/// Called on the app's settings owner, so no network worker edits live settings.
pub fn exchange(
    path: &Path,
    request: Exchange,
    settings: &mut AppSettings,
    mut persist: impl FnMut(&AppSettings) -> Result<()>,
) -> Result<Reply> {
    if !settings.labs.profile_sync {
        return Ok(Reply::code(503));
    }
    locked(path, || {
        let mut state = load(path)?;
        if !state.import_pending {
            adapter::absorb(
                &mut state.profile,
                &state.applied,
                &settings.vocabulary,
                &settings.dictation_processing.default_mode.post_processing,
                &state.device,
                crate::desktop::events::now_ms(),
            );
        }
        // Save the merged intention before updating settings. A failed apply retries
        // from `applied`, rather than stamping stale settings over the remote edits.
        save(path, &state)?;
        apply(&mut state, settings, &mut persist, path)?;
        if let Exchange::Put {
            text,
            if_match,
            if_none_match,
        } = request
        {
            if if_match.is_none() && if_none_match.is_none() {
                return Ok(Reply::code(428));
            }
            if if_none_match.is_some() || if_match.as_deref() != Some(etag(&state.profile).as_str())
            {
                return Ok(Reply::code(412));
            }
            let incoming = match decode(&text) {
                Ok(p) => p,
                Err(_) => return Ok(Reply::code(422)),
            };
            state.profile = incoming;
            save(path, &state)?;
            apply(&mut state, settings, &mut persist, path)?;
            return Ok(Reply {
                status: 204,
                body: String::new(),
                etag: Some(etag(&state.profile)),
            });
        }
        Ok(Reply {
            status: 200,
            body: encode(&state.profile),
            etag: Some(etag(&state.profile)),
        })
    })
}

#[derive(Serialize, Deserialize)]
pub struct Device {
    pub name: String,
    pub hash: String,
}

pub fn devices(path: &Path) -> Result<Vec<Device>> {
    match fs::read(path.join("devices.json")) {
        Ok(bytes) => Ok(serde_json::from_slice(&bytes)?),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(e) => Err(e.into()),
    }
}

pub fn authorized(path: &Path, token: &str) -> bool {
    if token.is_empty() || token.len() > 1024 {
        return false;
    }
    let hash = hash(token.as_bytes());
    devices(path).is_ok_and(|devices| {
        devices.iter().fold(false, |found, device| {
            // Both digests have a fixed length; never compare the secret token itself.
            let matches = device.hash.len() == hash.len()
                && device
                    .hash
                    .bytes()
                    .zip(hash.bytes())
                    .fold(0u8, |d, (a, b)| d | (a ^ b))
                    == 0;
            found | matches
        })
    })
}

pub fn add_device(path: &Path, name: &str, token_file: &Path) -> Result<()> {
    if name.is_empty()
        || name.len() > 40
        || !name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
    {
        return Err(eyre!(
            "Use a device name of up to 40 letters, numbers, dots, dashes or underscores."
        ));
    }
    locked(path, || {
        let mut list = devices(path)?;
        if list.iter().any(|d| d.name == name) {
            return Err(eyre!(
                "That device already exists. Revoke it before pairing again."
            ));
        }
        let token = format!("{}{}", identifier()?, identifier()?);
        // Refuse to overwrite any existing file, especially the host's own state.
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        use std::io::Write;
        let mut file = options.open(token_file)?;
        file.write_all(token.as_bytes())?;
        file.sync_all()?;
        list.push(Device {
            name: name.into(),
            hash: hash(token.as_bytes()),
        });
        write_private(&path.join("devices.json"), &serde_json::to_vec(&list)?)
    })
}

pub fn revoke(path: &Path, name: &str) -> Result<()> {
    locked(path, || {
        let mut list = devices(path)?;
        list.retain(|d| d.name != name);
        write_private(&path.join("devices.json"), &serde_json::to_vec(&list)?)
    })
}

/// Explicit migration only. Preserve the old files for rollback; never import audio or auth sessions.
pub fn import_companion(path: &Path, old: &Path) -> Result<()> {
    locked(path, || {
        if path.join("devices.json").exists() {
            return Err(eyre!(
                "A hosted profile already exists; refusing to overwrite it."
            ));
        }
        let profile =
            decode(&fs::read_to_string(old.join("profile.json"))?).map_err(|e| eyre!(e))?;
        let tokens = fs::read_to_string(old.join("tokens"))?;
        let mut list = Vec::new();
        for line in tokens.lines().filter(|line| !line.trim().is_empty()) {
            let fields: Vec<_> = line.split_whitespace().collect();
            if fields.len() != 2
                || fields[0].len() != 64
                || !fields[0].bytes().all(|b| b.is_ascii_hexdigit())
            {
                return Err(eyre!("The companion token file is malformed."));
            }
            list.push(Device {
                hash: fields[0].to_lowercase(),
                name: fields[1].into(),
            });
        }
        if path.join("state.json").exists() {
            let previous = load(path)?;
            if !previous.import_pending || previous.profile != profile {
                return Err(eyre!(
                    "A hosted profile already exists; refusing to overwrite it."
                ));
            }
            return write_private(&path.join("devices.json"), &serde_json::to_vec(&list)?);
        }
        // The imported profile is pending application, not an empty baseline
        // against which current defaults should win on first contact.
        save(
            path,
            &State {
                device: identifier()?,
                applied: profile.clone(),
                profile,
                import_pending: true,
            },
        )?;
        write_private(&path.join("devices.json"), &serde_json::to_vec(&list)?)
    })
}

#[cfg(test)]
mod tests;
