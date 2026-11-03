// Copyright 2026, libkrun-sdk authors.
// SPDX-License-Identifier: Apache-2.0

//! Zero-Config Embedded Micro-DNS Responder for `*.cro.local`.
//!
//! Listens on UDP (default: 127.0.0.1:5354) and resolves all `*.cro.local`
//! queries to `127.0.0.1` (IPv4 A) and `::1` (IPv6 AAAA) without editing `/etc/hosts`.

use anyhow::{bail, Context, Result};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::path::Path;
use std::sync::Arc;
use tokio::net::UdpSocket;
use tokio::sync::broadcast;
use tracing::{debug, info, warn};

pub const DEFAULT_DNS_PORT: u16 = 5354;
pub const TYPE_A: u16 = 1;
pub const TYPE_AAAA: u16 = 28;
pub const CLASS_IN: u16 = 1;

/// Parses a binary DNS query packet into `(tx_id, qname, qtype)`.
pub fn parse_dns_query(packet: &[u8]) -> Result<(u16, String, u16)> {
    if packet.len() < 12 {
        bail!("DNS query packet too short (less than 12 bytes header)");
    }

    let tx_id = u16::from_be_bytes([packet[0], packet[1]]);
    let qdcount = u16::from_be_bytes([packet[4], packet[5]]);
    if qdcount == 0 {
        bail!("DNS packet contains no question");
    }

    // Parse QNAME starting at offset 12
    let mut offset = 12;
    let mut labels = Vec::new();

    while offset < packet.len() {
        let len = packet[offset] as usize;
        if len == 0 {
            offset += 1;
            break;
        }

        // Pointer / compression not allowed in standard query question QNAME
        if len & 0xc0 == 0xc0 {
            bail!("Compression pointer not permitted in DNS query question");
        }

        offset += 1;
        if offset + len > packet.len() {
            bail!("QNAME label exceeds packet bounds");
        }

        let label = std::str::from_utf8(&packet[offset..offset + len])
            .context("QNAME label is not valid UTF-8")?;
        labels.push(label.to_ascii_lowercase());
        offset += len;
    }

    if offset + 4 > packet.len() {
        bail!("Incomplete DNS question section (missing QTYPE or QCLASS)");
    }

    let qtype = u16::from_be_bytes([packet[offset], packet[offset + 1]]);
    let qclass = u16::from_be_bytes([packet[offset + 2], packet[offset + 3]]);

    if qclass != CLASS_IN {
        debug!("Non-IN DNS class queried: {}", qclass);
    }

    let qname = labels.join(".");
    Ok((tx_id, qname, qtype))
}

