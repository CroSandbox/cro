// Copyright 2026, libkrun-sdk authors.
// SPDX-License-Identifier: Apache-2.0

use crate::ui::app::{App, Tab};
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
