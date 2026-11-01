//! M4.7: `https://` through rustls against in-process TLS servers whose self-signed CA is the
//! only trusted root; certificate errors are clear and never fall back to an insecure
//! connection; `http://` keeps working; "Reconnect now" cuts the backoff short.

use std::io::{ErrorKind, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::mpsc;

use rcgen::{BasicConstraints, CertificateParams, CertifiedIssuer, DnType, IsCa, KeyPair};
use rustls::pki_types::{PrivateKeyDer, PrivatePkcs8KeyDer};
use rustls::{ServerConfig, ServerConnection, StreamOwned};

use super::*;
use crate::http::{self, Endpoint};

/// A CA and a server certificate it signed.
struct Pki {
    ca: Vec<u8>,
    server: Arc<ServerConfig>,
}

fn ca(name: &str) -> CertifiedIssuer<'static, KeyPair> {
    let mut p = CertificateParams::new(Vec::<String>::new()).unwrap();
    p.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    p.distinguished_name.push(DnType::CommonName, name);
    CertifiedIssuer::self_signed(p, KeyPair::generate().unwrap()).unwrap()
}

/// A server certificate for `names` (host names or IP addresses), optionally long expired.
fn pki(names: &[&str], expired: bool) -> Pki {
    let ca = ca("TrackCraft test CA");
    let key = KeyPair::generate().unwrap();
    let mut p = CertificateParams::new(names.iter().map(|n| n.to_string()).collect::<Vec<_>>()).unwrap();
    if expired {
        p.not_before = rcgen::date_time_ymd(2000, 1, 1);
        p.not_after = rcgen::date_time_ymd(2001, 1, 1);
    }
    let leaf = p.signed_by(&key, &ca).unwrap();
    let chain = vec![leaf.der().clone(), ca.der().clone()];
    let pk = PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(key.serialize_der()));
    let server = ServerConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_no_client_auth()
        .with_single_cert(chain, pk)
        .unwrap();
    Pki { ca: ca.der().to_vec(), server: Arc::new(server) }
}

/// Reads a request head (up to the blank line) from `r`.
fn read_request(r: &mut impl Read) -> Option<String> {
    let mut head = Vec::new();
    let mut b = [0u8; 1];
    while !head.ends_with(b"\r\n\r\n") && head.len() < 64 * 1024 {
        match r.read(&mut b) {
            Ok(1) => head.push(b[0]),
            _ => return None,
        }
    }
    String::from_utf8(head).ok()
}

/// A TLS server answering every request with `response`; request heads go to the receiver.
fn canned_tls(pki: &Pki, response: &'static [u8]) -> (SocketAddr, mpsc::Receiver<String>) {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = l.local_addr().unwrap();
    let (tx, rx) = mpsc::channel();
    let cfg = Arc::clone(&pki.server);
    std::thread::spawn(move || {
        for sock in l.incoming().flatten() {
            let _ = sock.set_read_timeout(Some(Duration::from_secs(5)));
            let Ok(conn) = ServerConnection::new(Arc::clone(&cfg)) else { continue };
            let mut tls = StreamOwned::new(conn, sock);
            // A failed handshake (the client refused the certificate) ends here.
            let Some(head) = read_request(&mut tls) else { continue };
            let _ = tx.send(head);
            let _ = tls.write_all(response);
            tls.conn.send_close_notify();
            let _ = tls.flush();
        }
    });
    (addr, rx)
}

/// A plain-TCP server answering every request with `response`.
fn canned_plain(response: &'static [u8]) -> SocketAddr {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = l.local_addr().unwrap();
    std::thread::spawn(move || {
        for mut sock in l.incoming().flatten() {
            let _ = sock.set_read_timeout(Some(Duration::from_secs(5)));
            let mut buf = [0u8; 4096];
            let _ = sock.read(&mut buf);
            let _ = sock.write_all(response);
        }
    });
    addr
}

