//! PROXY protocol v2 parser.
//!
//! Implements the minimal subset needed to extract the real client
//! address from a HAProxy-style PROXY protocol v2 header. Only
//! `AF_INET` (IPv4/TCP) and `AF_INET6` (IPv6/TCP) are supported.

use std::io;
use std::net::{Ipv4Addr, Ipv6Addr, SocketAddr, SocketAddrV4, SocketAddrV6};

use thiserror::Error;
use tokio::io::AsyncReadExt;
use tokio::net::TcpStream;

/// PROXY protocol v2 signature (12 bytes).
const SIGNATURE: [u8; 12] = [
    0x0D, 0x0A, 0x0D, 0x0A, 0x00, 0x0D, 0x0A, 0x51, 0x55, 0x49, 0x54, 0x0A,
];

/// Version 2 in the upper nibble of the 13th byte.
const VERSION_2: u8 = 0x20;

/// Command: PROXY (upper nibble = version, lower nibble = command).
const CMD_PROXY: u8 = 0x01;
/// Command: LOCAL (health check).
const CMD_LOCAL: u8 = 0x00;

/// `AF_INET` + STREAM (TCP).
const AF_INET_STREAM: u8 = 0x11;
/// `AF_INET6` + STREAM (TCP).
const AF_INET6_STREAM: u8 = 0x21;

/// Errors from PROXY protocol v2 header parsing.
#[derive(Debug, Error)]
pub enum ProxyProtoError {
    /// Underlying I/O failure while reading the header.
    #[error("proxy protocol I/O error: {0}")]
    Io(#[from] io::Error),
    /// The 12-byte signature did not match the v2 spec.
    #[error("invalid PROXY protocol v2 signature")]
    InvalidSignature,
    /// Version nibble is not 0x2.
    #[error("unsupported PROXY protocol version")]
    UnsupportedVersion,
    /// Address family/protocol combination is not supported.
    #[error("unsupported PROXY protocol address family")]
    UnsupportedFamily,
}

/// Read and parse a PROXY protocol v2 header from a raw TCP stream.
///
/// Returns `Some(addr)` for a proxied connection or `None` for a LOCAL
/// command (health-check probe). The caller should close the connection
/// cleanly on `None`.
pub async fn read_proxy_header(
    stream: &mut TcpStream,
) -> Result<Option<SocketAddr>, ProxyProtoError> {
    // 16-byte fixed header: 12 signature + ver/cmd + fam/proto + 2 length.
    let mut hdr = [0u8; 16];
    stream.read_exact(&mut hdr).await?;

    if hdr[..12] != SIGNATURE {
        return Err(ProxyProtoError::InvalidSignature);
    }

    let ver_cmd = hdr[12];
    let version = ver_cmd & 0xF0;
    let command = ver_cmd & 0x0F;

    if version != VERSION_2 {
        return Err(ProxyProtoError::UnsupportedVersion);
    }

    let fam_proto = hdr[13];
    let addr_len = u16::from_be_bytes([hdr[14], hdr[15]]) as usize;

    // Read the address block (variable length).
    let mut addr_buf = vec![0u8; addr_len];
    if addr_len > 0 {
        stream.read_exact(&mut addr_buf).await?;
    }

    if command == CMD_LOCAL {
        return Ok(None);
    }
    if command != CMD_PROXY {
        return Err(ProxyProtoError::UnsupportedVersion);
    }

    match fam_proto {
        AF_INET_STREAM => {
            // 4 src + 4 dst + 2 src_port + 2 dst_port = 12 bytes minimum.
            if addr_buf.len() < 12 {
                return Err(ProxyProtoError::Io(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "IPv4 address block too short",
                )));
            }
            let src_ip = Ipv4Addr::new(addr_buf[0], addr_buf[1], addr_buf[2], addr_buf[3]);
            let src_port = u16::from_be_bytes([addr_buf[8], addr_buf[9]]);
            Ok(Some(SocketAddr::V4(SocketAddrV4::new(src_ip, src_port))))
        }
        AF_INET6_STREAM => {
            // 16 src + 16 dst + 2 src_port + 2 dst_port = 36 bytes minimum.
            if addr_buf.len() < 36 {
                return Err(ProxyProtoError::Io(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "IPv6 address block too short",
                )));
            }
            let mut src = [0u8; 16];
            src.copy_from_slice(&addr_buf[..16]);
            let src_ip = Ipv6Addr::from(src);
            let src_port = u16::from_be_bytes([addr_buf[32], addr_buf[33]]);
            Ok(Some(SocketAddr::V6(SocketAddrV6::new(
                src_ip, src_port, 0, 0,
            ))))
        }
        _ => Err(ProxyProtoError::UnsupportedFamily),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::AsyncWriteExt;
    use tokio::net::TcpListener;