/// Constructs a valid binary DNS response for a resolved query.
pub fn build_dns_response(
    tx_id: u16,
    qname: &str,
    qtype: u16,
    ipv4: Ipv4Addr,
    ipv6: Ipv6Addr,
) -> Vec<u8> {
    let mut resp = Vec::with_capacity(128);

    // 1. Header (12 bytes)
    // ID
    resp.extend_from_slice(&tx_id.to_be_bytes());
    // Flags: QR=1 (response), AA=1 (authoritative), RD=1, RA=1 -> 0x8580 (or standard 0x8180)
    let has_answer = qtype == TYPE_A || qtype == TYPE_AAAA;
    let flags: u16 = if has_answer { 0x8580 } else { 0x8180 };
    resp.extend_from_slice(&flags.to_be_bytes());
    // QDCOUNT: 1
    resp.extend_from_slice(&1u16.to_be_bytes());
    // ANCOUNT: 1 if answerable, 0 otherwise
    let ancount: u16 = if has_answer { 1 } else { 0 };
    resp.extend_from_slice(&ancount.to_be_bytes());
    // NSCOUNT: 0, ARCOUNT: 0
    resp.extend_from_slice(&0u16.to_be_bytes());
    resp.extend_from_slice(&0u16.to_be_bytes());

    // 2. Question section
    let qname_start = resp.len();
    for label in qname.split('.') {
        if !label.is_empty() {
            resp.push(label.len() as u8);
            resp.extend_from_slice(label.as_bytes());
        }
    }
    resp.push(0x00); // Terminal null label
    resp.extend_from_slice(&qtype.to_be_bytes());
    resp.extend_from_slice(&CLASS_IN.to_be_bytes());

    // 3. Answer section
    if has_answer {
        // Name pointer to QNAME in Question section (0xc000 | qname_start)
        let name_ptr = 0xc000u16 | (qname_start as u16);
        resp.extend_from_slice(&name_ptr.to_be_bytes());
        // TYPE
        resp.extend_from_slice(&qtype.to_be_bytes());
        // CLASS: IN
        resp.extend_from_slice(&CLASS_IN.to_be_bytes());
        // TTL: 60 seconds
        resp.extend_from_slice(&60u32.to_be_bytes());

        if qtype == TYPE_A {
            // RDLENGTH: 4
            resp.extend_from_slice(&4u16.to_be_bytes());
            // RDATA: IPv4 octets
            resp.extend_from_slice(&ipv4.octets());
        } else {
            // RDLENGTH: 16
            resp.extend_from_slice(&16u16.to_be_bytes());
            // RDATA: IPv6 octets
            resp.extend_from_slice(&ipv6.octets());
        }
    }

    resp
}

/// Constructs a DNS NXDOMAIN response.
pub fn build_nxdomain_response(tx_id: u16, qname: &str, qtype: u16) -> Vec<u8> {
    let mut resp = Vec::with_capacity(128);

    // ID
    resp.extend_from_slice(&tx_id.to_be_bytes());
    // Flags: QR=1, RCODE=3 (NXDOMAIN) -> 0x8183
    resp.extend_from_slice(&0x8183u16.to_be_bytes());
    // QDCOUNT: 1, ANCOUNT: 0, NSCOUNT: 0, ARCOUNT: 0
    resp.extend_from_slice(&1u16.to_be_bytes());
    resp.extend_from_slice(&0u16.to_be_bytes());
    resp.extend_from_slice(&0u16.to_be_bytes());
    resp.extend_from_slice(&0u16.to_be_bytes());

    // Question
    for label in qname.split('.') {
        if !label.is_empty() {
            resp.push(label.len() as u8);
            resp.extend_from_slice(label.as_bytes());
        }
    }
    resp.push(0x00);
    resp.extend_from_slice(&qtype.to_be_bytes());
    resp.extend_from_slice(&CLASS_IN.to_be_bytes());

    resp
}

/// Active handle to the running embedded Micro-DNS server.
pub struct MicroDnsHandle {
    pub bind_addr: String,
    pub port: u16,
    shutdown_tx: broadcast::Sender<()>,
}

impl MicroDnsHandle {
    /// Signals the DNS server task to stop.
    pub fn shutdown(&self) {
        let _ = self.shutdown_tx.send(());
    }
}

pub struct MicroDnsServer;