/// A TLS-terminating proxy in front of a plain server (the deployment docs/sync.md describes).
fn tls_proxy(pki: &Pki, backend: SocketAddr) -> SocketAddr {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = l.local_addr().unwrap();
    let cfg = Arc::clone(&pki.server);
    std::thread::spawn(move || {
        for sock in l.incoming().flatten() {
            let cfg = Arc::clone(&cfg);
            std::thread::spawn(move || {
                let Ok(conn) = ServerConnection::new(cfg) else { return };
                let Ok(mut back) = TcpStream::connect(backend) else { return };
                let tick = Some(Duration::from_millis(5));
                if sock.set_read_timeout(tick).is_err() || back.set_read_timeout(tick).is_err() {
                    return;
                }
                let mut tls = StreamOwned::new(conn, sock);
                let mut buf = vec![0u8; 64 * 1024];
                let idle = |e: &std::io::Error| matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut);
                loop {
                    match tls.read(&mut buf) {
                        Ok(0) => return,
                        Ok(n) => {
                            if back.write_all(&buf[..n]).is_err() {
                                return;
                            }
                        }
                        Err(e) if idle(&e) => {}
                        Err(_) => return,
                    }
                    match back.read(&mut buf) {
                        Ok(0) => {
                            tls.conn.send_close_notify();
                            let _ = tls.flush();
                            return;
                        }
                        Ok(n) => {
                            if tls.write_all(&buf[..n]).and_then(|_| tls.flush()).is_err() {
                                return;
                            }
                        }
                        Err(e) if idle(&e) => {}
                        Err(_) => return,
                    }
                }
            });
        }
    });
    addr
}

fn https(addr: SocketAddr, ca: &[u8]) -> Endpoint {
    Endpoint::parse(&format!("https://{addr}"), None).unwrap().with_ca_certs(&[ca.to_vec()]).unwrap()
}

const T: Duration = Duration::from_secs(5);

