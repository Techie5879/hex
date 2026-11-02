//! `hex profile`: connects this computer to a sync server and syncs on request. Needs
//! the running Hex, whose own commands read and apply the settings, so a sync looks
//! like any other settings change. Local state is in `profile-sync/`, owner-only.
use std::fs;
use std::path::PathBuf;

use color_eyre::eyre::{Result, eyre};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::{adapter, decode, empty, encode, merge, transport};
use crate::app_settings::DictationPostProcessing;
use crate::control::cli::{Output, ask_running_hex};
use crate::control::{ControlCommand, ControlReply};
use crate::dictation::vocabulary::VocabularyEntry;

const MODE_KEY: &str = "dictation_processing.default_mode";
const MAX_ATTEMPTS: usize = 5;

#[derive(Debug, clap::Subcommand)]
pub enum ProfileCommand {
    /// Pair or revoke a phone for sync hosted by the running Mac app.
    Device {
        #[command(subcommand)]
        command: DeviceCommand,
    },
    /// Import the old companion's profile and token hashes without deleting its files.
    MigrateCompanion {
        directory: PathBuf,
        #[command(flatten)]
        output: Output,
    },
    /// Save the sync server's URL, and read its token from a file or stdin.
    Connect {
        url: String,
        /// File holding the token; `-` reads stdin. Never pass it as an argument.
        #[arg(long, value_name = "FILE")]
        token_file: String,
        #[command(flatten)]
        output: Output,
    },
    /// Merge with the server, then apply the result to this computer.
    Sync {
        #[command(flatten)]
        output: Output,
    },
    /// Show the server, the last sync, and open conflicts.
    Status {
        #[command(flatten)]
        output: Output,
    },
}

#[derive(Debug, clap::Subcommand)]
pub enum DeviceCommand {
    /// Write a new device token to a new private file. Never print it in shell history.
    Add {
        name: String,
        #[arg(long)]
        token_file: PathBuf,
        #[command(flatten)]
        output: Output,
    },
    /// Immediately revoke a device's access.
    Revoke {
        name: String,
        #[command(flatten)]
        output: Output,
    },
    /// List paired device names, never their tokens.
    List {
        #[command(flatten)]
        output: Output,
    },
}

#[derive(Deserialize, Serialize)]
struct Connection {
    url: String,
    token: String,
    device: String,
    #[serde(default)]
    last_sync_ms: u64,
}

fn directory() -> Result<PathBuf> {
    let directory = crate::app_paths::support_dir()?.join("profile-sync");
    fs::create_dir_all(&directory)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&directory, fs::Permissions::from_mode(0o700))?;
    }
    Ok(directory)
}

fn load_connection() -> Result<Connection> {
    let text = fs::read_to_string(directory()?.join("connection.json")).map_err(|_| {
        eyre!("Not connected. Run `hex profile connect <url> --token-file <file>`.")
    })?;
    Ok(serde_json::from_str(&text)?)
}

fn save_connection(connection: &Connection) -> Result<()> {
    let path = directory()?.join("connection.json");
    crate::durable::write(&path, serde_json::to_string(connection)?.as_bytes())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}

fn load_document(name: &str) -> Result<Value> {
    let path = directory()?.join(name);
    match fs::read_to_string(&path) {
        Ok(text) => decode(&text).map_err(|error| eyre!(error)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(empty()),
        Err(error) => Err(error.into()),
    }
}

fn save_document(name: &str, profile: &Value) -> Result<()> {
    crate::durable::write(&directory()?.join(name), encode(profile).as_bytes())?;
    Ok(())
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_millis() as u64)
}

fn ask(command: ControlCommand) -> Result<ControlReply> {
    match ask_running_hex(command) {
        ControlReply::Error { message, .. } => Err(eyre!(message)),
        reply => Ok(reply),
    }
}

/// The saved Global mode as JSON, so everything outside post-processing survives a write.
fn global_mode() -> Result<Value> {
    match ask(ControlCommand::SettingsGet {
        key: Some(MODE_KEY.into()),
    })? {
        ControlReply::Settings { settings } => settings
            .into_iter()
            .find(|setting| setting.key == MODE_KEY)
            .map(|setting| setting.value)
            .ok_or_else(|| eyre!("Hex has no {MODE_KEY}")),
        _ => Err(eyre!("Unexpected reply reading {MODE_KEY}")),
    }
}

fn post_processing(mode: &Value) -> Result<DictationPostProcessing> {
    serde_json::from_value(mode.get("post_processing").cloned().unwrap_or(json!({})))
        .map_err(Into::into)
}

fn vocabulary() -> Result<Vec<VocabularyEntry>> {
    match ask(ControlCommand::VocabularyList)? {
        ControlReply::Vocabulary { entries } => Ok(entries),
        _ => Err(eyre!("Unexpected reply reading the vocabulary")),
    }
}