impl MicroDnsServer {
    /// Starts the embedded Micro-DNS server listening on `bind_addr:port`.
    pub async fn start(bind_addr: &str, port: u16) -> Result<MicroDnsHandle> {
        let addr = format!("{}:{}", bind_addr, port);
        let socket = UdpSocket::bind(&addr)
            .await
            .with_context(|| format!("Failed to bind Micro-DNS UDP server to {}", addr))?;

        let (shutdown_tx, _) = broadcast::channel(16);
        let mut shutdown_rx = shutdown_tx.subscribe();
        let socket = Arc::new(socket);

        info!("🌐 Embedded Micro-DNS server listening on {}", addr);

        tokio::spawn(async move {
            let mut buf = [0u8; 1024];
            loop {
                tokio::select! {
                    _ = shutdown_rx.recv() => {
                        debug!("Micro-DNS server received shutdown signal");
                        break;
                    }
                    recv_res = socket.recv_from(&mut buf) => {
                        match recv_res {
                            Ok((size, peer)) => {
                                let query_bytes = &buf[..size];
                                match parse_dns_query(query_bytes) {
                                    Ok((tx_id, qname, qtype)) => {
                                        let is_match = qname.ends_with(".cro.local")
                                            || qname == "cro.local"
                                            || qname.ends_with(".localhost")
                                            || qname == "localhost";

                                        let resp = if is_match {
                                            build_dns_response(
                                                tx_id,
                                                &qname,
                                                qtype,
                                                Ipv4Addr::new(127, 0, 0, 1),
                                                Ipv6Addr::LOCALHOST,
                                            )
                                        } else {
                                            build_nxdomain_response(tx_id, &qname, qtype)
                                        };

                                        if let Err(e) = socket.send_to(&resp, peer).await {
                                            debug!("Failed to send DNS response to {}: {}", peer, e);
                                        }
                                    }
                                    Err(e) => {
                                        debug!("Malformed DNS query received from {}: {}", peer, e);
                                    }
                                }
                            }
                            Err(e) => {
                                warn!("Error reading from Micro-DNS socket: {}", e);
                            }
                        }
                    }
                }
            }
        });

        Ok(MicroDnsHandle {
            bind_addr: bind_addr.to_string(),
            port,
            shutdown_tx,
        })
    }
}

/// Helper function to configure native `/etc/resolver/cro.local` on macOS.
pub fn setup_macos_resolver(dns_port: u16) -> Result<String> {
    let resolver_dir = Path::new("/etc/resolver");
    let resolver_file = resolver_dir.join("cro.local");
    let content = format!(
        "# Generated by cro (libkrun microVM)\nnameserver 127.0.0.1\nport {}\n",
        dns_port
    );

    if resolver_file.exists() {
        if let Ok(existing) = std::fs::read_to_string(&resolver_file) {
            if existing.contains(&format!("port {}", dns_port)) {
                return Ok(format!(
                    "✅ Native resolver {} is already active on port {}.",
                    resolver_file.display(),
                    dns_port
                ));
            }
        }
    }

    match std::fs::create_dir_all(resolver_dir).and_then(|_| std::fs::write(&resolver_file, &content)) {
        Ok(_) => Ok(format!(
            "✅ Successfully created {} (routing *.cro.local -> 127.0.0.1:{}).",
            resolver_file.display(),
            dns_port
        )),
        Err(_) => Ok(format!(
            "💡 To enable system-wide *.cro.local resolution without editing /etc/hosts, run:\n   sudo mkdir -p /etc/resolver\n   sudo tee /etc/resolver/cro.local << 'EOF'\nnameserver 127.0.0.1\nport {}\nEOF",
            dns_port
        )),
    }
}

