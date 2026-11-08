use std::io::{self, Read, Write};
use std::net::TcpStream;
use std::os::fd::AsRawFd;
use std::os::unix::net::UnixStream;
use std::thread;
use std::time::Duration;

use rustls::ServerConnection;

const CHUNK: usize = 16 * 1024;
const MAX_PENDING: usize = 256 * 1024;
const POLL_TIMEOUT_MS: i32 = 1000;
const HANGUP_BACKOFF: Duration = Duration::from_millis(5);

enum Step {
    Idle,
    Progress,
    Closed,
}

fn events(read: bool, write: bool) -> i16 {
    let mut mask = 0;
    if read {
        mask |= libc::POLLIN;
    }
    if write {
        mask |= libc::POLLOUT;
    }
    mask
}

fn poll(fds: &mut [libc::pollfd]) -> io::Result<()> {
    let rc = unsafe { libc::poll(fds.as_mut_ptr(), fds.len() as libc::nfds_t, POLL_TIMEOUT_MS) };
    if rc < 0 {
        let err = io::Error::last_os_error();
        if err.kind() == io::ErrorKind::Interrupted {
            return Ok(());
        }
        return Err(err);
    }
    Ok(())
}

fn flush_tls(conn: &mut ServerConnection, tcp: &mut TcpStream) -> io::Result<()> {
    while conn.wants_write() {
        match conn.write_tls(tcp) {
            Ok(0) => return Err(io::ErrorKind::WriteZero.into()),
            Ok(_) => {}
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => return Ok(()),
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e),
        }
    }
    Ok(())
}

fn flush_upstream(upstream: &mut UnixStream, pending: &mut Vec<u8>) -> io::Result<()> {
    let mut written = 0;
    let result = loop {
        if written == pending.len() {
            break Ok(());
        }
        match upstream.write(&pending[written..]) {
            Ok(0) => break Err(io::ErrorKind::WriteZero.into()),
            Ok(n) => written += n,
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => break Ok(()),
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(e) => break Err(e),
        }
    };
    pending.drain(..written);
    result
}

fn read_tcp(
    conn: &mut ServerConnection,
    tcp: &mut TcpStream,
    pending: &mut Vec<u8>,
    chunk: &mut [u8],
) -> io::Result<Step> {
    match conn.read_tls(tcp) {
        Ok(0) => Ok(Step::Closed),
        Ok(_) => {
            conn.process_new_packets()
                .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e.to_string()))?;
            loop {
                match conn.reader().read(chunk) {
                    Ok(0) => return Ok(Step::Closed),
                    Ok(n) => pending.extend_from_slice(&chunk[..n]),
                    Err(e) if e.kind() == io::ErrorKind::WouldBlock => break,
                    Err(e) => return Err(e),
                }
            }
            Ok(Step::Progress)
        }
        Err(e) if e.kind() == io::ErrorKind::WouldBlock => Ok(Step::Idle),
        Err(e) if e.kind() == io::ErrorKind::Interrupted => Ok(Step::Idle),
        Err(e) => Err(e),
    }
}

fn read_upstream(
    conn: &mut ServerConnection,
    tcp: &mut TcpStream,
    upstream: &mut UnixStream,
    chunk: &mut [u8],
) -> io::Result<Step> {
    match upstream.read(chunk) {
        Ok(0) => {
            conn.send_close_notify();
            let _ = flush_tls(conn, tcp);
            Ok(Step::Closed)
        }
        Ok(n) => {
            conn.writer().write_all(&chunk[..n])?;
            Ok(Step::Progress)
        }
        Err(e) if e.kind() == io::ErrorKind::WouldBlock => Ok(Step::Idle),
        Err(e) if e.kind() == io::ErrorKind::Interrupted => Ok(Step::Idle),
        Err(e) => Err(e),
    }
}

pub fn relay(
    mut conn: ServerConnection,
    mut tcp: TcpStream,
    mut upstream: UnixStream,
) -> io::Result<()> {
    tcp.set_read_timeout(None)?;
    tcp.set_write_timeout(None)?;
    tcp.set_nonblocking(true)?;
    upstream.set_nonblocking(true)?;
    conn.set_buffer_limit(Some(MAX_PENDING));

    let mut pending: Vec<u8> = Vec::new();
    let mut chunk = [0u8; CHUNK];

    loop {
        flush_tls(&mut conn, &mut tcp)?;
        flush_upstream(&mut upstream, &mut pending)?;

        let tls_backlog = conn.wants_write();
        let upstream_backlog = !pending.is_empty();
        let may_read_tcp = pending.len() < MAX_PENDING;
        let may_read_upstream = !tls_backlog;

        let mut fds = [
            libc::pollfd {
                fd: tcp.as_raw_fd(),
                events: events(may_read_tcp, tls_backlog),
                revents: 0,
            },
            libc::pollfd {
                fd: upstream.as_raw_fd(),
                events: events(may_read_upstream, upstream_backlog),
                revents: 0,
            },
        ];
        poll(&mut fds)?;

        let failed = libc::POLLERR | libc::POLLNVAL;
        if fds[0].revents & failed != 0 {
            return Err(io::Error::new(
                io::ErrorKind::ConnectionAborted,
                "network socket error",
            ));
        }
        if fds[1].revents & failed != 0 {
            return Err(io::Error::new(
                io::ErrorKind::ConnectionAborted,
                "upstream socket error",
            ));
        }

        let mut idle = true;
        if fds[0].revents != 0 && may_read_tcp {
            match read_tcp(&mut conn, &mut tcp, &mut pending, &mut chunk)? {
                Step::Closed => return Ok(()),
                Step::Progress => idle = false,
                Step::Idle => {}
            }
        }
        if fds[1].revents != 0 && may_read_upstream {
            match read_upstream(&mut conn, &mut tcp, &mut upstream, &mut chunk)? {
                Step::Closed => return Ok(()),
                Step::Progress => idle = false,
                Step::Idle => {}
            }
        }

        let hung_up = (fds[0].revents | fds[1].revents) & libc::POLLHUP != 0;
        if idle && hung_up {
            thread::sleep(HANGUP_BACKOFF);
        }
    }
}
