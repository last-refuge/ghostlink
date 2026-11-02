//! Proxy handshakes over either TCP or an existing SSH direct-tcpip channel.
use super::*;
use base64::Engine as _;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

pub(super) trait IoStream: AsyncRead + AsyncWrite + Unpin + Send {}
impl<T: AsyncRead + AsyncWrite + Unpin + Send> IoStream for T {}
pub(super) type Stream = Box<dyn IoStream>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProxyKind {
    Socks5,
    Socks4,
    Http,
}
#[derive(Clone)]
pub struct ProxyOptions {
    pub kind: ProxyKind,
    pub host: String,
    pub port: u16,
    pub username: String,
    pub password: Option<Zeroizing<String>>,
    pub on_password_accepted: Option<Arc<dyn Fn(Zeroizing<String>) + Send + Sync>>,
}
impl fmt::Debug for ProxyOptions {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ProxyOptions")
            .field("kind", &self.kind)
            .field("host", &self.host)
            .field("port", &self.port)
            .field("username", &self.username)
            .field("password", &self.password.as_ref().map(|_| "[redacted]"))
            .finish_non_exhaustive()
    }
}
impl ProxyOptions {
    pub fn validate(&self) -> Result<()> {
        ensure!(self.port > 0, "Proxy port must be between 1 and 65535");
        ensure!(
            !self.host.is_empty()
                && self.host.len() <= 253
                && self
                    .host
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || b".-_:%".contains(&byte)),
            "Enter a proxy hostname or IP address without a port"
        );
        ensure!(
            !self.host.contains(':') || self.host.parse::<std::net::Ipv6Addr>().is_ok(),
            "Enter an IPv6 address or a proxy hostname without a port"
        );
        ensure!(
            self.username.len() <= 255 && !self.username.chars().any(char::is_control),
            "Proxy username is invalid or longer than 255 bytes"
        );
        if self.kind == ProxyKind::Http {
            ensure!(
                !self.username.contains(':'),
                "An HTTP proxy username cannot contain a colon"
            );
        }
        if let Some(password) = &self.password {
            ensure!(
                password.len()
                    <= if self.kind == ProxyKind::Socks5 {
                        255
                    } else {
                        8192
                    },
                "Proxy password is too long"
            );
        }
        Ok(())
    }
    pub fn needs_password(&self) -> bool {
        self.kind != ProxyKind::Socks4 && !self.username.is_empty()
    }
}

pub(super) async fn tunnel(
    mut stream: Stream,
    proxy: &ProxyOptions,
    host: &str,
    port: u16,
) -> Result<Stream> {
    proxy.validate()?;
    ensure!(
        port > 0
            && !host.is_empty()
            && host
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b".-_:%".contains(&byte)),
        "Invalid proxy destination"
    );
    match proxy.kind {
        ProxyKind::Socks5 => {
            if proxy.needs_password() {
                let password = proxy
                    .password
                    .as_ref()
                    .context("A SOCKS5 proxy password is required")?;
                let tunnel = tokio_socks::tcp::Socks5Stream::connect_with_password_and_socket(
                    stream,
                    (host, port),
                    &proxy.username,
                    password.as_str(),
                )
                .await
                .context("SOCKS5 proxy connection failed")?;
                stream = tunnel.into_inner();
            } else {
                stream = tokio_socks::tcp::Socks5Stream::connect_with_socket(stream, (host, port))
                    .await
                    .context("SOCKS5 proxy connection failed")?
                    .into_inner();
            }
        }
        ProxyKind::Socks4 => {
            ensure!(
                host.parse::<std::net::Ipv6Addr>().is_err() && !host.contains(':'),
                "SOCKS4a cannot connect to IPv6; use SOCKS5 or HTTP CONNECT"
            );
            stream = tokio_socks::tcp::Socks4Stream::connect_with_userid_and_socket(
                stream,
                (host, port),
                &proxy.username,
            )
            .await
            .context("SOCKS4a proxy connection failed")?
            .into_inner();
        }
        ProxyKind::Http => http_connect(&mut stream, proxy, host, port).await?,
    }
    if proxy.needs_password()
        && let (Some(callback), Some(password)) = (&proxy.on_password_accepted, &proxy.password)
    {
        callback(password.clone());
    }
    Ok(stream)
}
async fn http_connect(
    stream: &mut Stream,
    proxy: &ProxyOptions,
    host: &str,
    port: u16,
) -> Result<()> {
    let authority = if host.contains(':') {
        format!("[{host}]:{port}")
    } else {
        format!("{host}:{port}")
    };
    let mut request = Zeroizing::new(format!(
        "CONNECT {authority} HTTP/1.1\r\nHost: {authority}\r\n"
    ));
    if proxy.needs_password() {
        let password = proxy
            .password
            .as_ref()
            .context("An HTTP proxy password is required")?;
        let credentials = Zeroizing::new(format!("{}:{}", proxy.username, password.as_str()));
        let encoded = Zeroizing::new(
            base64::engine::general_purpose::STANDARD.encode(credentials.as_bytes()),
        );
        request.push_str("Proxy-Authorization: Basic ");
        request.push_str(encoded.as_str());
        request.push_str("\r\n");
    }
    request.push_str("\r\n");
    stream
        .write_all(request.as_bytes())
        .await
        .context("Sending HTTP CONNECT")?;
    for _ in 0..4 {
        // Read exactly through the headers, leaving a coalesced SSH banner in the stream.
        let mut headers = Vec::new();
        while !headers.ends_with(b"\r\n\r\n") {
            ensure!(
                headers.len() < 16 * 1024,
                "HTTP proxy response headers exceed 16 KiB"
            );
            headers.push(
                stream
                    .read_u8()
                    .await
                    .context("Reading HTTP proxy response")?,
            );
        }
        let line = headers
            .split(|byte| *byte == b'\n')
            .next()
            .unwrap_or_default();
        let line = std::str::from_utf8(line).context("Invalid HTTP proxy status")?;
        let mut words = line.split_ascii_whitespace();
        ensure!(
            matches!(words.next(), Some("HTTP/1.0" | "HTTP/1.1")),
            "Invalid HTTP proxy version"
        );
        let code = words
            .next()
            .context("Missing HTTP proxy status")?
            .parse::<u16>()
            .context("Invalid HTTP proxy status")?;
        if (100..200).contains(&code) {
            continue;
        }
        ensure!(
            (200..300).contains(&code),
            "HTTP CONNECT proxy rejected the connection (status {code})"
        );
        return Ok(());
    }
    bail!("HTTP proxy sent too many interim responses")
}

