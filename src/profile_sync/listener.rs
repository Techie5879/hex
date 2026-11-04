//! Loopback-only, profile-only listener. Owned and stopped by desktop Hex.
#![cfg_attr(not(target_os = "macos"), allow(dead_code))]
use std::io::Write;
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
    mpsc::{self, Receiver, SyncSender},
};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use super::host::{self, Exchange, Reply};
use crate::local_api::http::{read_body_bounded, read_request};
use color_eyre::Result;

pub const PORT: u16 = 8787;
pub struct Call {
    pub exchange: Exchange,
    pub reply: SyncSender<Reply>,
}
pub struct Listener {
    stop: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
}

impl Listener {
    pub fn start(directory: PathBuf, calls: SyncSender<Call>) -> Result<Self> {
        Self::bind(
            TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, PORT))?,
            directory,
            calls,
        )
    }
    fn bind(listener: TcpListener, directory: PathBuf, calls: SyncSender<Call>) -> Result<Self> {
        listener.set_nonblocking(true)?;
        let stop = Arc::new(AtomicBool::new(false));
        let shutdown = stop.clone();
        let worker = thread::Builder::new()
            .name("hex-profile-sync".into())
            .spawn(move || {
                while !shutdown.load(Ordering::Acquire) {
                    match listener.accept() {
                        Ok((mut stream, peer)) if peer.ip().is_loopback() => {
                            let _ = stream.set_read_timeout(Some(Duration::from_millis(100)));
                            let _ = stream.set_write_timeout(Some(Duration::from_secs(1)));
                            let response = handle(&mut stream, &directory, &calls, &shutdown);
                            let _ = write_reply(&mut stream, response);
                        }
                        Ok(_) => {}
                        Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                            thread::sleep(Duration::from_millis(50))
                        }
                        Err(_) => break,
                    }
                }
            })?;
        Ok(Self {
            stop,
            worker: Some(worker),
        })
    }
    pub fn stop(&self) {
        self.stop.store(true, Ordering::Release);
    }
    pub fn finished(&self) -> bool {
        self.worker.as_ref().is_none_or(|w| w.is_finished())
    }
}
impl Drop for Listener {
    fn drop(&mut self) {
        self.stop();
        // Never block the UI on a peer. Read and reply waits observe `stop`.
        self.worker.take();
    }
}

fn handle(
    stream: &mut TcpStream,
    directory: &std::path::Path,
    calls: &SyncSender<Call>,
    stop: &AtomicBool,
) -> Reply {
    let request = match read_request(stream, stop) {
        Ok(r) => r,
        Err(_) => return Reply::code(400),
    };
    let token = request
        .authorization
        .as_deref()
        .and_then(|s| s.strip_prefix("Bearer "))
        .unwrap_or("");
    if !host::authorized(directory, token) {
        return Reply::code(401);
    }
    if request.path == "/v1/health" && request.method == "GET" {
        return Reply {
            status: 200,
            body: "{\"ok\":true}".into(),
            etag: None,
        };
    }
    if request.path != "/v1/profile" {
        return Reply::code(404);
    }
    let exchange = match request.method.as_str() {
        "GET" => Exchange::Get,
        "PUT" => {
            let Some(length) = request.content_length else {
                return Reply::code(411);
            };
            if length > super::MAX_BYTES {
                return Reply::code(413);
            }
            let mut body = request.body_prefix;
            if read_body_bounded(
                stream,
                length,
                &mut body,
                stop,
                super::MAX_BYTES,
                Duration::from_secs(4),
            )
            .is_err()
            {
                return Reply::code(400);
            }
            let Ok(text) = String::from_utf8(body) else {
                return Reply::code(422);
            };
            Exchange::Put {
                text,
                if_match: request.if_match,
                if_none_match: request.if_none_match,
            }
        }
        _ => return Reply::code(405),
    };
    let (send, receive) = mpsc::sync_channel(1);
    if calls
        .try_send(Call {
            exchange,
            reply: send,
        })
        .is_err()
    {
        return Reply::code(503);
    }
    wait_reply(receive, stop)
}

