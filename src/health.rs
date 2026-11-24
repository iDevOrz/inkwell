//! Health probe shared by the `/healthz` route and the `inkwell healthcheck`
//! command. The probe speaks plain HTTP/1.0 over std sockets so the runtime
//! image needs no curl/wget.

use std::io::{Read, Write};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, TcpStream};
use std::time::Duration;

use anyhow::{Context, Result, bail};

pub const HEALTH_PATH: &str = "/healthz";
const PROBE_TIMEOUT: Duration = Duration::from_secs(3);
/// The status line is all we need; cap reads so a bad peer cannot stall us.
const MAX_RESPONSE_BYTES: u64 = 4096;

/// Connects to the server at `bind_addr` and succeeds only on HTTP 200.
pub fn probe(bind_addr: SocketAddr) -> Result<()> {
    let target = reachable_addr(bind_addr);
    let mut stream = TcpStream::connect_timeout(&target, PROBE_TIMEOUT)
        .with_context(|| format!("connect to {target}"))?;
    stream.set_read_timeout(Some(PROBE_TIMEOUT))?;
    stream.set_write_timeout(Some(PROBE_TIMEOUT))?;
    write!(
        stream,
        "GET {HEALTH_PATH} HTTP/1.0\r\nHost: localhost\r\nConnection: close\r\n\r\n"
    )
    .context("send health request")?;

    let mut response = String::new();
    stream
        .take(MAX_RESPONSE_BYTES)
        .read_to_string(&mut response)
        .context("read health response")?;
    let status_line = response.lines().next().unwrap_or_default();
    match parse_status(status_line) {
        Some(200) => Ok(()),
        Some(status) => bail!("{HEALTH_PATH} returned HTTP {status}"),
        None => bail!("unexpected response: {status_line:?}"),
    }
}

/// A server bound to 0.0.0.0 / [::] is probed on the matching loopback.
fn reachable_addr(bind_addr: SocketAddr) -> SocketAddr {
    let ip = match bind_addr.ip() {
        IpAddr::V4(ip) if ip.is_unspecified() => IpAddr::V4(Ipv4Addr::LOCALHOST),
        IpAddr::V6(ip) if ip.is_unspecified() => IpAddr::V6(Ipv6Addr::LOCALHOST),
        ip => ip,
    };
    SocketAddr::new(ip, bind_addr.port())
}

fn parse_status(status_line: &str) -> Option<u16> {
    let mut parts = status_line.split_whitespace();
    let version = parts.next()?;
    if !version.starts_with("HTTP/") {
        return None;
    }
    parts.next()?.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_status_lines() {
        assert_eq!(parse_status("HTTP/1.1 200 OK"), Some(200));
        assert_eq!(parse_status("HTTP/1.0 503 Service Unavailable"), Some(503));
        assert_eq!(parse_status("garbage"), None);
        assert_eq!(parse_status(""), None);
    }

    #[test]
    fn unspecified_bind_addresses_are_probed_on_loopback() {
        assert_eq!(
            reachable_addr("0.0.0.0:3000".parse().unwrap()),
            "127.0.0.1:3000".parse().unwrap()
        );
        assert_eq!(
            reachable_addr("[::]:3000".parse().unwrap()),
            "[::1]:3000".parse().unwrap()
        );
        assert_eq!(
            reachable_addr("192.0.2.1:80".parse().unwrap()),
            "192.0.2.1:80".parse().unwrap()
        );
    }

    #[test]
    fn probe_fails_when_nothing_listens() {
        // Bind then drop to get a port that is very likely closed.
        let port = std::net::TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        assert!(probe(SocketAddr::from(([127, 0, 0, 1], port))).is_err());
    }
}
