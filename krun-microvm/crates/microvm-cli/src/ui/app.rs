// Copyright 2026, libkrun-sdk authors.
// SPDX-License-Identifier: Apache-2.0

//! Centralized state model and navigation logic for the interactive terminal cockpit.

use microvm_core::config::KrunConfig;
use microvm_core::modelpack::{DiscoveredModel, ModelDiscovery};
use microvm_core::net::PendingApproval;
use microvm_core::{collect_process_stats, ProcessStats, StateManager, VmState, VmStatus};
use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::time::{Duration, Instant};

/// Active dashboard tab.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Instances,
    Logs,
    Serve,
    Approvals,
}

impl Tab {
    pub fn all() -> &'static [Tab] {
        &[Tab::Instances, Tab::Logs, Tab::Serve, Tab::Approvals]
    }

    pub fn title(&self) -> &'static str {
        match self {
            Tab::Instances => "1: Instances",
            Tab::Logs => "2: Live Logs",
            Tab::Serve => "3: AI Serving",
            Tab::Approvals => "4: HITL Approvals",
        }
    }

    pub fn next(&self) -> Self {
        match self {
            Tab::Instances => Tab::Logs,
            Tab::Logs => Tab::Serve,
            Tab::Serve => Tab::Approvals,
            Tab::Approvals => Tab::Instances,
        }
    }

    pub fn prev(&self) -> Self {
        match self {
            Tab::Instances => Tab::Approvals,
            Tab::Logs => Tab::Instances,
            Tab::Serve => Tab::Logs,
            Tab::Approvals => Tab::Serve,
        }
    }
}

/// Display summary item for an active or stopped microVM.
#[derive(Debug, Clone)]
pub struct VmDisplayItem {
    pub state: VmState,
    pub cpu_pct: f64,
    pub rss_bytes: u64,
    pub is_alive: bool,
    pub status_text: String,
}

/// Display item for an active AI model serving engine.
#[derive(Debug, Clone)]
pub struct ServeDisplayItem {
    pub vm_id: String,
    pub model_name: String,
    pub engine: String,
    pub host_port: u16,
    pub base_url: String,
    pub status: String,
    pub memory_mb: u32,
    pub vcpus: u8,
}

/// Focused sub-pane within the AI Serving tab.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServeFocus {
    Active,
    Discovered,
}

/// Root state holding all cockpit views, telemetry, and input selections.
pub struct App {
    pub current_tab: Tab,
    pub data_dir: PathBuf,
    pub instances: Vec<VmDisplayItem>,
    pub selected_instance: usize,
    pub cpu_history: HashMap<String, VecDeque<u64>>,
    pub prev_cpu_samples: HashMap<u32, (Instant, ProcessStats)>,
    pub log_buffer: Vec<String>,
    pub log_scroll_offset: usize,
    pub log_follow: bool,
    pub log_vm_id: Option<String>,
    pub last_log_size: u64,
    pub serve_items: Vec<ServeDisplayItem>,
    pub selected_serve: usize,
    pub discovered_models: Vec<DiscoveredModel>,
    pub selected_discovered: usize,
    pub serve_focus: ServeFocus,
    pub approvals: Vec<PendingApproval>,
    pub selected_approval: usize,
    pub notification: Option<(String, Instant)>,
    pub should_quit: bool,
    pub tick_count: u64,
}

impl App {
    pub fn new(data_dir: PathBuf) -> Self {
        Self {
            current_tab: Tab::Instances,
            data_dir,
            instances: Vec::new(),
            selected_instance: 0,
            cpu_history: HashMap::new(),
            prev_cpu_samples: HashMap::new(),
            log_buffer: Vec::new(),
            log_scroll_offset: 0,
            log_follow: true,
            log_vm_id: None,
            last_log_size: 0,
            serve_items: Vec::new(),
            selected_serve: 0,
            discovered_models: Vec::new(),
            selected_discovered: 0,
            serve_focus: ServeFocus::Discovered,
            approvals: Vec::new(),
            selected_approval: 0,
            notification: None,
            should_quit: false,
            tick_count: 0,
        }
    }

