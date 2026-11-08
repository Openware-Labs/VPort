// VPort — VPort gateway
// SPDX-License-Identifier: MIT
// Copyright 2026-present (C) Openware Labs
// Code Author(s): Openware Labs developers

mod relay;

use std::fs;
use std::io::{self, Write};
use std::net::{TcpListener, TcpStream};
use std::os::fd::{AsRawFd, RawFd};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use rustls::server::WebPkiClientVerifier;
use rustls::{RootCertStore, ServerConfig, ServerConnection};
use vport_proto::{Message, ProxyIdentity, MAX_IDENTITY_BYTES};

const DEFAULT_UPSTREAM: &str = "/run/vport/gateway.sock";
const HANDSHAKE_DEADLINE: Duration = Duration::from_secs(5);
const HANDSHAKE_STEP_TIMEOUT: Duration = Duration::from_secs(1);
const MAX_CONNECTIONS: usize = 64;
const MAX_HANDSHAKES: usize = 8;
const THREAD_STACK_BYTES: usize = 256 * 1024;
const KEEPALIVE_IDLE_SECS: libc::c_int = 10;
const KEEPALIVE_INTERVAL_SECS: libc::c_int = 5;
const KEEPALIVE_PROBES: libc::c_int = 3;

struct Settings {
    tls: Arc<ServerConfig>,
    allow_path: PathBuf,
    upstream: PathBuf,
}

struct Slot {
    counter: Arc<AtomicUsize>,
}

impl Slot {
    fn acquire(counter: &Arc<AtomicUsize>, limit: usize) -> Option<Slot> {
        let previous = counter.fetch_add(1, Ordering::SeqCst);
        if previous >= limit {
            counter.fetch_sub(1, Ordering::SeqCst);
            return None;
        }
        Some(Slot {
            counter: Arc::clone(counter),
        })
    }
}

impl Drop for Slot {
    fn drop(&mut self) {
        self.counter.fetch_sub(1, Ordering::SeqCst);
    }
}

fn required_env(name: &str) -> Result<String, String> {
    std::env::var(name).map_err(|_| format!("{name} is not set"))
}

fn load_certs(path: &str) -> Result<Vec<CertificateDer<'static>>, String> {
    CertificateDer::pem_file_iter(path)
    .map_err(|e| format!("cannot read {path}: {e}"))?
    .collect::<Result<Vec<_>, _>>()
    .map_err(|e| format!("invalid certificate in {path}: {e}"))
}

fn load_tls(cert_path: &str, key_path: &str, ca_path: &str) -> Result<Arc<ServerConfig>, String> {
    let mode = fs::metadata(key_path)
    .map_err(|e| format!("cannot read {key_path}: {e}"))?
    .permissions()
    .mode();
    if mode & 0o077 != 0 {
        return Err(format!(
            "{key_path} must not be accessible by group or others"
        ));
    }

    let certs = load_certs(cert_path)?;
    let key = PrivateKeyDer::from_pem_file(key_path)
    .map_err(|e| format!("invalid key in {key_path}: {e}"))?;

    let mut roots = RootCertStore::empty();
    for ca in load_certs(ca_path)? {
        roots
        .add(ca)
        .map_err(|e| format!("invalid CA certificate in {ca_path}: {e}"))?;
    }

    let verifier = WebPkiClientVerifier::builder(Arc::new(roots))
    .build()
    .map_err(|e| format!("cannot build the client verifier: {e}"))?;

    let mut config = ServerConfig::builder_with_protocol_versions(&[&rustls::version::TLS13])
    .with_client_cert_verifier(verifier)
    .with_single_cert(certs, key)
    .map_err(|e| format!("invalid server certificate or key: {e}"))?;
    config.send_tls13_tickets = 0;
    Ok(Arc::new(config))
}

fn load_allow_list(path: &Path) -> io::Result<Vec<String>> {
    let text = fs::read_to_string(path)?;
    Ok(text
    .lines()
    .map(str::trim)
    .filter(|line| !line.is_empty() && !line.starts_with('#'))
    .map(str::to_string)
    .collect())
}

fn set_socket_option(fd: RawFd, level: libc::c_int, name: libc::c_int, value: libc::c_int) {
    let rc = unsafe {
        libc::setsockopt(
            fd,
            level,
            name,
            &value as *const libc::c_int as *const libc::c_void,
            std::mem::size_of::<libc::c_int>() as libc::socklen_t,
        )
    };
    if rc < 0 {
        eprintln!(
            "vport-gateway: could not set a socket option: {}",
            io::Error::last_os_error()
        );
    }
}

fn set_keepalive(tcp: &TcpStream) {
    let fd = tcp.as_raw_fd();
    set_socket_option(fd, libc::SOL_SOCKET, libc::SO_KEEPALIVE, 1);
    set_socket_option(fd, libc::IPPROTO_TCP, libc::TCP_KEEPIDLE, KEEPALIVE_IDLE_SECS);
    set_socket_option(
        fd,
        libc::IPPROTO_TCP,
        libc::TCP_KEEPINTVL,
        KEEPALIVE_INTERVAL_SECS,
    );
    set_socket_option(fd, libc::IPPROTO_TCP, libc::TCP_KEEPCNT, KEEPALIVE_PROBES);
}