    /// Build a valid PROXY protocol v2 header for an IPv4 source.
    fn build_v2_ipv4_header(src: SocketAddrV4, dst: SocketAddrV4) -> Vec<u8> {
        let mut buf = Vec::new();
        buf.extend_from_slice(&SIGNATURE);
        buf.push(VERSION_2 | CMD_PROXY); // ver=2, cmd=PROXY
        buf.push(AF_INET_STREAM); // AF_INET + STREAM
        buf.extend_from_slice(&12u16.to_be_bytes()); // address length
        buf.extend_from_slice(&src.ip().octets());
        buf.extend_from_slice(&dst.ip().octets());
        buf.extend_from_slice(&src.port().to_be_bytes());
        buf.extend_from_slice(&dst.port().to_be_bytes());
        buf
    }

    fn build_v2_local_header() -> Vec<u8> {
        let mut buf = Vec::new();
        buf.extend_from_slice(&SIGNATURE);
        buf.push(VERSION_2 | CMD_LOCAL);
        buf.push(0x00); // AF_UNSPEC + UNSPEC
        buf.extend_from_slice(&0u16.to_be_bytes());
        buf
    }

    #[tokio::test]
    async fn parses_ipv4_proxy_header() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        let src: SocketAddrV4 = "10.0.0.42:12345".parse().unwrap();
        let dst: SocketAddrV4 = "192.168.1.1:6667".parse().unwrap();
        let header = build_v2_ipv4_header(src, dst);

        let client = tokio::spawn(async move {
            let mut stream = tokio::net::TcpStream::connect(addr).await.unwrap();
            stream.write_all(&header).await.unwrap();
            stream.shutdown().await.unwrap();
        });

        let (mut stream, _) = listener.accept().await.unwrap();
        let result = read_proxy_header(&mut stream).await.unwrap();
        assert_eq!(result, Some(SocketAddr::V4(src)));

        client.await.unwrap();
    }

    #[tokio::test]
    async fn parses_local_command_as_none() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        let header = build_v2_local_header();

        let client = tokio::spawn(async move {
            let mut stream = tokio::net::TcpStream::connect(addr).await.unwrap();
            stream.write_all(&header).await.unwrap();
            stream.shutdown().await.unwrap();
        });

        let (mut stream, _) = listener.accept().await.unwrap();
        let result = read_proxy_header(&mut stream).await.unwrap();
        assert_eq!(result, None);

        client.await.unwrap();
    }

    #[tokio::test]
    async fn rejects_bad_signature() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        let client = tokio::spawn(async move {
            let mut stream = tokio::net::TcpStream::connect(addr).await.unwrap();
            stream.write_all(&[0u8; 16]).await.unwrap();
            stream.shutdown().await.unwrap();
        });

        let (mut stream, _) = listener.accept().await.unwrap();
        let err = read_proxy_header(&mut stream).await.unwrap_err();
        assert!(matches!(err, ProxyProtoError::InvalidSignature));

        client.await.unwrap();
    }

    #[tokio::test]
    async fn parses_ipv6_proxy_header() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        let src_ip: Ipv6Addr = "2001:db8::1".parse().unwrap();
        let dst_ip: Ipv6Addr = "2001:db8::2".parse().unwrap();
        let src_port: u16 = 54321;
        let dst_port: u16 = 6667;

        let mut header = Vec::new();
        header.extend_from_slice(&SIGNATURE);
        header.push(VERSION_2 | CMD_PROXY);
        header.push(AF_INET6_STREAM);
        header.extend_from_slice(&36u16.to_be_bytes());
        header.extend_from_slice(&src_ip.octets());
        header.extend_from_slice(&dst_ip.octets());
        header.extend_from_slice(&src_port.to_be_bytes());
        header.extend_from_slice(&dst_port.to_be_bytes());

        let client = tokio::spawn(async move {
            let mut stream = tokio::net::TcpStream::connect(addr).await.unwrap();
            stream.write_all(&header).await.unwrap();
            stream.shutdown().await.unwrap();
        });

        let (mut stream, _) = listener.accept().await.unwrap();
        let result = read_proxy_header(&mut stream).await.unwrap();
        let expected = SocketAddr::V6(SocketAddrV6::new(src_ip, src_port, 0, 0));
        assert_eq!(result, Some(expected));

        client.await.unwrap();
    }
}