    /// Cycles forward to the next tab.
    pub fn next_tab(&mut self) {
        self.current_tab = self.current_tab.next();
    }

    /// Cycles backward to the previous tab.
    pub fn prev_tab(&mut self) {
        self.current_tab = self.current_tab.prev();
    }

    /// Selects a specific tab.
    pub fn select_tab(&mut self, tab: Tab) {
        self.current_tab = tab;
    }

    /// Navigates to the next item in the active view's list.
    pub fn next_item(&mut self) {
        match self.current_tab {
            Tab::Instances => {
                if !self.instances.is_empty() {
                    self.selected_instance = (self.selected_instance + 1) % self.instances.len();
                }
            }
            Tab::Serve => match self.serve_focus {
                ServeFocus::Active => {
                    if !self.serve_items.is_empty() {
                        self.selected_serve = (self.selected_serve + 1) % self.serve_items.len();
                    }
                }
                ServeFocus::Discovered => {
                    if !self.discovered_models.is_empty() {
                        self.selected_discovered =
                            (self.selected_discovered + 1) % self.discovered_models.len();
                    }
                }
            },
            Tab::Approvals => {
                if !self.approvals.is_empty() {
                    self.selected_approval = (self.selected_approval + 1) % self.approvals.len();
                }
            }
            Tab::Logs => {
                self.scroll_logs_down(1);
            }
        }
    }

    /// Navigates to the previous item in the active view's list.
    pub fn prev_item(&mut self) {
        match self.current_tab {
            Tab::Instances => {
                if !self.instances.is_empty() {
                    if self.selected_instance == 0 {
                        self.selected_instance = self.instances.len() - 1;
                    } else {
                        self.selected_instance -= 1;
                    }
                }
            }
            Tab::Serve => match self.serve_focus {
                ServeFocus::Active => {
                    if !self.serve_items.is_empty() {
                        if self.selected_serve == 0 {
                            self.selected_serve = self.serve_items.len() - 1;
                        } else {
                            self.selected_serve -= 1;
                        }
                    }
                }
                ServeFocus::Discovered => {
                    if !self.discovered_models.is_empty() {
                        if self.selected_discovered == 0 {
                            self.selected_discovered = self.discovered_models.len() - 1;
                        } else {
                            self.selected_discovered -= 1;
                        }
                    }
                }
            },
            Tab::Approvals => {
                if !self.approvals.is_empty() {
                    if self.selected_approval == 0 {
                        self.selected_approval = self.approvals.len() - 1;
                    } else {
                        self.selected_approval -= 1;
                    }
                }
            }
            Tab::Logs => {
                self.scroll_logs_up(1);
            }
        }
    }

    /// Displays a temporary notification toast for 4 seconds.
    pub fn set_notification(&mut self, message: impl Into<String>) {
        self.notification = Some((message.into(), Instant::now()));
    }

    /// Returns the active notification if it has not expired.
    pub fn active_notification(&self) -> Option<&str> {
        if let Some((msg, time)) = &self.notification {
            if time.elapsed() < Duration::from_secs(4) {
                return Some(msg.as_str());
            }
        }
        None
    }

    /// Returns the currently selected microVM state, if any.
    pub fn selected_vm(&self) -> Option<&VmDisplayItem> {
        self.instances.get(self.selected_instance)
    }

    /// Returns the currently selected approval request, if any.
    pub fn selected_approval_req(&self) -> Option<&PendingApproval> {
        self.approvals.get(self.selected_approval)
    }

    /// Returns the currently selected discovered model, if any.
    pub fn selected_discovered_model(&self) -> Option<&DiscoveredModel> {
        self.discovered_models.get(self.selected_discovered)
    }

    /// Toggles focus between active serving engines and discovered models.
    pub fn toggle_serve_focus(&mut self) {
        self.serve_focus = match self.serve_focus {
            ServeFocus::Active => ServeFocus::Discovered,
            ServeFocus::Discovered => ServeFocus::Active,
        };
    }

