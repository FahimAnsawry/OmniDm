// SPDX-License-Identifier: GPL-3.0-or-later

//! A minimal HTTP proxy for tests. It logs the target it was asked to reach and can
//! require a username and password. Every host name leads to this computer, so a test
//! can use a made-up name that only works through the proxy.

use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::oneshot;

/// One client connection to the proxy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProxyConnection {
    /// `host:port` as the client asked for it.
    pub target: String,
    /// The right credentials were given (always true when none are required).
    pub authorized: bool,
}

type Credentials = Option<(String, String)>;

/// A running HTTP proxy on `127.0.0.1` with a random port, handling absolute-URI
/// requests and `CONNECT`. Stops when dropped.
pub struct TestProxy {
    addr: SocketAddr,
    log: Arc<Mutex<Vec<ProxyConnection>>>,
    shutdown: Option<oneshot::Sender<()>>,
}

impl TestProxy {
    pub async fn start(credentials: Option<(&str, &str)>) -> std::io::Result<Self> {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let addr = listener.local_addr()?;
        let log = Arc::new(Mutex::new(Vec::new()));
        let credentials: Credentials = credentials.map(|(u, p)| (u.into(), p.into()));
        let (tx, mut rx) = oneshot::channel();
        let accept_log = log.clone();
        tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = &mut rx => break,
                    accepted = listener.accept() => {
                        let Ok((client, _)) = accepted else { continue };
                        let log = accept_log.clone();
                        let credentials = credentials.clone();
                        tokio::spawn(async move {
                            let _ = serve_http(client, credentials, log).await;
                        });
                    }
                }
            }
        });
        Ok(Self {
            addr,
            log,
            shutdown: Some(tx),
        })
    }

    /// The proxy's URL, `http://127.0.0.1:port`.
    pub fn url(&self) -> String {
        format!("http://{}", self.addr)
    }

    pub fn addr(&self) -> SocketAddr {
        self.addr
    }

    /// Every connection so far, oldest first.
    pub fn connections(&self) -> Vec<ProxyConnection> {
        self.log.lock().unwrap().clone()
    }
}

impl Drop for TestProxy {
    fn drop(&mut self) {
        if let Some(tx) = self.shutdown.take() {
            let _ = tx.send(());
        }
    }
}

async fn serve_http(
    mut client: TcpStream,
    credentials: Credentials,
    log: Arc<Mutex<Vec<ProxyConnection>>>,
) -> std::io::Result<()> {
    // Read the request head.
    let mut head = Vec::new();
    let mut byte = [0u8; 1];
    while !head.ends_with(b"\r\n\r\n") {
        if client.read(&mut byte).await? == 0 {
            return Ok(());
        }
        head.push(byte[0]);
    }
    let text = String::from_utf8_lossy(&head).into_owned();
    let mut lines = text.split("\r\n");
    let request_line = lines.next().unwrap_or_default();
    let mut words = request_line.split(' ');
    let (method, uri) = (
        words.next().unwrap_or_default(),
        words.next().unwrap_or_default(),
    );
    let given = lines
        .filter_map(|line| line.split_once(':'))
        .find(|(name, _)| name.trim().eq_ignore_ascii_case("proxy-authorization"))
        .map(|(_, value)| value.trim().to_string());
    let authorized = match &credentials {
        None => true,
        Some((user, pass)) => given == Some(format!("Basic {}", base64(&format!("{user}:{pass}")))),
    };

    let target = if method == "CONNECT" {
        uri.to_string()
    } else {
        // Absolute form: http://host:port/path
        let rest = uri.strip_prefix("http://").unwrap_or(uri);
        let authority = rest.split('/').next().unwrap_or_default();
        if authority.contains(':') {
            authority.to_string()
        } else {
            format!("{authority}:80")
        }
    };
    log.lock().unwrap().push(ProxyConnection {
        target: target.clone(),
        authorized,
    });
    if !authorized {
        client
            .write_all(
                b"HTTP/1.1 407 Proxy Authentication Required\r\n\
                  Proxy-Authenticate: Basic realm=\"test\"\r\n\
                  Content-Length: 0\r\nConnection: close\r\n\r\n",
            )
            .await?;
        return Ok(());
    }
    // Every name is this computer, so tests can use addresses like `files.omnidm.test`
    // that only work through the proxy.
    let port = target.rsplit(':').next().unwrap_or("80");
    let mut upstream = TcpStream::connect(format!("127.0.0.1:{port}")).await?;
    if method == "CONNECT" {
        client
            .write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n")
            .await?;
    } else {
        upstream.write_all(&head).await?;
    }
    tokio::io::copy_bidirectional(&mut client, &mut upstream).await?;
    Ok(())
}

fn base64(text: &str) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let bytes = text.as_bytes();
    let mut out = String::new();
    for chunk in bytes.chunks(3) {
        let n = chunk
            .iter()
            .enumerate()
            .fold(0u32, |acc, (i, &b)| acc | (u32::from(b) << (16 - 8 * i)));
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(TABLE[(n >> (18 - 6 * i)) as usize & 63] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    #[test]
    fn base64_matches_the_standard_examples() {
        assert_eq!(super::base64("me:secret"), "bWU6c2VjcmV0");
        assert_eq!(super::base64("a"), "YQ==");
        assert_eq!(super::base64("ab"), "YWI=");
        assert_eq!(
            super::base64("Aladdin:open sesame"),
            "QWxhZGRpbjpvcGVuIHNlc2FtZQ=="
        );
    }
}
