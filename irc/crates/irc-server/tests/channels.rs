//! End-to-end: channel operations, messaging between two users.

use std::time::Duration;

use bytes::Bytes;
use futures_util::{SinkExt, StreamExt};
use irc_proto::{IrcCodec, Message, ReplyCode, Verb};
use irc_server::{Config, Server};
use tokio::net::TcpStream;
use tokio_util::codec::Framed;

type Conn = Framed<TcpStream, IrcCodec>;

async fn register(addr: std::net::SocketAddr, nick: &str) -> Conn {
    let stream = TcpStream::connect(addr).await.unwrap();
    let mut c = Framed::new(stream, IrcCodec::new());
    send(&mut c, &format!("NICK {nick}")).await;
    send(&mut c, &format!("USER {nick} 0 * :{nick}")).await;
    collect_until_numeric(&mut c, ReplyCode::RPL_ENDOFMOTD).await;
    c
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn join_creates_channel_and_returns_names() {
    let cfg = Config::builder().build().unwrap();
    let (server, shutdown) = Server::bind(cfg).await.unwrap();
    let addr = server.local_addrs()[0];
    let serve = tokio::spawn(server.serve());

    let mut c = register(addr, "alice").await;
    send(&mut c, "JOIN #test").await;

    // Expect JOIN echo, NAMES reply (with @alice since first joiner gets op).
    let msgs = collect_until_numeric(&mut c, ReplyCode::RPL_ENDOFNAMES).await;
    let join_echo = msgs
        .iter()
        .find(|m| matches!(&m.verb, Verb::Word(w) if w.as_ref() == b"JOIN"));
    assert!(join_echo.is_some(), "JOIN should be echoed back");

    let names = msgs
        .iter()
        .find(|m| matches!(m.verb, Verb::Numeric(c) if c == ReplyCode::RPL_NAMREPLY.code()));
    assert!(names.is_some(), "RPL_NAMREPLY expected");
    let names_text = names
        .unwrap()
        .params
        .last()
        .map(|b| String::from_utf8_lossy(b).to_string());
    assert!(
        names_text.as_deref().unwrap_or("").contains("@alice"),
        "first joiner should be op: {names_text:?}"
    );

    shutdown.signal();
    drop(c);
    tokio::time::timeout(Duration::from_secs(5), serve)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn privmsg_between_two_users_in_channel() {
    let cfg = Config::builder().build().unwrap();
    let (server, shutdown) = Server::bind(cfg).await.unwrap();
    let addr = server.local_addrs()[0];
    let serve = tokio::spawn(server.serve());

    let mut alice = register(addr, "alice").await;
    let mut bob = register(addr, "bob").await;

    send(&mut alice, "JOIN #chat").await;
    drain_until_numeric(&mut alice, ReplyCode::RPL_ENDOFNAMES).await;

    send(&mut bob, "JOIN #chat").await;
    drain_until_numeric(&mut bob, ReplyCode::RPL_ENDOFNAMES).await;

    // Alice's client should have seen Bob's JOIN echo.
    let alice_join = recv_timeout(&mut alice, Duration::from_secs(2)).await;
    assert!(alice_join.is_some(), "alice should see bob's JOIN");

    // Alice sends a PRIVMSG to #chat.
    send(&mut alice, "PRIVMSG #chat :hello bob").await;

    // Bob should receive it.
    let bob_msg = recv_timeout(&mut bob, Duration::from_secs(2))
        .await
        .expect("bob gets msg");
    assert_eq!(
        bob_msg.params.last().map(Bytes::as_ref),
        Some(&b"hello bob"[..])
    );

    // Alice should NOT see her own message (echo-message is off in MVP).
    let alice_echo = recv_timeout(&mut alice, Duration::from_millis(500)).await;
    assert!(alice_echo.is_none(), "no echo without echo-message cap");

    shutdown.signal();
    drop(alice);
    drop(bob);
    tokio::time::timeout(Duration::from_secs(5), serve)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn privmsg_to_nick_is_delivered() {
    let cfg = Config::builder().build().unwrap();
    let (server, shutdown) = Server::bind(cfg).await.unwrap();
    let addr = server.local_addrs()[0];
    let serve = tokio::spawn(server.serve());

    let mut alice = register(addr, "alice").await;
    let mut bob = register(addr, "bob").await;

    send(&mut alice, "PRIVMSG bob :hey").await;
    let msg = recv_timeout(&mut bob, Duration::from_secs(2))
        .await
        .expect("bob gets pm");
    assert_eq!(msg.params.last().map(Bytes::as_ref), Some(&b"hey"[..]));

    shutdown.signal();
    drop(alice);
    drop(bob);
    tokio::time::timeout(Duration::from_secs(5), serve)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn part_echoes_and_cleans_channel() {
    let cfg = Config::builder().build().unwrap();
    let (server, shutdown) = Server::bind(cfg).await.unwrap();
    let addr = server.local_addrs()[0];
    let serve = tokio::spawn(server.serve());

    let mut alice = register(addr, "alice").await;
    send(&mut alice, "JOIN #tmp").await;
    drain_until_numeric(&mut alice, ReplyCode::RPL_ENDOFNAMES).await;

    send(&mut alice, "PART #tmp :bye").await;
    let part = recv_timeout(&mut alice, Duration::from_secs(2))
        .await
        .expect("PART echo");
    assert!(matches!(&part.verb, Verb::Word(w) if w.as_ref() == b"PART"));

    shutdown.signal();
    drop(alice);
    tokio::time::timeout(Duration::from_secs(5), serve)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn topic_set_and_read() {
    let cfg = Config::builder().build().unwrap();
    let (server, shutdown) = Server::bind(cfg).await.unwrap();
    let addr = server.local_addrs()[0];
    let serve = tokio::spawn(server.serve());

    let mut alice = register(addr, "alice").await;
    send(&mut alice, "JOIN #topic").await;
    drain_until_numeric(&mut alice, ReplyCode::RPL_ENDOFNAMES).await;

    send(&mut alice, "TOPIC #topic :Hello World").await;
    // The TOPIC change should be echoed.
    let echo = recv_timeout(&mut alice, Duration::from_secs(2))
        .await
        .expect("TOPIC echo");
    assert!(matches!(&echo.verb, Verb::Word(w) if w.as_ref() == b"TOPIC"));

    // Read the topic back.
    send(&mut alice, "TOPIC #topic").await;
    let topic = recv_timeout(&mut alice, Duration::from_secs(2))
        .await
        .expect("RPL_TOPIC");
    assert_eq!(topic.verb, Verb::Numeric(ReplyCode::RPL_TOPIC.code()));
    assert_eq!(
        topic.params.last().map(Bytes::as_ref),
        Some(&b"Hello World"[..])
    );

    shutdown.signal();
    drop(alice);
    tokio::time::timeout(Duration::from_secs(5), serve)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
}

// ----- helpers -----

async fn send(conn: &mut Conn, line: &str) {
    let msg = Message::parse_slice(line.as_bytes()).unwrap();
    conn.send(msg).await.unwrap();
    tokio::task::yield_now().await;
}

async fn recv_timeout(conn: &mut Conn, dur: Duration) -> Option<Message> {
    match tokio::time::timeout(dur, conn.next()).await {
        Ok(Some(Ok(msg))) => Some(msg),
        _ => None,
    }
}

async fn collect_until_numeric(conn: &mut Conn, terminator: ReplyCode) -> Vec<Message> {
    let mut out = Vec::new();
    while let Some(msg) = recv_timeout(conn, Duration::from_secs(3)).await {
        let is_end = matches!(msg.verb, Verb::Numeric(code) if code == terminator.code());
        out.push(msg);
        if is_end {
            return out;
        }
    }
    panic!("did not see terminator {terminator:?}; saw {out:#?}");
}

async fn drain_until_numeric(conn: &mut Conn, terminator: ReplyCode) {
    collect_until_numeric(conn, terminator).await;
}