fn sync(connection: &mut Connection) -> Result<Value> {
    if !crate::labs::profile_sync() {
        return Err(eyre!(
            "Profile sync is off. Turn on Labs › Profile sync first."
        ));
    }
    let mode = global_mode()?;
    let post = post_processing(&mode)?;
    let words = vocabulary()?;

    // Edits made here since the last complete apply become stamped changes before anything merges.
    let applied = load_document("applied.json")?;
    let mut local = load_document("profile.json")?;
    adapter::absorb(
        &mut local,
        &applied,
        &words,
        &post,
        &connection.device,
        now_ms(),
    );
    save_document("profile.json", &local)?;

    let ancestor = load_document("ancestor.json")?;
    let mut merged = None;
    for _ in 0..MAX_ATTEMPTS {
        let (remote, etag, exists) =
            match transport::fetch(&connection.url, &connection.token).map_err(|e| eyre!(e))? {
                transport::Fetched::Missing => (empty(), None, false),
                transport::Fetched::Document { text, etag } => {
                    (decode(&text).map_err(|e| eyre!(e))?, etag, true)
                }
            };
        let candidate = merge(&local, &remote, &ancestor);
        save_document("profile.json", &candidate)?;
        local = candidate.clone();
        let encoded = encode(&candidate);
        if exists && encoded == encode(&remote) {
            merged = Some(candidate);
            break;
        }
        match transport::store(
            &connection.url,
            &connection.token,
            &encoded,
            etag.as_deref(),
        )
        .map_err(|e| eyre!(e))?
        {
            transport::Stored::Done => {
                merged = Some(candidate);
                break;
            }
            transport::Stored::Changed => continue,
        }
    }
    let merged =
        merged.ok_or_else(|| eyre!("Another device kept changing the profile. Try again."))?;
    save_document("ancestor.json", &merged)?;

    // Apply through the running Hex: same validation and reload as any settings change.
    let wanted_post = adapter::apply(&merged, &post);
    if wanted_post != post {
        let mut mode = mode;
        mode["post_processing"] = serde_json::to_value(&wanted_post)?;
        ask(ControlCommand::SettingsSet {
            key: MODE_KEY.into(),
            value: mode.to_string(),
        })?;
    }
    let (removed, added) = adapter::vocabulary_changes(&words, &adapter::vocabulary(&merged));
    for term in removed {
        ask(ControlCommand::VocabularyRemove { term })?;
    }
    for entry in added {
        ask(ControlCommand::VocabularyAdd {
            term: entry.term,
            sounds_like: entry.sounds_like,
        })?;
    }
    // Only now does this computer match the document. A failure above retries next time.
    save_document("applied.json", &merged)?;
    connection.last_sync_ms = now_ms();
    save_connection(connection)?;
    Ok(merged)
}

fn device_id() -> Result<String> {
    let mut bytes = [0u8; 6];
    getrandom::fill(&mut bytes).map_err(|error| eyre!("{error}"))?;
    Ok(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
}

fn open_conflicts(profile: &Value) -> usize {
    super::section(profile, "conflicts")
        .values()
        .filter(|conflict| conflict.get("resolved_stamp").is_none())
        .count()
}

pub fn run(command: ProfileCommand) -> u8 {
    let (result, output) = match command {
        ProfileCommand::Device { command } => {
            let (result, output) = match command {
                DeviceCommand::Add { name, token_file, output } => (
                    super::host::directory().and_then(|p| super::host::add_device(&p, &name, &token_file))
                        .map(|()| ControlReply::done("Paired. Token saved to the private file. Turn on Labs › Profile sync in the Mac app.")), output),
                DeviceCommand::Revoke { name, output } => (
                    super::host::directory().and_then(|p| super::host::revoke(&p, &name))
                        .map(|()| ControlReply::done("Device revoked.")), output),
                DeviceCommand::List { output } => (
                    super::host::directory().and_then(|p| super::host::devices(&p))
                        .map(|list| ControlReply::unchanged(list.into_iter().map(|d| d.name).collect::<Vec<_>>().join("\n"))), output),
            };
            (result, output)
        }
        ProfileCommand::MigrateCompanion { directory, output } => (
            super::host::directory().and_then(|p| super::host::import_companion(&p, &directory))
                .map(|()| ControlReply::done("Imported the companion profile and paired devices. Old files are preserved.")), output),
        ProfileCommand::Connect {
            url,
            token_file,
            output,
        } => {
            let result = (|| -> Result<ControlReply> {
                transport::check_url(&url).map_err(|e| eyre!(e))?;
                let token = if token_file == "-" {
                    std::io::read_to_string(std::io::stdin())?
                } else {
                    fs::read_to_string(&token_file)?
                };
                let device = load_connection().map_or_else(|_| device_id(), |c| Ok(c.device))?;
                save_connection(&Connection {
                    url,
                    token: token.trim().to_owned(),
                    device,
                    last_sync_ms: 0,
                })?;
                Ok(ControlReply::Done {
                    message: "Connected. Run `hex profile sync`.".into(),
                    changed: true,
                })
            })();
            (result, output)
        }
        ProfileCommand::Sync { output } => {
            let result = load_connection()
                .and_then(|mut connection| sync(&mut connection))
                .map(|profile| ControlReply::Done {
                    message: format!(
                        "Synced {} terms. {} conflicts open.",
                        adapter::vocabulary(&profile).len(),
                        open_conflicts(&profile),
                    ),
                    changed: true,
                });
            (result, output)
        }
        ProfileCommand::Status { output } => {
            let result = load_connection().and_then(|connection| {
                let profile = load_document("profile.json")?;
                Ok(ControlReply::Done {
                    message: format!(
                        "Server: {}. Last sync: {}. {} terms. {} conflicts open.",
                        connection.url,
                        if connection.last_sync_ms == 0 {
                            "never".to_owned()
                        } else {
                            connection.last_sync_ms.to_string()
                        },
                        adapter::vocabulary(&profile).len(),
                        open_conflicts(&profile),
                    ),
                    changed: false,
                })
            }).or_else(|_| {
                let devices = super::host::devices(&super::host::directory()?)?;
                Ok(ControlReply::unchanged(format!(
                    "Desktop-hosted sync: {}. {} paired devices. Listener port: {} while Hex is running.",
                    if crate::labs::profile_sync() { "enabled" } else { "off" },
                    devices.len(), super::listener::PORT
                )))
            });
            (result, output)
        }
    };
    crate::control::cli::print(
        result.unwrap_or_else(|error| ControlReply::error("failed", format!("{error:#}"))),
        output,
    )
}
