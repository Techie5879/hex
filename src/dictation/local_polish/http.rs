//! Bounded, cancellable HTTP to the child on loopback. Never follows redirects.
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::time::{Duration, Instant};

use color_eyre::eyre::{Result, bail, eyre};

const MAX_REPLY: usize = 2 * 1024 * 1024;

pub(super) fn request(
    address: SocketAddr,
    token: &str,
    method: &str,
    path: &str,
    body: Option<&[u8]>,
    until: Instant,
    check: &dyn Fn() -> Result<()>,
) -> Result<Vec<u8>> {
    check()?;
    let remaining = || until.saturating_duration_since(Instant::now());
    if remaining().is_zero() {
        bail!("Local cleanup timed out");
    }
    let mut stream =
        TcpStream::connect_timeout(&address, remaining().min(Duration::from_millis(200)))?;
    stream.set_write_timeout(Some(remaining().max(Duration::from_millis(1))))?;
    let head = format!(
        "{method} {path} HTTP/1.1\r\nHost: {address}\r\nAuthorization: Bearer {token}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.map_or(0, <[u8]>::len)
    );
    stream.write_all(head.as_bytes())?;
    stream.write_all(body.unwrap_or_default())?;
    let mut bytes = Vec::new();
    let mut buffer = [0; 8192];
    loop {
        check()?;
        if remaining().is_zero() {
            bail!("Local cleanup timed out");
        }
        stream.set_read_timeout(Some(
            remaining().clamp(Duration::from_millis(1), Duration::from_millis(50)),
        ))?;
        match stream.read(&mut buffer) {
            Ok(0) => {
                return complete(&bytes)?.ok_or_else(|| eyre!("Incomplete local cleanup response"));
            }
            Ok(n) => bytes.extend_from_slice(&buffer[..n]),
            Err(e)
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::WouldBlock
                        | std::io::ErrorKind::TimedOut
                        | std::io::ErrorKind::Interrupted
                ) =>
            {
                continue;
            }
            Err(e) => return Err(e.into()),
        }
        if bytes.len() > MAX_REPLY {
            bail!("Local cleanup response is too large");
        }
        if let Some(body) = complete(&bytes)? {
            return Ok(body);
        }
    }
}

fn complete(bytes: &[u8]) -> Result<Option<Vec<u8>>> {
    let Some(split) = bytes.windows(4).position(|w| w == b"\r\n\r\n") else {
        return Ok(None);
    };
    let head = std::str::from_utf8(&bytes[..split])?;
    if head.split_whitespace().nth(1) != Some("200") {
        bail!("Local cleanup worker returned an HTTP error");
    }
    let mut size = None;
    for line in head.lines().skip(1) {
        if let Some((key, value)) = line.split_once(':') {
            if key.eq_ignore_ascii_case("content-length") {
                size = Some(value.trim().parse::<usize>()?);
            }
            if key.eq_ignore_ascii_case("transfer-encoding") {
                bail!("Unexpected streaming cleanup response");
            }
        }
    }
    let size = size.ok_or_else(|| eyre!("Local cleanup response has no content length"))?;
    if size > MAX_REPLY {
        bail!("Local cleanup response is too large");
    }
    let body = &bytes[split + 4..];
    Ok((body.len() >= size).then(|| body[..size].to_vec()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;
    use std::thread;

    #[test]
    fn finishes_at_content_length_without_waiting_for_eof() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let (done, wait) = std::sync::mpsc::channel();
        let peer = thread::spawn(move || {
            let (mut s, _) = listener.accept().unwrap();
            let mut b = [0; 2048];
            let received = s.read(&mut b).unwrap();
            assert!(received > 0);
            s.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\n{}")
                .unwrap();
            let _ = wait.recv_timeout(Duration::from_secs(2));
        });
        let started = Instant::now();
        assert_eq!(
            request(
                addr,
                "secret",
                "GET",
                "/health",
                None,
                started + Duration::from_secs(1),
                &|| Ok(())
            )
            .unwrap(),
            b"{}"
        );
        done.send(()).unwrap();
        peer.join().unwrap();
    }

    #[test]
    fn stalled_read_observes_cancellation() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let peer = thread::spawn(move || {
            let (_s, _) = listener.accept().unwrap();
            thread::sleep(Duration::from_millis(300));
        });
        let started = Instant::now();
        let result = request(
            addr,
            "secret",
            "GET",
            "/health",
            None,
            started + Duration::from_secs(2),
            &|| {
                if started.elapsed() > Duration::from_millis(70) {
                    bail!("cancelled");
                }
                Ok(())
            },
        );
        assert!(result.unwrap_err().to_string().contains("cancelled"));
        peer.join().unwrap();
    }

    #[test]
    fn rejects_truncated_and_oversized_responses() {
        assert!(
            complete(b"HTTP/1.1 200 OK\r\nContent-Length: 8\r\n\r\n{}")
                .unwrap()
                .is_none()
        );
        assert!(complete(b"HTTP/1.1 200 OK\r\nContent-Length: 9999999\r\n\r\n").is_err());
        assert!(complete(b"HTTP/1.1 500 Error\r\nContent-Length: 0\r\n\r\n").is_err());
    }
}
