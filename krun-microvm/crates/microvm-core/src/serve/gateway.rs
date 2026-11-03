// Copyright 2026, libkrun-sdk authors.
// SPDX-License-Identifier: Apache-2.0

//! Scale-to-Zero HTTP Gateway for `cro serve`.
//!
//! Provides automatic pausing of idle inference microVMs (dropping CPU to 0.0%)
//! and sub-2ms `SIGCONT` wake-on-request HTTP proxying.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{broadcast, Mutex};
use tracing::{debug, error, info, warn};

use crate::state::StateManager;

/// Snapshot of live gateway runtime metrics.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct GatewayStatsSnapshot {
    pub active_requests: usize,
    pub is_paused: bool,
    pub total_requests: u64,
    pub wake_count: u64,
    pub pause_count: u64,
    pub last_wake_latency_us: u64,
}

/// Atomic counters and metrics for the serving gateway.
#[derive(Debug, Default)]
pub struct GatewayStats {
    pub active_requests: AtomicUsize,
    pub is_paused: AtomicBool,
    pub total_requests: AtomicU64,
    pub wake_count: AtomicU64,
    pub pause_count: AtomicU64,
    pub last_wake_latency_us: AtomicU64,
}

impl GatewayStats {
    pub fn snapshot(&self) -> GatewayStatsSnapshot {
        GatewayStatsSnapshot {
            active_requests: self.active_requests.load(Ordering::Relaxed),
            is_paused: self.is_paused.load(Ordering::SeqCst),
            total_requests: self.total_requests.load(Ordering::Relaxed),
            wake_count: self.wake_count.load(Ordering::Relaxed),
            pause_count: self.pause_count.load(Ordering::Relaxed),
            last_wake_latency_us: self.last_wake_latency_us.load(Ordering::Relaxed),
        }
    }
}

/// Allocates an available ephemeral TCP port on the loopback interface.
pub fn find_available_loopback_port() -> Result<u16> {
    let listener = std::net::TcpListener::bind("127.0.0.1:0")
        .context("Failed to allocate ephemeral loopback port for scale-to-zero gateway")?;
    let port = listener.local_addr()?.port();
    drop(listener);
    Ok(port)
}

/// Control handle for the running scale-to-zero reverse proxy gateway.
pub struct ServeGatewayHandle {
    pub host: String,
    pub host_port: u16,
    pub backend_port: u16,
    pub vm_id: String,
    stats: Arc<GatewayStats>,
    shutdown_tx: broadcast::Sender<()>,
}

impl ServeGatewayHandle {
    /// Returns true if the associated microVM is currently paused in scale-to-zero.
    pub fn is_paused(&self) -> bool {
        self.stats.is_paused.load(Ordering::SeqCst)
    }

    /// Number of in-flight active HTTP requests currently being proxied.
    pub fn active_requests(&self) -> usize {
        self.stats.active_requests.load(Ordering::Relaxed)
    }

    /// Returns a point-in-time snapshot of the gateway's performance statistics.
    pub fn stats(&self) -> GatewayStatsSnapshot {
        self.stats.snapshot()
    }

    /// Gracefully signals the gateway server and background idle monitor to terminate.
    pub fn shutdown(&self) {
        let _ = self.shutdown_tx.send(());
    }
}

pub struct ServeGateway;

