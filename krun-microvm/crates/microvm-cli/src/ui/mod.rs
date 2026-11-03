// Copyright 2026, libkrun-sdk authors.
// SPDX-License-Identifier: Apache-2.0

//! Interactive multi-tab terminal cockpit (`cro ui`) subsystem.

pub mod app;
pub mod events;
#[cfg(test)]
pub mod tests;
pub mod views;

pub use app::{App, ServeFocus, Tab};
use events::{Event, EventHandler};

use anyhow::{Context, Result};
use crossterm::cursor::Show;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use microvm_core::net::ApprovalStore;
use microvm_core::{ServeConfig, ServeManager, StateManager};
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Direction, Layout};
use ratatui::Terminal;
use std::io::Stdout;
use std::path::PathBuf;
use std::time::Duration;

use crate::utils::default_data_dir;

/// Initializes terminal in raw mode with alternate screen and panic hook safety.
pub fn setup_terminal() -> Result<Terminal<CrosstermBackend<Stdout>>> {
    enable_raw_mode().context("Failed to enable raw mode")?;
    let mut stdout = std::io::stdout();
    execute!(stdout, EnterAlternateScreen).context("Failed to enter alternate screen")?;

    // Custom panic hook to restore terminal if an unexpected panic occurs
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = disable_raw_mode();
        let _ = execute!(std::io::stdout(), LeaveAlternateScreen, Show);
        default_hook(info);
    }));

    let backend = CrosstermBackend::new(stdout);
    Terminal::new(backend).context("Failed to initialize ratatui terminal backend")
}

/// Restores normal terminal state and re-enables cursor.
pub fn restore_terminal(terminal: &mut Terminal<CrosstermBackend<Stdout>>) -> Result<()> {
    disable_raw_mode().context("Failed to disable raw mode")?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen, Show)
        .context("Failed to leave alternate screen")?;
    terminal.show_cursor().context("Failed to show cursor")?;
    Ok(())
}

/// Main entry point for the interactive terminal cockpit.
pub async fn run_cockpit(data_dir: Option<PathBuf>) -> Result<()> {
    let base = data_dir.unwrap_or_else(default_data_dir);
    let mut terminal = setup_terminal()?;
    let mut app = App::new(base.clone());
    let mut event_handler = EventHandler::new(Duration::from_millis(250));

    // Initial data fetch
    app.refresh_data();

    let res = run_loop(&mut terminal, &mut app, &mut event_handler).await;

    // Always restore terminal on exit
    let _ = restore_terminal(&mut terminal);
    res
}

/// Interactive event and render loop.
async fn run_loop(
    terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    app: &mut App,
    events: &mut EventHandler,
) -> Result<()> {
    while !app.should_quit {
        terminal.draw(|frame| {
            let area = frame.area();

            // Check minimum terminal bounds
            if area.width < 75 || area.height < 18 {
                let msg = ratatui::widgets::Paragraph::new(
                    "Terminal window is too small for cro cockpit.\nPlease resize your terminal to at least 80x24.",
                )
                .style(ratatui::style::Style::default().fg(ratatui::style::Color::Yellow));
                frame.render_widget(msg, area);
                return;
            }

            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Length(4), // Header + Tabs
                    Constraint::Min(10),   // Active Tab View
                    Constraint::Length(1), // Footer Shortcuts
                ])
                .split(area);

            views::header::render_header(frame, chunks[0], app);

            match app.current_tab {
                Tab::Instances => views::instances::render_instances(frame, chunks[1], app),
                Tab::Logs => views::logs::render_logs(frame, chunks[1], app),
                Tab::Serve => views::serve::render_serve(frame, chunks[1], app),
                Tab::Approvals => views::approvals::render_approvals(frame, chunks[1], app),
            }

            views::footer::render_footer(frame, chunks[2], app);
        })?;

        if let Some(event) = events.next().await {
            match event {
                Event::Tick => {
                    app.refresh_data();
                }
                Event::Key(key) => {
                    handle_key(app, key).await;
                }
                Event::Resize(_, _) => {}
            }
        }
    }

    Ok(())
}

