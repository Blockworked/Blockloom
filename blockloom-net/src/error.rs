use std::fmt;

#[derive(Debug)]
pub enum NetError {
    Io(std::io::Error),
    Quic(quiche::Error),
    Tls(String),
    /// A peer's reliable send queue is full; the caller decides whether to drop or disconnect.
    QueueFull,
    /// A datagram exceeds the negotiated maximum (or datagrams are unsupported).
    DatagramTooLarge {
        max: Option<usize>,
    },
    NotEstablished,
}

impl fmt::Display for NetError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            NetError::Io(e) => write!(f, "io: {e}"),
            NetError::Quic(e) => write!(f, "quic: {e}"),
            NetError::Tls(e) => write!(f, "tls: {e}"),
            NetError::QueueFull => write!(f, "reliable send queue is full"),
            NetError::DatagramTooLarge { max: Some(m) } => write!(f, "datagram over {m} bytes"),
            NetError::DatagramTooLarge { max: None } => write!(f, "datagrams are not available"),
            NetError::NotEstablished => write!(f, "connection is not established"),
        }
    }
}

impl std::error::Error for NetError {}

impl From<std::io::Error> for NetError {
    fn from(e: std::io::Error) -> Self {
        NetError::Io(e)
    }
}

impl From<quiche::Error> for NetError {
    fn from(e: quiche::Error) -> Self {
        NetError::Quic(e)
    }
}

impl From<boring::error::ErrorStack> for NetError {
    fn from(e: boring::error::ErrorStack) -> Self {
        NetError::Tls(e.to_string())
    }
}
