// Copyright 2026, libkrun-sdk authors.
// SPDX-License-Identifier: Apache-2.0

use crate::ui::app::{App, ServeFocus, Tab};
use microvm_core::modelpack::{DiscoveredModel, ModelFormat, ModelSource};
use std::path::PathBuf;

#[test]
fn test_tab_cycling() {
    assert_eq!(Tab::Instances.next(), Tab::Logs);
    assert_eq!(Tab::Logs.next(), Tab::Serve);
    assert_eq!(Tab::Serve.next(), Tab::Approvals);
    assert_eq!(Tab::Approvals.next(), Tab::Instances);

    assert_eq!(Tab::Instances.prev(), Tab::Approvals);
    assert_eq!(Tab::Approvals.prev(), Tab::Serve);
    assert_eq!(Tab::Serve.prev(), Tab::Logs);
    assert_eq!(Tab::Logs.prev(), Tab::Instances);
}

#[test]
fn test_app_tab_navigation() {
    let mut app = App::new(PathBuf::from("/tmp"));
    assert_eq!(app.current_tab, Tab::Instances);

    app.next_tab();
    assert_eq!(app.current_tab, Tab::Logs);

    app.next_tab();
    assert_eq!(app.current_tab, Tab::Serve);

    app.next_tab();
    assert_eq!(app.current_tab, Tab::Approvals);

    app.next_tab();
    assert_eq!(app.current_tab, Tab::Instances);

    app.prev_tab();
    assert_eq!(app.current_tab, Tab::Approvals);

    app.select_tab(Tab::Serve);
    assert_eq!(app.current_tab, Tab::Serve);
}

#[test]
fn test_app_notification_toast() {
    let mut app = App::new(PathBuf::from("/tmp"));
    assert!(app.active_notification().is_none());

    app.set_notification("MicroVM paused");
    assert_eq!(app.active_notification(), Some("MicroVM paused"));
}

#[test]
fn test_app_log_scrolling() {
    let mut app = App::new(PathBuf::from("/tmp"));
    app.log_buffer = (0..100).map(|i| format!("Line {}", i)).collect();
    app.log_scroll_offset = 99;
    app.log_follow = true;

    app.scroll_logs_up(10);
    assert_eq!(app.log_scroll_offset, 89);
    assert!(!app.log_follow);

    app.scroll_logs_down(5);
    assert_eq!(app.log_scroll_offset, 94);

    app.scroll_logs_to_bottom();
    assert_eq!(app.log_scroll_offset, 99);
    assert!(app.log_follow);
}

#[test]
fn test_app_item_navigation_empty() {
    let mut app = App::new(PathBuf::from("/tmp"));
    // Navigating with empty items should not panic
    app.next_item();
    assert_eq!(app.selected_instance, 0);

    app.prev_item();
    assert_eq!(app.selected_instance, 0);
}

#[test]
fn test_app_discovered_models_navigation() {
    let mut app = App::new(PathBuf::from("/tmp"));
    app.select_tab(Tab::Serve);
    assert_eq!(app.serve_focus, ServeFocus::Discovered);

    app.discovered_models = vec![
        DiscoveredModel {
            name: "model-1.gguf".to_string(),
            path: PathBuf::from("/models/m1.gguf"),
            format: ModelFormat::Gguf,
            size_bytes: 1024 * 1024 * 100,
            source: ModelSource::LocalDir,
        },
        DiscoveredModel {
            name: "model-2.safetensors".to_string(),
            path: PathBuf::from("/models/m2.safetensors"),
            format: ModelFormat::Safetensors,
            size_bytes: 1024 * 1024 * 500,
            source: ModelSource::HuggingFace,
        },
    ];

    assert_eq!(app.selected_discovered, 0);
    assert_eq!(
        app.selected_discovered_model().unwrap().name,
        "model-1.gguf"
    );

    // Navigate next
    app.next_item();
    assert_eq!(app.selected_discovered, 1);
    assert_eq!(
        app.selected_discovered_model().unwrap().name,
        "model-2.safetensors"
    );

    // Wrap around
    app.next_item();
    assert_eq!(app.selected_discovered, 0);

    // Prev item
    app.prev_item();
    assert_eq!(app.selected_discovered, 1);

    // Toggle focus
    app.toggle_serve_focus();
    assert_eq!(app.serve_focus, ServeFocus::Active);
    app.toggle_serve_focus();
    assert_eq!(app.serve_focus, ServeFocus::Discovered);
}