    /// Discovers host models across Hugging Face, LM Studio, Ollama, and local directories.
    pub fn refresh_discovered_models(&mut self) {
        let models = ModelDiscovery::discover(&self.data_dir, &[]);
        self.discovered_models = models;
        if !self.discovered_models.is_empty()
            && self.selected_discovered >= self.discovered_models.len()
        {
            self.selected_discovered = self.discovered_models.len() - 1;
        }
    }

    /// Toggles auto-scroll follow mode in live logs.
    pub fn toggle_log_follow(&mut self) {
        self.log_follow = !self.log_follow;
        if self.log_follow {
            self.scroll_logs_to_bottom();
        }
    }

    /// Scrolls the log viewer up.
    pub fn scroll_logs_up(&mut self, lines: usize) {
        self.log_follow = false;
        self.log_scroll_offset = self.log_scroll_offset.saturating_sub(lines);
    }

    /// Scrolls the log viewer down.
    pub fn scroll_logs_down(&mut self, lines: usize) {
        let max_offset = self.log_buffer.len().saturating_sub(1);
        self.log_scroll_offset = (self.log_scroll_offset + lines).min(max_offset);
    }

    /// Scrolls logs to the bottom and re-enables follow.
    pub fn scroll_logs_to_bottom(&mut self) {
        self.log_scroll_offset = self.log_buffer.len().saturating_sub(1);
        self.log_follow = true;
    }

    /// Refreshes telemetry, process stats, approvals, and logs from disk.
    pub fn refresh_data(&mut self) {
        self.tick_count += 1;

        // 1. Refresh MicroVM Instances
        if let Ok(vms) = StateManager::list(&self.data_dir) {
            let mut items = Vec::with_capacity(vms.len());
            let now = Instant::now();

            for vm in vms {
                let is_alive = vm.is_process_alive();
                let mut cpu_pct = 0.0;
                let mut rss = 0u64;

                if is_alive {
                    if let Some(s2) = collect_process_stats(vm.pid) {
                        rss = s2.memory_rss_bytes;
                        if let Some((t1, s1)) = self.prev_cpu_samples.get(&vm.pid) {
                            let wall_ns = now.duration_since(*t1).as_nanos() as f64;
                            let cpu_ns = s2.total_cpu_ns.saturating_sub(s1.total_cpu_ns) as f64;
                            if wall_ns > 0.0 {
                                cpu_pct = (cpu_ns / wall_ns) * 100.0;
                            }
                        }
                        self.prev_cpu_samples.insert(vm.pid, (now, s2));
                    }
                }

                // Record history buffer for sparkline (capped at 60 points)
                let history = self
                    .cpu_history
                    .entry(vm.id.clone())
                    .or_insert_with(|| VecDeque::with_capacity(60));
                history.push_back(cpu_pct.round() as u64);
                if history.len() > 60 {
                    history.pop_front();
                }

                let status_text = match vm.status {
                    VmStatus::Running if is_alive => "RUNNING".to_string(),
                    VmStatus::Paused => "PAUSED".to_string(),
                    _ => "STOPPED".to_string(),
                };

                items.push(VmDisplayItem {
                    state: vm,
                    cpu_pct,
                    rss_bytes: rss,
                    is_alive,
                    status_text,
                });
            }

            // Sort: Running first, then Paused, then Stopped
            items.sort_by_key(|i| match i.status_text.as_str() {
                "RUNNING" => 0,
                "PAUSED" => 1,
                _ => 2,
            });

            self.instances = items;
            if !self.instances.is_empty() && self.selected_instance >= self.instances.len() {
                self.selected_instance = self.instances.len() - 1;
            }
        }

        // 2. Discover AI Serving Instances
        self.refresh_serving_items();

        // 3. Discover Host Models periodically (on tick 1 and every 30 ticks ~ 15s)
        if self.tick_count == 1 || self.tick_count.is_multiple_of(30) {
            self.refresh_discovered_models();
        }

        // 4. Refresh Approvals
        let approval_dir = self.data_dir.join("approvals");
        if let Ok(pending) = microvm_core::net::ApprovalStore::cli_list_pending(&approval_dir) {
            self.approvals = pending;
            if !self.approvals.is_empty() && self.selected_approval >= self.approvals.len() {
                self.selected_approval = self.approvals.len() - 1;
            }
        }

        // 4. Refresh Log Stream
        self.refresh_logs();
    }