/// Sends a test DNS query to a Micro-DNS server and resolves the IP address.
pub async fn query_micro_dns(server_addr: &str, server_port: u16, domain: &str) -> Result<IpAddr> {
    let client = UdpSocket::bind("127.0.0.1:0")
        .await
        .context("Failed to bind UDP client socket for DNS query")?;

    let tx_id: u16 = 0xbeef;
    let mut query = Vec::with_capacity(64);
    // Header
    query.extend_from_slice(&tx_id.to_be_bytes());
    query.extend_from_slice(&0x0100u16.to_be_bytes()); // Standard query with RD=1
    query.extend_from_slice(&1u16.to_be_bytes()); // QDCOUNT=1
    query.extend_from_slice(&0u16.to_be_bytes());
    query.extend_from_slice(&0u16.to_be_bytes());
    query.extend_from_slice(&0u16.to_be_bytes());

    // QNAME
    for label in domain.split('.') {
        if !label.is_empty() {
            query.push(label.len() as u8);
            query.extend_from_slice(label.as_bytes());
        }
    }
    query.push(0x00);
    query.extend_from_slice(&TYPE_A.to_be_bytes());
    query.extend_from_slice(&CLASS_IN.to_be_bytes());

    let target = format!("{}:{}", server_addr, server_port);
    client
        .send_to(&query, &target)
        .await
        .with_context(|| format!("Failed to send DNS query to {}", target))?;

    let mut buf = [0u8; 512];
    let (n, _) = tokio::time::timeout(
        std::time::Duration::from_secs(2),
        client.recv_from(&mut buf),
    )
    .await
    .context("Timed out waiting for DNS response")?
    .context("Failed to receive DNS response")?;

    let resp = &buf[..n];
    if resp.len() < 12 {
        bail!("DNS response too short");
    }

    let ancount = u16::from_be_bytes([resp[6], resp[7]]);
    if ancount == 0 {
        bail!("DNS server returned 0 answers for '{}'", domain);
    }

    // Skip question section: find 0x00 terminal after offset 12
    let mut offset = 12;
    while offset < resp.len() && resp[offset] != 0 {
        offset += (resp[offset] as usize) + 1;
    }
    offset += 1 + 4; // null byte + QTYPE + QCLASS

    if offset + 12 > resp.len() {
        bail!("Malformed DNS response answer section");
    }

    // Answer name (2 bytes), TYPE (2), CLASS (2), TTL (4), RDLENGTH (2)
    let rdlength = u16::from_be_bytes([resp[offset + 10], resp[offset + 11]]) as usize;
    let rdata_start = offset + 12;

    if rdlength == 4 && rdata_start + 4 <= resp.len() {
        let ip = Ipv4Addr::new(
            resp[rdata_start],
            resp[rdata_start + 1],
            resp[rdata_start + 2],
            resp[rdata_start + 3],
        );
        Ok(IpAddr::V4(ip))
    } else {
        bail!("Unexpected RDLENGTH {} in DNS answer", rdlength);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_and_build_dns_query() {
        // Construct query for "llama.cro.local"
        let query = vec![
            0x12, 0x34, // ID
            0x01, 0x00, // Flags
            0x00, 0x01, // QDCOUNT = 1
            0x00, 0x00, // ANCOUNT
            0x00, 0x00, // NSCOUNT
            0x00, 0x00, // ARCOUNT
            5, b'l', b'l', b'a', b'm', b'a', 3, b'c', b'r', b'o', 5, b'l', b'o', b'c', b'a', b'l',
            0, // QNAME
            0x00, 0x01, // QTYPE = A
            0x00, 0x01, // QCLASS = IN
        ];

        let (tx_id, qname, qtype) = parse_dns_query(&query).unwrap();
        assert_eq!(tx_id, 0x1234);
        assert_eq!(qname, "llama.cro.local");
        assert_eq!(qtype, TYPE_A);

        let resp = build_dns_response(
            tx_id,
            &qname,
            qtype,
            Ipv4Addr::new(127, 0, 0, 1),
            Ipv6Addr::LOCALHOST,
        );
        assert!(resp.len() > query.len());
        // Verify response flag QR=1
        assert_eq!(resp[2] & 0x80, 0x80);
        // Verify answer contains 127.0.0.1
        assert_eq!(&resp[resp.len() - 4..], &[127, 0, 0, 1]);
    }

    #[tokio::test]
    async fn test_micro_dns_server_resolution() {
        let port = 15354;
        let handle = MicroDnsServer::start("127.0.0.1", port).await.unwrap();

        // 1. Resolve *.cro.local
        let ip = query_micro_dns("127.0.0.1", port, "web.cro.local")
            .await
            .unwrap();
        assert_eq!(ip, IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)));

        // 2. Resolve subdomains
        let ip2 = query_micro_dns("127.0.0.1", port, "api.service.cro.local")
            .await
            .unwrap();
        assert_eq!(ip2, IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)));

        // 3. Resolve localhost
        let ip3 = query_micro_dns("127.0.0.1", port, "my-app.localhost")
            .await
            .unwrap();
        assert_eq!(ip3, IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)));

        handle.shutdown();
    }
}
