//! DCC (Direct Client-to-Client) protocol types.
//!
//! DCC messages are CTCP payloads inside `PRIVMSG`:
//! ```text
//! \x01DCC CHAT chat <ip> <port>\x01
//! \x01DCC SEND <filename> <ip> <port> <filesize>\x01
//! ```
//!
//! IP addresses are encoded as big-endian `u32` integers.

use std::net::Ipv4Addr;

use bytes::{BufMut, Bytes, BytesMut};

/// Convert an IPv4 address to its big-endian `u32` representation used in DCC.
#[must_use]
pub fn ip_to_u32(ip: Ipv4Addr) -> u32 {
    u32::from(ip)
}

/// Convert a big-endian `u32` back to an IPv4 address.
#[must_use]
pub fn u32_to_ip(n: u32) -> Ipv4Addr {
    Ipv4Addr::from(n)
}

/// Parsed DCC request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DccRequest {
    /// DCC CHAT — open a direct TCP chat connection.
    Chat {
        /// Peer IP address.
        ip: Ipv4Addr,
        /// Peer TCP port.
        port: u16,
    },
    /// DCC SEND — offer a file transfer.
    Send {
        /// Name of the offered file.
        filename: String,
        /// Peer IP address.
        ip: Ipv4Addr,
        /// Peer TCP port.
        port: u16,
        /// Size of the file in bytes.
        size: u64,
    },
}

impl DccRequest {
    /// Parse from a CTCP message body (the args after `DCC`).
    ///
    /// Expects the raw byte content: `CHAT chat <ip> <port>` or
    /// `SEND <filename> <ip> <port> <filesize>`.
    #[must_use]
    pub fn parse(args: &[u8]) -> Option<Self> {
        let s = std::str::from_utf8(args).ok()?;
        let mut parts = s.split_ascii_whitespace();
        let sub = parts.next()?;

        match sub.to_ascii_uppercase().as_str() {
            "CHAT" => {
                let _protocol = parts.next()?; // "chat"
                let ip_str = parts.next()?;
                let port_str = parts.next()?;
                let ip_num: u32 = ip_str.parse().ok()?;
                let port: u16 = port_str.parse().ok()?;
                Some(Self::Chat {
                    ip: u32_to_ip(ip_num),
                    port,
                })
            }
            "SEND" => {
                let filename = parts.next()?;
                let ip_str = parts.next()?;
                let port_str = parts.next()?;
                let size_str = parts.next()?;
                let ip_num: u32 = ip_str.parse().ok()?;
                let port: u16 = port_str.parse().ok()?;
                let size: u64 = size_str.parse().ok()?;
                Some(Self::Send {
                    filename: filename.to_owned(),
                    ip: u32_to_ip(ip_num),
                    port,
                    size,
                })
            }
            _ => None,
        }
    }

    /// Encode back to CTCP args (without the leading `DCC` command word).
    #[must_use]
    pub fn to_ctcp_args(&self) -> Bytes {
        match self {
            Self::Chat { ip, port } => {
                let s = format!("CHAT chat {} {port}", ip_to_u32(*ip));
                Bytes::from(s)
            }
            Self::Send {
                filename,
                ip,
                port,
                size,
            } => {
                let s = format!("SEND {filename} {} {port} {size}", ip_to_u32(*ip));
                Bytes::from(s)
            }
        }
    }

    /// Build a full CTCP payload (`\x01DCC ... \x01`) for embedding in a PRIVMSG.
    #[must_use]
    pub fn to_ctcp_payload(&self) -> Bytes {
        let args = self.to_ctcp_args();
        let mut buf = BytesMut::with_capacity(5 + args.len() + 2);
        buf.put_u8(0x01);
        buf.extend_from_slice(b"DCC ");
        buf.extend_from_slice(&args);
        buf.put_u8(0x01);
        buf.freeze()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_chat() {
        // 127.0.0.1 = 2130706433
        let req = DccRequest::parse(b"CHAT chat 2130706433 5000").unwrap();
        assert_eq!(
            req,
            DccRequest::Chat {
                ip: Ipv4Addr::LOCALHOST,
                port: 5000,
            }
        );
    }

    #[test]
    fn parse_send() {
        // 192.168.1.1 = 3232235777
        let req = DccRequest::parse(b"SEND example.txt 3232235777 6000 1048576").unwrap();
        assert_eq!(
            req,
            DccRequest::Send {
                filename: "example.txt".to_owned(),
                ip: Ipv4Addr::new(192, 168, 1, 1),
                port: 6000,
                size: 1_048_576,
            }
        );
    }

    #[test]
    fn roundtrip() {
        let chat = DccRequest::Chat {
            ip: Ipv4Addr::new(10, 0, 0, 1),
            port: 9999,
        };
        let args = chat.to_ctcp_args();
        let parsed = DccRequest::parse(&args).unwrap();
        assert_eq!(parsed, chat);

        let send = DccRequest::Send {
            filename: "file.bin".to_owned(),
            ip: Ipv4Addr::new(172, 16, 0, 5),
            port: 4321,
            size: 999_999,
        };
        let args = send.to_ctcp_args();
        let parsed = DccRequest::parse(&args).unwrap();
        assert_eq!(parsed, send);
    }

    #[test]
    fn ip_conversion() {
        let ip = Ipv4Addr::new(192, 168, 1, 1);
        assert_eq!(ip_to_u32(ip), 3_232_235_777);
        assert_eq!(u32_to_ip(3_232_235_777), ip);
    }

    #[test]
    fn parse_bad_input_returns_none() {
        assert!(DccRequest::parse(b"").is_none());
        assert!(DccRequest::parse(b"UNKNOWN foo bar").is_none());
        assert!(DccRequest::parse(b"CHAT chat notanumber 5000").is_none());
        assert!(DccRequest::parse(b"SEND").is_none());
    }
}