fn wait_reply(receive: Receiver<Reply>, stop: &AtomicBool) -> Reply {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !stop.load(Ordering::Acquire) && Instant::now() < deadline {
        match receive.recv_timeout(Duration::from_millis(100)) {
            Ok(reply) => return reply,
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
            Err(mpsc::RecvTimeoutError::Timeout) => {}
        }
    }
    Reply::code(503)
}

fn write_reply(stream: &mut TcpStream, reply: Reply) -> std::io::Result<()> {
    // Status text is constant and contains no client-controlled bytes.
    write!(
        stream,
        "HTTP/1.1 {} Reply\r\nContent-Length: {}\r\nContent-Type: application/json\r\nCache-Control: no-store\r\nConnection: close\r\nX-Content-Type-Options: nosniff\r\n",
        reply.status,
        reply.body.len()
    )?;
    if let Some(etag) = reply.etag {
        write!(stream, "ETag: {etag}\r\n")?;
    }
    if reply.status == 401 {
        write!(stream, "WWW-Authenticate: Bearer\r\n")?;
    }
    write!(stream, "\r\n{}", reply.body)?;
    stream.flush()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    fn request(address: std::net::SocketAddr, headers: &str) -> String {
        let mut stream = TcpStream::connect(address).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        stream.write_all(headers.as_bytes()).unwrap();
        let mut response = String::new();
        stream.read_to_string(&mut response).unwrap();
        response
    }
    #[test]
    fn only_authenticated_profile_requests_reach_the_settings_owner() {
        let path = std::env::temp_dir().join(format!("hex-listener-test-{}", std::process::id()));
        host::private_directory(&path).unwrap();
        host::write_private(
            &path.join("devices.json"),
            &serde_json::to_vec(&vec![host::Device {
                name: "test".into(),
                hash: host::hash(b"synthetic-token"),
            }])
            .unwrap(),
        )
        .unwrap();
        let socket = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).unwrap();
        let address = socket.local_addr().unwrap();
        let (send, receive) = mpsc::sync_channel(2);
        let listener = Listener::bind(socket, path.clone(), send).unwrap();
        assert!(
            request(
                address,
                "GET /v1/profile HTTP/1.1\r\nHost: localhost\r\n\r\n"
            )
            .starts_with("HTTP/1.1 401")
        );
        assert!(
            request(
                address,
                "POST /control HTTP/1.1\r\nAuthorization: Bearer synthetic-token\r\n\r\n"
            )
            .starts_with("HTTP/1.1 404")
        );
        assert!(request(address,"PUT /v1/profile HTTP/1.1\r\nAuthorization: Bearer synthetic-token\r\nContent-Length: 1048577\r\n\r\n").starts_with("HTTP/1.1 413"));
        assert!(receive.try_recv().is_err());
        let owner = thread::spawn(move || {
            let call = receive.recv_timeout(Duration::from_secs(2)).unwrap();
            assert!(matches!(call.exchange, Exchange::Get));
            call.reply
                .send(Reply {
                    status: 200,
                    body: "{}".into(),
                    etag: Some("\"synthetic\"".into()),
                })
                .unwrap();
        });
        let response = request(
            address,
            "GET /v1/profile HTTP/1.1\r\nAuthorization: Bearer synthetic-token\r\n\r\n",
        );
        assert!(response.starts_with("HTTP/1.1 200"));
        assert!(response.contains("ETag: \"synthetic\""));
        owner.join().unwrap();
        listener.stop();
        let start = Instant::now();
        while !listener.finished() && start.elapsed() < Duration::from_secs(2) {
            thread::sleep(Duration::from_millis(10));
        }
        assert!(listener.finished());
        assert!(TcpStream::connect(address).is_err());
        std::fs::remove_dir_all(path).unwrap();
    }
}
