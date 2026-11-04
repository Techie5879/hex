//! One remote document over HTTPS through the platform's curl: GET with its ETag,
//! PUT only if unchanged. The token stays off the command line, so it never shows
//! up in a process listing.
use std::io::Write;
use std::process::{Command, Stdio};

use super::MAX_BYTES;

pub enum Fetched {
    Missing,
    Document { text: String, etag: Option<String> },
}

pub enum Stored {
    Done,
    Changed,
}

/// Fails unless `url` is HTTPS, so a token is never sent in the clear.
pub fn check_url(url: &str) -> Result<(), String> {
    match url::Url::parse(url) {
        Ok(url) if url.scheme() == "https" && url.host().is_some() => Ok(()),
        _ => Err("The sync URL must start with https://".into()),
    }
}

/// `Authorization` for the companion's bearer token, or `user:password` for WebDAV.
fn authorization(token: &str) -> String {
    if token.contains(':') {
        format!(
            "Basic {}",
            crate::desktop::opencode_service::base64(token.as_bytes())
        )
    } else {
        format!("Bearer {token}")
    }
}

/// Curl's config file format: `"` and `\` escaped.
fn config_line(name: &str, value: &str) -> String {
    format!(
        "{name} = \"{}\"\n",
        value.replace('\\', "\\\\").replace('"', "\\\"")
    )
}

fn run(
    url: &str,
    token: &str,
    method: &str,
    headers: &[String],
    body: Option<&std::path::Path>,
) -> Result<(u16, Vec<String>, Vec<u8>), String> {
    let mut config = config_line("url", url);
    config.push_str(&config_line("request", method));
    config.push_str(&config_line(
        "header",
        &format!("Authorization: {}", authorization(token)),
    ));
    for header in headers {
        config.push_str(&config_line("header", header));
    }
    if let Some(body) = body {
        config.push_str(&config_line("data-binary", &format!("@{}", body.display())));
    }
    let mut child = Command::new(if cfg!(windows) { "curl.exe" } else { "curl" })
        .args([
            "--silent",
            "--show-error",
            "--include",
            "--proto",
            "=https",
            "--max-time",
            "30",
            "--max-filesize",
            "4194304",
            "--config",
            "-",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| format!("Could not run curl: {error}"))?;
    child
        .stdin
        .take()
        .expect("stdin is piped")
        .write_all(config.as_bytes())
        .map_err(|error| error.to_string())?;
    let output = child
        .wait_with_output()
        .map_err(|error| error.to_string())?;
    if !output.status.success() {
        return Err("Can't reach the sync server".into());
    }
    parse_response(&output.stdout)
}

/// Splits `--include` output, skipping any interim or proxy heads before the real one.
fn parse_response(mut bytes: &[u8]) -> Result<(u16, Vec<String>, Vec<u8>), String> {
    loop {
        let split = bytes
            .windows(4)
            .position(|w| w == b"\r\n\r\n")
            .ok_or("Malformed response")?;
        let head = String::from_utf8_lossy(&bytes[..split]).to_string();
        let mut lines = head.split("\r\n").map(str::to_owned);
        let status: u16 = lines
            .next()
            .and_then(|line| {
                line.split_whitespace()
                    .nth(1)
                    .and_then(|code| code.parse().ok())
            })
            .ok_or("Malformed response")?;
        let rest = &bytes[split + 4..];
        if (100..200).contains(&status)
            || (status == 200 && head.to_lowercase().starts_with("http/1.1 200 connection"))
        {
            bytes = rest;
            continue;
        }
        return Ok((status, lines.collect(), rest.to_vec()));
    }
}

fn status_error(status: u16) -> String {
    match status {
        401 | 403 => "The sync server refused the token".into(),
        _ => format!("The sync server said {status}"),
    }
}

pub fn fetch(url: &str, token: &str) -> Result<Fetched, String> {
    let (status, headers, body) = run(
        url,
        token,
        "GET",
        &["Accept: application/json".into()],
        None,
    )?;
    match status {
        200 => {
            if body.len() > MAX_BYTES {
                return Err("The remote profile is too large".into());
            }
            let etag = headers.iter().find_map(|line| {
                let (name, value) = line.split_once(':')?;
                name.eq_ignore_ascii_case("etag")
                    .then(|| value.trim().to_owned())
            });
            Ok(Fetched::Document {
                text: String::from_utf8(body).map_err(|_| "The remote profile isn't text")?,
                etag,
            })
        }
        404 => Ok(Fetched::Missing),
        _ => Err(status_error(status)),
    }
}

pub fn store(url: &str, token: &str, text: &str, if_match: Option<&str>) -> Result<Stored, String> {
    let directory = crate::app_paths::support_dir()
        .map_err(|error| error.to_string())?
        .join("profile-sync");
    let path = directory.join(".upload");
    crate::durable::write(&path, text.as_bytes()).map_err(|error| error.to_string())?;
    let condition = match if_match {
        Some(etag) => format!("If-Match: {etag}"),
        None => "If-None-Match: *".to_owned(),
    };
    let result = run(
        url,
        token,
        "PUT",
        &[
            "Content-Type: application/json".into(),
            "Expect:".into(),
            condition,
        ],
        Some(&path),
    );
    let _ = std::fs::remove_file(&path);
    let (status, _, _) = result?;
    match status {
        200..=299 => Ok(Stored::Done),
        412 => Ok(Stored::Changed),
        _ => Err(status_error(status)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_https_urls() {
        assert!(check_url("https://host.example.ts.net:8443/v1/profile").is_ok());
        assert!(check_url("http://host/v1/profile").is_err());
        assert!(check_url("nonsense").is_err());
    }

    #[test]
    fn tokens_pick_their_scheme() {
        assert_eq!(authorization("abc"), "Bearer abc");
        assert_eq!(authorization("user:pw"), "Basic dXNlcjpwdw==");
    }

    #[test]
    fn config_lines_escape_quotes() {
        assert_eq!(
            config_line("header", "a\"b\\c"),
            "header = \"a\\\"b\\\\c\"\n"
        );
    }

    #[test]
    fn responses_skip_interim_heads() {
        let raw = b"HTTP/1.1 100 Continue\r\n\r\nHTTP/2 201\r\netag: \"x\"\r\n\r\nbody";
        let (status, headers, body) = parse_response(raw).unwrap();
        assert_eq!(status, 201);
        assert_eq!(headers, vec!["etag: \"x\"".to_string()]);
        assert_eq!(body, b"body");
    }
}
