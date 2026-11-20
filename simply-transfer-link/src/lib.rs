pub mod client;
pub(crate) mod data;
pub mod paths;
pub mod protocol;
pub mod server;
pub mod trust;

#[derive(Debug, thiserror::Error)]
pub enum LinkError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("protocol error: {0}")]
    Protocol(String),
    #[error("frame too large: {0} bytes")]
    FrameTooLarge(u32),
    #[error("auth error: {0}")]
    Auth(String),
    #[error("path error: {0}")]
    Path(String),
    #[error("host key mismatch")]
    HostKeyMismatch,
}