#[cfg(test)]
mod tests {
    use super::*;
    fn options(kind: ProxyKind) -> ProxyOptions {
        ProxyOptions {
            kind,
            host: "proxy.invalid".into(),
            port: 1080,
            username: String::new(),
            password: None,
            on_password_accepted: None,
        }
    }
    fn runtime() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
    }
    fn run(future: impl std::future::Future<Output = ()>) {
        runtime().block_on(async {
            tokio::time::timeout(Duration::from_secs(5), future)
                .await
                .expect("Proxy fixture timed out");
        });
    }
    async fn headers(stream: &mut tokio::io::DuplexStream) -> Vec<u8> {
        let mut result = Vec::new();
        while !result.ends_with(b"\r\n\r\n") {
            result.push(stream.read_u8().await.unwrap());
        }
        result
    }
    #[test]
    fn http_connect_auth_ipv6_interim_response_and_coalesced_banner() {
        run(async {
            let (client, mut server) = tokio::io::duplex(32768);
            let job = tokio::spawn(async move {
                let request = String::from_utf8(headers(&mut server).await).unwrap();
                assert!(request.starts_with("CONNECT [::1]:2222 HTTP/1.1\r\n"));
                assert!(request.contains("Host: [::1]:2222\r\n"));
                assert!(request.contains("Proxy-Authorization: Basic dXNlcjpzZWNyZXQ="));
                server.write_all(b"HTTP/1.1 100 Continue\r\n\r\nHTTP/1.1 200 Connection established\r\n\r\nSSH-2.0-fixture\r\n").await.unwrap();
                let mut bytes = [0; 4];
                server.read_exact(&mut bytes).await.unwrap();
                assert_eq!(&bytes, b"ping");
                server.write_all(b"pong").await.unwrap();
            });
            let mut proxy = options(ProxyKind::Http);
            proxy.username = "user".into();
            proxy.password = Some(Zeroizing::new("secret".into()));
            assert!(!format!("{proxy:?}").contains("secret"));
            let mut stream = tunnel(Box::new(client), &proxy, "::1", 2222).await.unwrap();
            let mut banner = [0; 17];
            stream.read_exact(&mut banner).await.unwrap();
            assert_eq!(&banner, b"SSH-2.0-fixture\r\n");
            stream.write_all(b"ping").await.unwrap();
            let mut reply = [0; 4];
            stream.read_exact(&mut reply).await.unwrap();
            assert_eq!(&reply, b"pong");
            job.await.unwrap();
        });
    }
    #[test]
    fn http_rejection_and_bounded_headers_do_not_form_a_tunnel() {
        run(async {
            for reply in [
                b"HTTP/1.1 407 Proxy Authentication Required\r\n\r\n".to_vec(),
                b"X".repeat(16 * 1024 + 1),
            ] {
                let (client, mut server) = tokio::io::duplex(32768);
                let job = tokio::spawn(async move {
                    headers(&mut server).await;
                    server.write_all(&reply).await.unwrap();
                });
                let result = tunnel(
                    Box::new(client),
                    &options(ProxyKind::Http),
                    "host.invalid",
                    22,
                )
                .await;
                assert!(result.is_err());
                job.await.unwrap();
            }
        });
    }
    #[test]
    fn socks4a_sends_userid_and_unresolved_domain_without_local_dns() {
        run(async {
            let (client, mut server) = tokio::io::duplex(1024);
            let job = tokio::spawn(async move {
                let mut request = [0; 8];
                server.read_exact(&mut request).await.unwrap();
                assert_eq!(request, [4, 1, 0, 22, 0, 0, 0, 1]);
                let mut parts = Vec::new();
                let mut zeroes = 0;
                while zeroes < 2 {
                    let byte = server.read_u8().await.unwrap();
                    if byte == 0 {
                        zeroes += 1;
                    }
                    parts.push(byte);
                }
                assert_eq!(&parts, b"user\0unresolved.invalid\0");
                server.write_all(&[0, 90, 0, 0, 0, 0, 0, 0]).await.unwrap();
                server.write_all(b"ready").await.unwrap();
            });
            let mut proxy = options(ProxyKind::Socks4);
            proxy.username = "user".into();
            let mut stream = tunnel(Box::new(client), &proxy, "unresolved.invalid", 22)
                .await
                .unwrap();
            let mut reply = [0; 5];
            stream.read_exact(&mut reply).await.unwrap();
            assert_eq!(&reply, b"ready");
            job.await.unwrap();
        });
    }
    #[test]
    fn socks5_password_auth_and_ipv6_target_preserve_tunneled_bytes() {
        run(async {
            let (client, mut server) = tokio::io::duplex(1024);
            let job = tokio::spawn(async move {
                let mut greeting = [0; 4];
                server.read_exact(&mut greeting).await.unwrap();
                assert_eq!(greeting, [5, 2, 0, 2]);
                server.write_all(&[5, 2]).await.unwrap();
                let mut auth = [0; 13];
                server.read_exact(&mut auth).await.unwrap();
                assert_eq!(&auth, b"\x01\x04user\x06secret");
                server.write_all(&[1, 0]).await.unwrap();
                let mut request = [0; 22];
                server.read_exact(&mut request).await.unwrap();
                assert_eq!(&request[..4], &[5, 1, 0, 4]);
                assert_eq!(&request[4..20], &std::net::Ipv6Addr::LOCALHOST.octets());
                assert_eq!(&request[20..], &2222u16.to_be_bytes());
                server
                    .write_all(&[5, 0, 0, 1, 127, 0, 0, 1, 0, 22])
                    .await
                    .unwrap();
                server.write_all(b"ready").await.unwrap();
            });
            let mut proxy = options(ProxyKind::Socks5);
            proxy.username = "user".into();
            proxy.password = Some(Zeroizing::new("secret".into()));
            let mut stream = tunnel(Box::new(client), &proxy, "::1", 2222).await.unwrap();
            let mut ready = [0; 5];
            stream.read_exact(&mut ready).await.unwrap();
            assert_eq!(&ready, b"ready");
            job.await.unwrap();
        });
    }
    #[test]
    fn rejected_proxy_password_is_not_remembered() {
        run(async {
            let (client, mut server) = tokio::io::duplex(1024);
            let job = tokio::spawn(async move {
                let mut greeting = [0; 4];
                server.read_exact(&mut greeting).await.unwrap();
                server.write_all(&[5, 2]).await.unwrap();
                let mut auth = [0; 13];
                server.read_exact(&mut auth).await.unwrap();
                server.write_all(&[1, 1]).await.unwrap();
            });
            let accepted = Arc::new(AtomicBool::new(false));
            let captured = accepted.clone();
            let mut proxy = options(ProxyKind::Socks5);
            proxy.username = "user".into();
            proxy.password = Some(Zeroizing::new("secret".into()));
            proxy.on_password_accepted = Some(Arc::new(move |_| {
                captured.store(true, Ordering::Release);
            }));
            assert!(
                tunnel(Box::new(client), &proxy, "host.invalid", 22)
                    .await
                    .is_err()
            );
            assert!(!accepted.load(Ordering::Acquire));
            job.await.unwrap();
        });
    }
}