#[test]
fn https_request_and_stream_with_a_private_root() {
    let pki = pki(&["127.0.0.1", "localhost"], false);
    let (addr, heads) = canned_tls(&pki, b"HTTP/1.1 200 OK\r\nContent-Length: 11\r\nConnection: close\r\n\r\nhello world");
    let r = http::request(&https(addr, &pki.ca), "POST", "/api/command", "tct_x", Some(b"{}"), T, 1024).unwrap();
    assert_eq!((r.status, r.body.as_slice()), (200, &b"hello world"[..]));
    let head = heads.recv_timeout(T).unwrap();
    assert!(head.starts_with("POST /api/command HTTP/1.1\r\n") && head.contains("Authorization: Bearer tct_x\r\n"), "{head}");
    // A host name works as well as an address.
    let ep = Endpoint::parse(&format!("https://localhost:{}", addr.port()), None).unwrap().with_ca_certs(std::slice::from_ref(&pki.ca)).unwrap();
    assert_eq!(http::request(&ep, "GET", "/", "tct_x", None, T, 1024).unwrap().status, 200);
    // Server-Sent Events over TLS, chunked.
    let (addr, _) =
        canned_tls(&pki, b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\nd\r\nevent: hello\n\r\n9\r\ndata: {}\n\r\n1\r\n\n\r\n0\r\n\r\n");
    let mut s = http::open_stream(&https(addr, &pki.ca), "/api/sync/events", "tct_x", T, T).unwrap();
    assert_eq!(s.status, 200);
    let lines: Vec<String> = std::iter::from_fn(|| s.line(1024).unwrap()).collect();
    assert_eq!(lines, ["event: hello", "data: {}", ""]);
}

#[test]
fn certificate_problems_are_clear_errors() {
    let ok = b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nok";
    let get = |ep: &Endpoint| http::request(ep, "GET", "/", "tct_x", None, T, 1024).map(|r| r.status);
    // Signed by a CA the client doesn't trust.
    let good = pki(&["127.0.0.1"], false);
    let (addr, heads) = canned_tls(&good, ok);
    let other = ca("Some other CA");
    let e = get(&https(addr, other.der())).unwrap_err();
    assert!(e.contains("certificate was refused") && e.contains("trusts"), "{e}");
    // The system trust store doesn't know the test CA either (or the machine has no roots).
    let e = get(&Endpoint::parse(&format!("https://{addr}"), None).unwrap()).unwrap_err();
    assert!(e.contains("certificate"), "{e}");
    // Nothing was sent over a connection whose certificate failed: no token leaked.
    assert!(heads.recv_timeout(Duration::from_millis(200)).is_err());
    // Issued for another host.
    let wrong = pki(&["other.example"], false);
    let (addr, _) = canned_tls(&wrong, ok);
    let e = get(&https(addr, &wrong.ca)).unwrap_err();
    assert!(e.contains("different host name"), "{e}");
    // Expired.
    let old = pki(&["127.0.0.1"], true);
    let (addr, _) = canned_tls(&old, ok);
    let e = get(&https(addr, &old.ca)).unwrap_err();
    assert!(e.contains("expired"), "{e}");
    // https:// to a plain HTTP server.
    let plain = canned_plain(b"HTTP/1.1 400 Bad Request\r\nContent-Length: 0\r\n\r\n");
    let e = get(&https(plain, &good.ca)).unwrap_err();
    assert!(e.contains("TLS") || e.contains("https"), "{e}");
    // Nothing listening.
    let closed = TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap();
    assert!(get(&https(closed, &good.ca)).is_err());
    // A refused certificate fails `Remote::connect` with the reason, not an insecure retry.
    let mut c = cfg(&format!("https://{addr}"), RILEY);
    c.ca_certs = vec![old.ca.clone()];
    let e = Remote::connect(c, None).err().unwrap();
    assert!(e.contains("expired"), "{e}");
    // Junk roots are refused up front.
    let mut c = cfg(&format!("https://{addr}"), RILEY);
    c.ca_certs = vec![b"junk".to_vec()];
    assert!(Remote::connect(c, None).is_err());
}

#[test]
fn plain_http_still_works() {
    let addr = canned_plain(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nok");
    let ep = Endpoint::parse(&format!("http://{addr}"), None).unwrap();
    assert!(ep.is_loopback() && ep.tls.is_none());
    let r = http::request(&ep, "GET", "/", "tct_x", None, T, 1024).unwrap();
    assert_eq!((r.status, r.body.as_slice()), (200, &b"ok"[..]));
}

#[test]
fn remote_session_syncs_over_https() {
    let server = new_server();
    let run = Running::start(&server, "127.0.0.1:0");
    let pki = pki(&["127.0.0.1"], false);
    let front = tls_proxy(&pki, run.addr);
    let mut c = cfg(&format!("https://{front}"), RILEY);
    c.ca_certs = vec![pki.ca.clone()];
    let (remote, view) = Remote::connect(c, None).unwrap();
    assert!(remote.secure() && !remote.unencrypted_remote());
    assert!(remote.server().starts_with("https://127.0.0.1:"));
    let mut a = Client { remote, view, notices: vec![] };
    let mut b = Client::connect(&run.url(), SAM);
    settle(&mut [&mut a], "online over TLS", |cs| cs[0].remote.state() == &ConnState::Online);
    a.run("issue.edit", json!({"key": "LUME-1", "summary": "Sent over TLS"}));
    settle(&mut [&mut a, &mut b], "B sees A's TLS edit", converged(&server));
    assert_eq!(b.summary("LUME-1"), "Sent over TLS");
    b.run("issue.comment", json!({"key": "LUME-1", "body": "Streamed back over TLS"}));
    settle(&mut [&mut a, &mut b], "A sees B's comment over the TLS event stream", converged(&server));
    assert!(a.notices.iter().all(|n| !matches!(n, Notice::Dropped(_) | Notice::Resync(_))), "{:?}", a.notices);
}

#[test]
fn reconnect_now_skips_the_backoff() {
    let server = new_server();
    let mut run = Running::start(&server, "127.0.0.1:0");
    let addr = run.addr;
    let mut c = cfg(&run.url(), RILEY);
    c.max_backoff = Duration::from_secs(60);
    let (remote, view) = Remote::connect(c, None).unwrap();
    let mut a = Client { remote, view, notices: vec![] };
    assert!(!a.remote.secure() && !a.remote.unencrypted_remote(), "http:// on loopback is not flagged");
    settle(&mut [&mut a], "online", |cs| cs[0].remote.state() == &ConnState::Online);
    run.stop();
    settle(&mut [&mut a], "offline", |cs| matches!(cs[0].remote.state(), ConnState::Offline { .. }));
    assert!(a.remote.last_error().is_some());
    // Let the backoff grow (250 ms, 500 ms, 1 s, 2 s: the next wait is 4 s or more).
    let end = Instant::now() + Duration::from_millis(4000);
    while Instant::now() < end {
        a.pump();
        std::thread::sleep(Duration::from_millis(20));
    }
    let _run2 = Running::start(&server, &addr.to_string());
    a.remote.reconnect_now();
    let start = Instant::now();
    let end = start + Duration::from_secs(3);
    while a.remote.state() != &ConnState::Online {
        assert!(Instant::now() < end, "reconnect_now did not reconnect within 3 s");
        a.pump();
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(a.remote.last_error().is_none());
}