impl ServeGateway {
    /// Launches the scale-to-zero gateway server on `host:host_port`, forwarding to `backend_port`.
    pub async fn start(
        host: &str,
        host_port: u16,
        backend_port: u16,
        vm_id: &str,
        data_dir: PathBuf,
        idle_timeout: Duration,
    ) -> Result<ServeGatewayHandle> {
        let bind_addr = format!("{}:{}", host, host_port);
        let listener = TcpListener::bind(&bind_addr)
            .await
            .with_context(|| format!("Failed to bind scale-to-zero gateway to {}", bind_addr))?;

        let (shutdown_tx, _) = broadcast::channel(16);
        let stats = Arc::new(GatewayStats::default());
        let last_activity = Arc::new(Mutex::new(Instant::now()));

        info!(
            "⚡ Scale-to-Zero Gateway listening on {} (backend: 127.0.0.1:{}, idle timeout: {:?})",
            bind_addr, backend_port, idle_timeout
        );

        // 1. Connection Accept & Wake-on-Request Loop
        let stats_for_accept = Arc::clone(&stats);
        let last_activity_for_accept = Arc::clone(&last_activity);
        let mut shutdown_rx_accept = shutdown_tx.subscribe();
        let vm_id_clone = vm_id.to_string();
        let data_dir_clone = data_dir.clone();

        tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = shutdown_rx_accept.recv() => {
                        debug!("Scale-to-zero gateway accept loop received shutdown signal");
                        break;
                    }
                    accept_res = listener.accept() => {
                        match accept_res {
                            Ok((client_stream, _peer_addr)) => {
                                let stats = Arc::clone(&stats_for_accept);
                                let last_act = Arc::clone(&last_activity_for_accept);
                                let vm_id = vm_id_clone.clone();
                                let data_dir = data_dir_clone.clone();

                                tokio::spawn(async move {
                                    if let Err(e) = Self::handle_connection(
                                        client_stream,
                                        backend_port,
                                        &vm_id,
                                        &data_dir,
                                        stats,
                                        last_act,
                                    ).await {
                                        debug!("Scale-to-zero proxy connection error: {:#}", e);
                                    }
                                });
                            }
                            Err(e) => {
                                warn!("Scale-to-zero gateway error accepting connection: {}", e);
                            }
                        }
                    }
                }
            }
        });

        // 2. Background Auto-Pause Idle Monitor
        let stats_for_monitor = Arc::clone(&stats);
        let last_activity_for_monitor = Arc::clone(&last_activity);
        let mut shutdown_rx_monitor = shutdown_tx.subscribe();
        let vm_id_for_monitor = vm_id.to_string();
        let data_dir_for_monitor = data_dir;

        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_millis(100));
            loop {
                tokio::select! {
                    _ = shutdown_rx_monitor.recv() => {
                        debug!("Scale-to-zero idle monitor received shutdown signal");
                        break;
                    }
                    _ = interval.tick() => {
                        let is_paused = stats_for_monitor.is_paused.load(Ordering::SeqCst);
                        let active = stats_for_monitor.active_requests.load(Ordering::SeqCst);

                        if !is_paused && active == 0 {
                            let elapsed = {
                                let guard = last_activity_for_monitor.lock().await;
                                guard.elapsed()
                            };

                            if elapsed >= idle_timeout {
                                info!(
                                    "💤 Idle timeout reached ({:.1}s >= {:.1}s) for microVM '{}'. Auto-pausing...",
                                    elapsed.as_secs_f64(),
                                    idle_timeout.as_secs_f64(),
                                    vm_id_for_monitor
                                );

                                match StateManager::pause(&data_dir_for_monitor, &vm_id_for_monitor) {
                                    Ok(_) => {
                                        stats_for_monitor.is_paused.store(true, Ordering::SeqCst);
                                        stats_for_monitor.pause_count.fetch_add(1, Ordering::Relaxed);
                                        info!("💤 MicroVM '{}' auto-paused. Guest CPU usage is 0.0%.", vm_id_for_monitor);
                                    }
                                    Err(e) => {
                                        warn!("Failed to auto-pause microVM '{}': {}", vm_id_for_monitor, e);
                                    }
                                }
                            }
                        }
                    }
                }
            }
        });

        Ok(ServeGatewayHandle {
            host: host.to_string(),
            host_port,
            backend_port,
            vm_id: vm_id.to_string(),
            stats,
            shutdown_tx,
        })
    }

    /// Handles an individual proxy connection: wakes the microVM if paused, then bidirectionally pipes traffic.
    async fn handle_connection(
        mut client: TcpStream,
        backend_port: u16,
        vm_id: &str,
        data_dir: &std::path::Path,
        stats: Arc<GatewayStats>,
        last_activity: Arc<Mutex<Instant>>,
    ) -> Result<()> {
        let _ = client.set_nodelay(true);
        stats.total_requests.fetch_add(1, Ordering::Relaxed);
        stats.active_requests.fetch_add(1, Ordering::SeqCst);
        {
            let mut guard = last_activity.lock().await;
            *guard = Instant::now();
        }

        // Wake on request if paused
        if stats.is_paused.load(Ordering::SeqCst) {
            let wake_start = Instant::now();
            info!(
                "⚡ Incoming HTTP request on paused microVM '{}'. Resuming via SIGCONT...",
                vm_id
            );

            match StateManager::resume(data_dir, vm_id) {
                Ok(_) => {
                    let wake_duration = wake_start.elapsed();
                    let wake_us = wake_duration.as_micros() as u64;
                    stats.is_paused.store(false, Ordering::SeqCst);
                    stats.wake_count.fetch_add(1, Ordering::Relaxed);
                    stats.last_wake_latency_us.store(wake_us, Ordering::Relaxed);
                    info!(
                        "⚡ MicroVM '{}' resumed in {:.2}ms ({:.1}µs)",
                        vm_id,
                        wake_duration.as_secs_f64() * 1000.0,
                        wake_us as f64
                    );
                }
                Err(e) => {
                    stats.active_requests.fetch_sub(1, Ordering::SeqCst);
                    error!("Failed to resume microVM '{}' on request: {}", vm_id, e);
                    let err_resp = b"HTTP/1.1 503 Service Unavailable\r\nConnection: close\r\nContent-Length: 32\r\n\r\nFailed to resume target microVM\n";
                    use tokio::io::AsyncWriteExt;
                    let _ = client.write_all(err_resp).await;
                    return Err(e);
                }
            }
        }

        // Connect to guest inference backend with rapid retry
        let backend_addr = format!("127.0.0.1:{}", backend_port);
        let mut backend_stream = None;
        for attempt in 0..6 {
            match TcpStream::connect(&backend_addr).await {
                Ok(s) => {
                    let _ = s.set_nodelay(true);
                    backend_stream = Some(s);
                    break;
                }
                Err(_) if attempt < 5 => {
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
                Err(e) => {
                    stats.active_requests.fetch_sub(1, Ordering::SeqCst);
                    return Err(anyhow::anyhow!(
                        "Failed to connect to microVM inference backend at {}: {}",
                        backend_addr,
                        e
                    ));
                }
            }
        }

        let mut backend = match backend_stream {
            Some(s) => s,
            None => {
                stats.active_requests.fetch_sub(1, Ordering::SeqCst);
                anyhow::bail!("Unreachable backend");
            }
        };

        // Bidirectional streaming
        let copy_result = tokio::io::copy_bidirectional(&mut client, &mut backend).await;

        stats.active_requests.fetch_sub(1, Ordering::SeqCst);
        {
            let mut guard = last_activity.lock().await;
            *guard = Instant::now();
        }

        copy_result.map(|_| ()).context("Proxy connection closed")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    #[tokio::test]
    async fn test_gateway_find_available_loopback_port() {
        let p1 = find_available_loopback_port().unwrap();
        let p2 = find_available_loopback_port().unwrap();
        assert!(p1 > 1024);
        assert!(p2 > 1024);
    }

    #[tokio::test]
    async fn test_gateway_proxy_and_metrics_flow() {
        // 1. Mock backend echo server
        let backend_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let backend_port = backend_listener.local_addr().unwrap().port();

        tokio::spawn(async move {
            while let Ok((mut stream, _)) = backend_listener.accept().await {
                tokio::spawn(async move {
                    let mut buf = [0u8; 1024];
                    if let Ok(n) = stream.read(&mut buf).await {
                        let resp = format!(
                            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n{}",
                            n,
                            String::from_utf8_lossy(&buf[..n])
                        );
                        let _ = stream.write_all(resp.as_bytes()).await;
                    }
                });
            }
        });

        // 2. Start Gateway
        let gw_port = find_available_loopback_port().unwrap();
        let temp = tempfile::tempdir().unwrap();

        let handle = ServeGateway::start(
            "127.0.0.1",
            gw_port,
            backend_port,
            "test-vm-scale-zero",
            temp.path().to_path_buf(),
            Duration::from_secs(60),
        )
        .await
        .unwrap();

        assert_eq!(handle.host_port, gw_port);
        assert_eq!(handle.backend_port, backend_port);
        assert!(!handle.is_paused());
        assert_eq!(handle.active_requests(), 0);

        // 3. Make client request through Gateway
        let mut client = TcpStream::connect(("127.0.0.1", gw_port)).await.unwrap();
        client.write_all(b"PING").await.unwrap();
        client.shutdown().await.unwrap();

        let mut resp = String::new();
        client.read_to_string(&mut resp).await.unwrap();
        assert!(resp.contains("PING"));

        let stats = handle.stats();
        assert_eq!(stats.total_requests, 1);
        assert_eq!(stats.active_requests, 0);

        handle.shutdown();
    }
}