fn peer_identity(conn: &ServerConnection) -> Result<String, String> {
    let certs = conn
    .peer_certificates()
    .ok_or("the client sent no certificate")?;
    let leaf = certs.first().ok_or("the client sent no certificate")?;
    let (_, cert) = x509_parser::parse_x509_certificate(leaf.as_ref())
    .map_err(|e| format!("unreadable client certificate: {e}"))?;
    let name = cert
    .subject()
    .iter_common_name()
    .next()
    .ok_or("the client certificate has no Common Name")?
    .as_str()
    .map_err(|e| format!("invalid Common Name: {e}"))?
    .to_string();
    let valid = !name.is_empty()
    && name.len() <= MAX_IDENTITY_BYTES
    && name.bytes().all(|byte| (0x21..=0x7e).contains(&byte));
    if !valid {
        return Err("the Common Name is not a valid identity".to_string());
    }
    Ok(name)
}

fn handshake(
    mut tcp: TcpStream,
    settings: &Settings,
) -> Result<(ServerConnection, TcpStream, String), String> {
    tcp.set_read_timeout(Some(HANDSHAKE_STEP_TIMEOUT))
    .map_err(|e| e.to_string())?;
    tcp.set_write_timeout(Some(HANDSHAKE_STEP_TIMEOUT))
    .map_err(|e| e.to_string())?;
    let _ = tcp.set_nodelay(true);

    let mut conn = ServerConnection::new(Arc::clone(&settings.tls)).map_err(|e| e.to_string())?;
    let started = Instant::now();
    while conn.is_handshaking() {
        if started.elapsed() > HANDSHAKE_DEADLINE {
            return Err("handshake deadline exceeded".to_string());
        }
        conn.complete_io(&mut tcp)
        .map_err(|e| format!("handshake failed: {e}"))?;
    }

    let identity = peer_identity(&conn)?;
    let allowed = load_allow_list(&settings.allow_path)
    .map_err(|e| format!("cannot read the allow list: {e}"))?;
    if !allowed.iter().any(|name| *name == identity) {
        return Err(format!("identity {identity} is not allowed"));
    }

    set_keepalive(&tcp);
    Ok((conn, tcp, identity))
}

fn connect_upstream(path: &Path, identity: &str) -> io::Result<UnixStream> {
    let mut upstream = UnixStream::connect(path)?;
    let mut frame = Vec::new();
    Message::ProxyIdentity(ProxyIdentity {
        identity: identity.to_string(),
    })
    .encode_into(&mut frame);
    upstream.write_all(&frame)?;
    Ok(upstream)
}

fn serve(tcp: TcpStream, settings: &Settings, handshake_slot: Slot) {
    let peer = tcp
    .peer_addr()
    .map(|addr| addr.to_string())
    .unwrap_or_else(|_| "unknown".to_string());

    let (conn, tcp, identity) = match handshake(tcp, settings) {
        Ok(result) => result,
        Err(e) => {
            eprintln!("vport-gateway: {peer} refused: {e}");
            return;
        }
    };
    drop(handshake_slot);
    eprintln!("vport-gateway: {peer} connected as {identity}");

    let result = connect_upstream(&settings.upstream, &identity)
    .and_then(|upstream| relay::relay(conn, tcp, upstream));
    match result {
        Ok(()) => eprintln!("vport-gateway: {peer} ({identity}) disconnected"),
        Err(e) => eprintln!("vport-gateway: {peer} ({identity}) closed: {e}"),
    }
}

fn run() -> Result<(), String> {
    let listen = required_env("VPORT_GATEWAY_LISTEN")?;
    let cert = required_env("VPORT_GATEWAY_CERT")?;
    let key = required_env("VPORT_GATEWAY_KEY")?;
    let ca = required_env("VPORT_GATEWAY_CA")?;
    let allow_path = PathBuf::from(required_env("VPORT_GATEWAY_ALLOW")?);
    let upstream = PathBuf::from(
        std::env::var("VPORT_GATEWAY_UPSTREAM").unwrap_or_else(|_| DEFAULT_UPSTREAM.to_string()),
    );

    let tls = load_tls(&cert, &key, &ca)?;
    let allowed = load_allow_list(&allow_path)
    .map_err(|e| format!("cannot read {}: {e}", allow_path.display()))?;
    if allowed.is_empty() {
        return Err(format!(
            "{} lists no identity, nobody could connect",
            allow_path.display()
        ));
    }

    let listener =
    TcpListener::bind(&listen).map_err(|e| format!("cannot listen on {listen}: {e}"))?;
    eprintln!(
        "vport-gateway: listening on {listen}, {} identities allowed, upstream {}",
        allowed.len(),
              upstream.display()
    );

    let settings = Arc::new(Settings {
        tls,
        allow_path,
        upstream,
    });
    let active = Arc::new(AtomicUsize::new(0));
    let handshaking = Arc::new(AtomicUsize::new(0));

    for incoming in listener.incoming() {
        let tcp = match incoming {
            Ok(tcp) => tcp,
            Err(e) => {
                eprintln!("vport-gateway: accept failed: {e}");
                thread::sleep(Duration::from_millis(50));
                continue;
            }
        };
        let connection_slot = match Slot::acquire(&active, MAX_CONNECTIONS) {
            Some(slot) => slot,
            None => {
                eprintln!("vport-gateway: connection limit reached");
                continue;
            }
        };
        let handshake_slot = match Slot::acquire(&handshaking, MAX_HANDSHAKES) {
            Some(slot) => slot,
            None => {
                eprintln!("vport-gateway: too many handshakes in progress");
                continue;
            }
        };
        let settings = Arc::clone(&settings);
        let spawned = thread::Builder::new()
        .stack_size(THREAD_STACK_BYTES)
        .spawn(move || {
            serve(tcp, &settings, handshake_slot);
            drop(connection_slot);
        });
        if let Err(e) = spawned {
            eprintln!("vport-gateway: could not start a worker: {e}");
        }
    }
    Ok(())
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("vport-gateway: {err}");
            ExitCode::FAILURE
        }
    }
}
