//! A bounded attempt to open the user's SSH agent endpoint without sending any
//! agent protocol data. Used for authentication and opt-in forwarding.
use std::{io, path::Path, time::Duration};

#[cfg(unix)]
pub type AgentStream = tokio::net::UnixStream;
#[cfg(windows)]
pub type AgentStream = tokio::net::windows::named_pipe::NamedPipeClient;

pub async fn connect(path: &Path) -> io::Result<AgentStream> {
    tokio::time::timeout(Duration::from_millis(500), open(path))
        .await
        .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "SSH agent connection timed out"))?
}
#[cfg(unix)]
async fn open(path: &Path) -> io::Result<AgentStream> {
    tokio::net::UnixStream::connect(path).await
}
#[cfg(windows)]
async fn open(path: &Path) -> io::Result<AgentStream> {
    loop {
        match tokio::net::windows::named_pipe::ClientOptions::new().open(path) {
            Ok(stream) => return Ok(stream),
            // ERROR_PIPE_BUSY: another client is using the agent's pipe instance.
            Err(error) if error.raw_os_error() == Some(231) => {
                tokio::time::sleep(Duration::from_millis(50)).await
            }
            Err(error) => return Err(error),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    #[test]
    fn local_agent_stream_supports_bidirectional_bytes() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async {
            #[cfg(unix)]
            let directory = tempfile::Builder::new()
                .prefix("gl-pipe-")
                .tempdir_in("/tmp")
                .unwrap();
            #[cfg(unix)]
            let (mut client, mut server) = {
                let path = directory.path().join("agent.sock");
                let listener = tokio::net::UnixListener::bind(&path).unwrap();
                let (client, server) = tokio::join!(connect(&path), listener.accept());
                (client.unwrap(), server.unwrap().0)
            };
            #[cfg(windows)]
            let (mut client, mut server) = {
                let path = std::path::PathBuf::from(format!(
                    r"\\.\pipe\ghostlink-test-{}-{}",
                    std::process::id(),
                    std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap()
                        .as_nanos()
                ));
                let server = tokio::net::windows::named_pipe::ServerOptions::new()
                    .first_pipe_instance(true)
                    .create(&path)
                    .unwrap();
                let (client, ready) = tokio::join!(connect(&path), server.connect());
                ready.unwrap();
                (client.unwrap(), server)
            };
            client.write_all(b"request").await.unwrap();
            let mut request = [0; 7];
            server.read_exact(&mut request).await.unwrap();
            assert_eq!(&request, b"request");
            server.write_all(b"reply").await.unwrap();
            let mut reply = [0; 5];
            client.read_exact(&mut reply).await.unwrap();
            assert_eq!(&reply, b"reply");
        });
    }
}
