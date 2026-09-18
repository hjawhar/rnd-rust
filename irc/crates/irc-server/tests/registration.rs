//! End-to-end: connect a raw TCP client, drive registration, expect
//! the welcome burst.

use std::time::Duration;

use bytes::BytesMut;
use futures_util::{SinkExt, StreamExt};
use irc_proto::{IrcCodec, Message, ReplyCode, Verb};
use irc_server::{Config, Server};
use tokio::net::TcpStream;
use tokio_util::codec::Framed;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn registration_happy_path_emits_welcome_burst() {
    let cfg = Config::builder().build().unwrap();
    let (server, shutdown) = Server::bind(cfg).await.unwrap();
    let addr = server.local_addrs()[0];
    let serve = tokio::spawn(server.serve());

    let stream = TcpStream::connect(addr).await.unwrap();
    let codec = IrcCodec::new();
    let mut conn = Framed::new(stream, codec);

    send(&mut conn, b"NICK alice").await;
    send(&mut conn, b"USER alice 0 * :Alice Example").await;

    let numerics = collect_until_numeric(&mut conn, ReplyCode::RPL_ENDOFMOTD).await;

    // Must include the first-line welcome, name info, and MOTD terminator.
    assert!(
        has_numeric(&numerics, ReplyCode::RPL_WELCOME),
        "missing 001 in {numerics:?}"
    );
    assert!(
        has_numeric(&numerics, ReplyCode::RPL_YOURHOST),
        "missing 002"
    );
    assert!(
        has_numeric(&numerics, ReplyCode::RPL_ISUPPORT),
        "missing 005"
    );
    assert!(
        has_numeric(&numerics, ReplyCode::RPL_MOTDSTART),
        "missing 375"
    );
    assert!(
        has_numeric(&numerics, ReplyCode::RPL_ENDOFMOTD),
        "missing 376"
    );

    send(&mut conn, b"QUIT :bye").await;
    // The server signals disconnect by closing the socket; the stream
    // ends naturally.
    let _ = tokio::time::timeout(Duration::from_secs(2), async {
        while conn.next().await.is_some() {}
    })
    .await;

    shutdown.signal();
    tokio::time::timeout(Duration::from_secs(5), serve)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn nick_collision_is_rejected() {
    let cfg = Config::builder().build().unwrap();
    let (server, shutdown) = Server::bind(cfg).await.unwrap();
    let addr = server.local_addrs()[0];
    let serve = tokio::spawn(server.serve());

    // First user registers successfully.
    let s1 = TcpStream::connect(addr).await.unwrap();
    let mut c1 = Framed::new(s1, IrcCodec::new());
    send(&mut c1, b"NICK alice").await;
    send(&mut c1, b"USER alice 0 * :Alice").await;
    let _ = collect_until_numeric(&mut c1, ReplyCode::RPL_ENDOFMOTD).await;

    // Second user tries the same (casemap-equivalent) nick.
    let s2 = TcpStream::connect(addr).await.unwrap();
    let mut c2 = Framed::new(s2, IrcCodec::new());
    send(&mut c2, b"NICK ALICE").await;
    let reply = expect_numeric(&mut c2, ReplyCode::ERR_NICKNAMEINUSE).await;
    assert!(reply.is_some());

    shutdown.signal();
    drop(c1);
    drop(c2);
    tokio::time::timeout(Duration::from_secs(5), serve)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ping_before_registration_is_answered() {
    let cfg = Config::builder().build().unwrap();
    let (server, shutdown) = Server::bind(cfg).await.unwrap();
    let addr = server.local_addrs()[0];
    let serve = tokio::spawn(server.serve());

    let s = TcpStream::connect(addr).await.unwrap();
    let mut c = Framed::new(s, IrcCodec::new());

    send(&mut c, b"PING :hello").await;
    let reply = recv_timeout(&mut c, Duration::from_secs(2))
        .await
        .expect("pong");
    match reply.verb {
        Verb::Word(ref w) if w.as_ref() == b"PONG" => {}
        other => panic!("expected PONG, got {other:?}"),
    }
    assert_eq!(&*reply.params[1], b"hello", "token echoed back as trailing");

    shutdown.signal();
    drop(c);
    tokio::time::timeout(Duration::from_secs(5), serve)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
}

// ----- helpers -----

async fn send(conn: &mut Framed<TcpStream, IrcCodec>, line: &[u8]) {
    let msg = Message::parse_slice(line).expect("valid test input");
    conn.send(msg).await.expect("send");
    // Small yield so the server can process before we race to read.
    tokio::task::yield_now().await;
}

fn has_numeric(msgs: &[Message], target: ReplyCode) -> bool {
    msgs.iter()
        .any(|m| matches!(m.verb, Verb::Numeric(code) if code == target.code()))
}

async fn recv_timeout(conn: &mut Framed<TcpStream, IrcCodec>, dur: Duration) -> Option<Message> {
    match tokio::time::timeout(dur, conn.next()).await {
        Ok(Some(Ok(msg))) => Some(msg),
        _ => None,
    }
}

async fn collect_until_numeric(
    conn: &mut Framed<TcpStream, IrcCodec>,
    terminator: ReplyCode,
) -> Vec<Message> {
    let mut out = Vec::new();
    while let Some(msg) = recv_timeout(conn, Duration::from_secs(3)).await {
        let is_end = matches!(msg.verb, Verb::Numeric(code) if code == terminator.code());
        out.push(msg);
        if is_end {
            return out;
        }
    }
    panic!("did not see terminator numeric {terminator:?}; saw {out:#?}");
}

async fn expect_numeric(
    conn: &mut Framed<TcpStream, IrcCodec>,
    target: ReplyCode,
) -> Option<Message> {
    let start = std::time::Instant::now();
    while start.elapsed() < Duration::from_secs(3) {
        if let Some(msg) = recv_timeout(conn, Duration::from_millis(500)).await {
            if matches!(msg.verb, Verb::Numeric(code) if code == target.code()) {
                return Some(msg);
            }
        }
    }
    None
}

// Silence unused-import warning when compiled standalone.
#[allow(dead_code)]
fn _keep_bytesmut() -> BytesMut {
    BytesMut::new()
}