/// Dispatches keyboard interactions based on the active tab and key pressed.
async fn handle_key(app: &mut App, key: KeyEvent) {
    // Global navigation shortcuts
    match (key.code, key.modifiers) {
        (KeyCode::Char('q'), _) | (KeyCode::Char('c'), KeyModifiers::CONTROL) => {
            app.should_quit = true;
            return;
        }
        (KeyCode::Tab, _) => {
            app.next_tab();
            return;
        }
        (KeyCode::BackTab, _) => {
            app.prev_tab();
            return;
        }
        (KeyCode::Char('1'), _) => {
            app.select_tab(Tab::Instances);
            return;
        }
        (KeyCode::Char('2'), _) => {
            app.select_tab(Tab::Logs);
            return;
        }
        (KeyCode::Char('3'), _) => {
            app.select_tab(Tab::Serve);
            return;
        }
        (KeyCode::Char('4'), _) => {
            app.select_tab(Tab::Approvals);
            return;
        }
        (KeyCode::Up, _) | (KeyCode::Char('k'), _) => {
            app.prev_item();
            return;
        }
        (KeyCode::Down, _) | (KeyCode::Char('j'), _) => {
            app.next_item();
            return;
        }
        _ => {}
    }

    // Contextual shortcuts
    match app.current_tab {
        Tab::Instances => match key.code {
            KeyCode::Enter => {
                if let Some(vm) = app.selected_vm() {
                    app.log_vm_id = Some(vm.state.id.clone());
                    app.log_buffer.clear();
                    app.last_log_size = 0;
                    app.log_follow = true;
                    app.select_tab(Tab::Logs);
                }
            }
            KeyCode::Char('p') => {
                if let Some(vm) = app.selected_vm() {
                    let id = vm.state.id.clone();
                    match StateManager::pause(&app.data_dir, &id) {
                        Ok(_) => app.set_notification(format!("MicroVM '{}' paused", id)),
                        Err(e) => app.set_notification(format!("Failed to pause: {}", e)),
                    }
                    app.refresh_data();
                }
            }
            KeyCode::Char('r') => {
                if let Some(vm) = app.selected_vm() {
                    let id = vm.state.id.clone();
                    match StateManager::resume(&app.data_dir, &id) {
                        Ok(_) => app.set_notification(format!("MicroVM '{}' resumed", id)),
                        Err(e) => app.set_notification(format!("Failed to resume: {}", e)),
                    }
                    app.refresh_data();
                }
            }
            KeyCode::Char('s') => {
                if let Some(vm) = app.selected_vm() {
                    let id = vm.state.id.clone();
                    let pid = vm.state.pid;
                    if vm.is_alive && pid > 0 {
                        let _ = nix::sys::signal::kill(
                            nix::unistd::Pid::from_raw(pid as i32),
                            nix::sys::signal::Signal::SIGTERM,
                        );
                        app.set_notification(format!(
                            "Sent SIGTERM to microVM '{}' (PID {})",
                            id, pid
                        ));
                    } else {
                        app.set_notification(format!("MicroVM '{}' is already stopped", id));
                    }
                    app.refresh_data();
                }
            }
            KeyCode::Char('x') => {
                if let Some(vm) = app.selected_vm() {
                    let id = vm.state.id.clone();
                    let pid = vm.state.pid;
                    if vm.is_alive && pid > 0 {
                        let _ = nix::sys::signal::kill(
                            nix::unistd::Pid::from_raw(pid as i32),
                            nix::sys::signal::Signal::SIGKILL,
                        );
                        app.set_notification(format!("Killed microVM '{}' (PID {})", id, pid));
                    }
                    app.refresh_data();
                }
            }
            KeyCode::Char('d') => {
                if let Some(vm) = app.selected_vm() {
                    let id = vm.state.id.clone();
                    if !vm.is_alive {
                        match StateManager::remove(&app.data_dir, &id) {
                            Ok(_) => app.set_notification(format!("Deleted microVM '{}'", id)),
                            Err(e) => app.set_notification(format!("Failed to delete: {}", e)),
                        }
                    } else {
                        app.set_notification(format!("Stop microVM '{}' before deleting", id));
                    }
                    app.refresh_data();
                }
            }
            _ => {}
        },
        Tab::Logs => match key.code {
            KeyCode::Char(' ') => {
                app.toggle_log_follow();
            }
            KeyCode::PageUp => {
                app.scroll_logs_up(15);
            }
            KeyCode::PageDown => {
                app.scroll_logs_down(15);
            }
            _ => {}
        },
        Tab::Serve => match key.code {
            KeyCode::Char('w') | KeyCode::Left | KeyCode::Right => {
                app.toggle_serve_focus();
            }
            KeyCode::Char('d') => {
                app.refresh_discovered_models();
                app.set_notification(format!(
                    "🔍 Discovered {} models across local repositories",
                    app.discovered_models.len()
                ));
            }
            KeyCode::Enter | KeyCode::Char('s') => match app.serve_focus {
                ServeFocus::Active => {
                    if let Some(s) = app.serve_items.get(app.selected_serve) {
                        app.set_notification(format!("Inference Endpoint: {}", s.base_url));
                    }
                }
                ServeFocus::Discovered => {
                    if let Some(m) = app.selected_discovered_model().cloned() {
                        let used_ports: std::collections::HashSet<u16> =
                            app.serve_items.iter().map(|s| s.host_port).collect();
                        let port = (8000..8100)
                            .find(|p| !used_ports.contains(p))
                            .unwrap_or(8000);

                        let mut cfg = ServeConfig::new(m.path.to_string_lossy().to_string());
                        cfg.data_dir = Some(app.data_dir.clone());
                        cfg.host_port = port;

                        match ServeManager::start(&cfg, true, false).await {
                            Ok(_) => {
                                app.set_notification(format!(
                                    "🚀 Launched serving microVM for '{}' on port {}",
                                    m.name, port
                                ));
                                app.refresh_data();
                            }
                            Err(e) => {
                                app.set_notification(format!("❌ Failed to serve model: {}", e));
                            }
                        }
                    }
                }
            },
            _ => {}
        },
        Tab::Approvals => match key.code {
            KeyCode::Char('y') => {
                if let Some(req) = app.selected_approval_req() {
                    let req_id = req.id.clone();
                    let approval_dir = app.data_dir.join("approvals");
                    match ApprovalStore::cli_decide(&approval_dir, &req_id, true) {
                        Ok(_) => app.set_notification(format!("Approved request '{}'", req_id)),
                        Err(e) => app.set_notification(format!("Failed to approve: {}", e)),
                    }
                    app.refresh_data();
                }
            }
            KeyCode::Char('n') => {
                if let Some(req) = app.selected_approval_req() {
                    let req_id = req.id.clone();
                    let approval_dir = app.data_dir.join("approvals");
                    match ApprovalStore::cli_decide(&approval_dir, &req_id, false) {
                        Ok(_) => app.set_notification(format!("Rejected request '{}'", req_id)),
                        Err(e) => app.set_notification(format!("Failed to reject: {}", e)),
                    }
                    app.refresh_data();
                }
            }
            _ => {}
        },
    }
}
