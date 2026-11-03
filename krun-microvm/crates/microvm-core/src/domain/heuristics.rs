// Copyright 2026, libkrun-sdk authors.
// SPDX-License-Identifier: Apache-2.0

//! Port heuristics for automatic local web service routing.

use crate::types::PortForward;

/// Standard HTTP/HTTPS and web development ports, ranked by likelihood.
pub const WEB_PORTS: [u16; 14] = [
    80, 8080, 3000, 8000, 5000, 5173, 4200, 8888, 9000, 8081, 4000, 3001, 15672, 8443,
];

/// Known non-web ports (databases, message brokers, caching stores) to deprioritize/filter.
pub const NOT_WEB: [u16; 12] = [
    5432, 3306, 6379, 27017, 5672, 4369, 25672, 9042, 11211, 1433, 1521, 2181,
];

/// Computes the web ranking priority of a port. Lower rank numbers mean higher priority.
pub fn rank_port(port: u16) -> usize {
    if let Some(pos) = WEB_PORTS.iter().position(|&p| p == port) {
        pos
    } else if NOT_WEB.contains(&port) {
        usize::MAX
    } else {
        WEB_PORTS.len()
    }
}

/// Selects the best candidate host port to forward HTTP traffic to,
/// given a list of `PortForward` rules.
pub fn select_best_forward_host_port(forwards: &[PortForward]) -> Option<u16> {
    if forwards.is_empty() {
        return None;
    }
    if forwards.len() == 1 {
        return Some(forwards[0].host);
    }

    // Rank primarily by guest port suitability, and secondarily by host port suitability
    let mut ranked = forwards.to_vec();
    ranked.sort_by_key(|pf| (rank_port(pf.guest), rank_port(pf.host), pf.host));
    Some(ranked[0].host)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rank_port() {
        assert_eq!(rank_port(80), 0);
        assert_eq!(rank_port(8080), 1);
        assert_eq!(rank_port(3000), 2);
        assert_eq!(rank_port(5432), usize::MAX);
        assert_eq!(rank_port(6379), usize::MAX);
        assert_eq!(rank_port(9999), WEB_PORTS.len());
    }

    #[test]
    fn test_select_best_forward_host_port() {
        let empty: Vec<PortForward> = Vec::new();
        assert_eq!(select_best_forward_host_port(&empty), None);

        let single = vec![PortForward::new(9090, 80)];
        assert_eq!(select_best_forward_host_port(&single), Some(9090));

        let mixed = vec![
            PortForward::new(5432, 5432), // postgres
            PortForward::new(8080, 80),   // web
            PortForward::new(6379, 6379), // redis
        ];
        assert_eq!(select_best_forward_host_port(&mixed), Some(8080));

        let web_priority = vec![PortForward::new(3000, 3000), PortForward::new(8080, 80)];
        assert_eq!(select_best_forward_host_port(&web_priority), Some(8080));
    }
}