    /// Discovers active model serving instances from running VM configurations.
    fn refresh_serving_items(&mut self) {
        let mut serves = Vec::new();

        for item in &self.instances {
            // Check both rootfs/.krun_config.json and instance_dir/.krun_config.json
            let krun_candidates = [
                item.state
                    .instance_dir
                    .join("rootfs")
                    .join(KrunConfig::FILE_NAME),
                item.state.instance_dir.join(KrunConfig::FILE_NAME),
            ];

            let mut cmd_vec = Vec::new();
            for path in &krun_candidates {
                if let Ok(data) = std::fs::read_to_string(path) {
                    if let Ok(kcfg) = serde_json::from_str::<KrunConfig>(&data) {
                        cmd_vec = kcfg.cmd;
                        break;
                    }
                }
            }

            let cmd_str = cmd_vec.join(" ");
            let is_vllm = cmd_str.contains("vllm");
            let is_llama = cmd_str.contains("llama-server") || cmd_str.contains("llamacpp");
            let is_uzu = cmd_str.contains("uzu");

            if is_vllm || is_llama || is_uzu {
                let engine = if is_vllm {
                    "vLLM"
                } else if is_llama {
                    "llama.cpp"
                } else {
                    "Uzu"
                };

                let port = item
                    .state
                    .port_forwards
                    .first()
                    .map(|p| p.host)
                    .unwrap_or(8000);
                let model = cmd_vec
                    .iter()
                    .position(|c| c == "-m" || c == "--model")
                    .and_then(|idx| cmd_vec.get(idx + 1))
                    .cloned()
                    .unwrap_or_else(|| item.state.image.clone());

                serves.push(ServeDisplayItem {
                    vm_id: item.state.id.clone(),
                    model_name: model,
                    engine: engine.to_string(),
                    host_port: port,
                    base_url: format!("http://127.0.0.1:{}/v1", port),
                    status: item.status_text.clone(),
                    memory_mb: item.state.memory_mib.unwrap_or(1024),
                    vcpus: item.state.vcpus.unwrap_or(1),
                });
            }
        }

        self.serve_items = serves;
        if !self.serve_items.is_empty() && self.selected_serve >= self.serve_items.len() {
            self.selected_serve = self.serve_items.len() - 1;
        }
    }

    /// Tailed read from the focused microVM's console.log file.
    fn refresh_logs(&mut self) {
        let active_vm_id = self
            .instances
            .get(self.selected_instance)
            .map(|i| i.state.id.clone());

        if active_vm_id != self.log_vm_id {
            self.log_vm_id = active_vm_id.clone();
            self.log_buffer.clear();
            self.last_log_size = 0;
            self.log_scroll_offset = 0;
        }

        if let Some(ref vm_id) = active_vm_id {
            let log_path = self
                .data_dir
                .join("instances")
                .join(vm_id)
                .join("console.log");
            if log_path.exists() {
                if let Ok(meta) = std::fs::metadata(&log_path) {
                    if meta.len() != self.last_log_size {
                        self.last_log_size = meta.len();
                        if let Ok(content) = std::fs::read_to_string(&log_path) {
                            let lines: Vec<String> =
                                content.lines().map(|l| l.to_string()).collect();
                            // Keep last 1000 lines
                            let start = lines.len().saturating_sub(1000);
                            self.log_buffer = lines[start..].to_vec();

                            if self.log_follow {
                                self.log_scroll_offset = self.log_buffer.len().saturating_sub(1);
                            }
                        }
                    }
                }
            }
        }
    }
}
