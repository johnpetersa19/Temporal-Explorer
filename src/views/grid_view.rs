/* views/grid_view.rs
 *
 * Copyright 2026 John Peter Sá
 *
 * This program is free software: you can redistribute it and/or modify
 * it under the terms of the GNU General Public License as published by
 * the Free Software Foundation, either version 3 of the License, or
 * (at your option) any later version.
 *
 * This program is distributed in the hope that it will be useful,
 * but WITHOUT ANY WARRANTY; without even the implied warranty of
 * MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
 * GNU General Public License for more details.
 *
 * You should have received a copy of the GNU General Public License
 * along with this program.  If not, see <https://www.gnu.org/licenses/>.
 *
 * SPDX-License-Identifier: GPL-3.0-or-later
 */

//! Grid-view builder for the file-browser panel.
//!
//! Extracted from `window.rs` to keep the main window module focused on
//! layout orchestration and signal wiring.  All grid-view widget
//! construction lives here.
//!
//! # Public API
//!
//! - [`build_grid_view`] – creates a [`gtk::ScrolledWindow`] containing a
//!   [`gtk::FlowBox`] of [`TreeNode`] cells.
//! - [`build_grid_cell`] – builds a single icon+label cell box for a node.

use gettextrs::gettext;
use gtk::glib;
use gtk::prelude::*;
use std::path::PathBuf;

use crate::git_engine::TreeNode;
use crate::icon_helpers::{folder_icon, mime_icon_full};

/// Callback type invoked when the user activates a directory cell.
pub type OnEnterDir = Box<dyn Fn(PathBuf) + 'static>;
/// Callback type invoked when the user activates a file cell.
pub type OnOpenFile = Box<dyn Fn(&std::path::Path, &str) + 'static>;

/// Builds a scrollable grid (flow) view for `children` at the given `hash`.
///
/// `on_enter_dir` is called when the user double-clicks / activates a
/// directory cell.  `on_open_file` is called for file cells.
/// Submodule entries reuse `on_enter_dir` — the caller navigates into
/// the submodule path the same way it navigates into a directory.
///
/// Returns a [`gtk::Widget`] (upcast from [`gtk::ScrolledWindow`]) ready
/// to be inserted as the right-panel content.
pub fn build_grid_view(
    children: &[TreeNode],
    hash: &str,
    on_enter_dir: OnEnterDir,
    on_open_file: OnOpenFile,
) -> gtk::Widget {
    let scrolled = gtk::ScrolledWindow::builder()
        .vexpand(true)
        .hexpand(true)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .build();

    let flow = gtk::FlowBox::builder()
        .selection_mode(gtk::SelectionMode::Single)
        .homogeneous(true)
        .column_spacing(6)
        .row_spacing(6)
        .margin_top(16)
        .margin_bottom(16)
        .margin_start(16)
        .margin_end(16)
        .max_children_per_line(64)
        .min_children_per_line(1)
        .build();

    if children.is_empty() {
        let placeholder = gtk::Label::builder()
            .label(gettext("Empty directory"))
            .margin_top(24)
            .margin_bottom(24)
            .build();
        placeholder.add_css_class("dim-label");
        flow.insert(&placeholder, -1);
    } else {
        for node in children {
            let cell = build_grid_cell(node);
            let child = gtk::FlowBoxChild::builder()
                .child(&cell)
                .valign(gtk::Align::Start)
                .halign(gtk::Align::Center)
                .build();
            flow.insert(&child, -1);
        }
    }

    let children_clone = children.to_vec();
    let hash_clone = hash.to_owned();
    // Keep a WeakRef to the flow so we can detect when it has been
    // un-parented (panel replaced) before the queued signal fires.
    // If the widget is gone or has no parent the event is stale — bail out.
    let flow_weak = flow.downgrade();
    flow.connect_child_activated(glib::clone!(
        move |_, child| {
            // Guard: if the FlowBox has already been un-parented (the panel
            // was swapped while this signal was still in the event queue)
            // then gtk_widget_get_layout_manager would fire a CRITICAL.
            // Upgrading the WeakRef and checking parent() short-circuits
            // that path safely without removing any existing behaviour.
            if flow_weak.upgrade().and_then(|w| w.parent()).is_none() {
                return;
            }
            // gtk::FlowBoxChild::index() returns -1 when the child is not
            // attached to a FlowBox (removal animations, re-render edge
            // cases).  Casting -1i32 as usize wraps to
            // 18_446_744_073_709_551_615 — guard against it here.
            let idx = match child.index() {
                i if i >= 0 => i as usize,
                _ => return,
            };
            if let Some(node) = children_clone.get(idx) {
                if node.is_dir() || node.is_submodule() {
                    on_enter_dir(node.path().to_path_buf());
                } else {
                    on_open_file(node.path(), &hash_clone);
                }
            }
        }
    ));

    scrolled.set_child(Some(&flow));
    scrolled.upcast()
}

/// Builds a single icon+label cell [`gtk::Box`] for a [`TreeNode`].
///
/// Layout (vertical):
/// ```text
/// ┌──────────────┐
/// │   [64px icon]│
/// │  [name label]│
/// └──────────────┘
/// ```
///
/// | Variant | Icon |
/// |---|---|
/// | `Dir` | `folder-*` (64 px) |
/// | `Submodule` | `folder-remote` (64 px) |
/// | `File` | full mime icon (64 px) |
pub fn build_grid_cell(node: &TreeNode) -> gtk::Box {
    let vbox = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(6)
        .margin_top(6)
        .margin_bottom(6)
        .margin_start(6)
        .margin_end(6)
        .width_request(96)
        .build();
    vbox.add_css_class("nautilus-view-cell");

    let icon_name = match node {
        TreeNode::Dir(p) => {
            folder_icon(p.file_name().and_then(|n| n.to_str()).unwrap_or(""))
        }
        TreeNode::File(p) => mime_icon_full(p),
        // Submodules use the full-colour "folder-remote" icon at grid size.
        TreeNode::Submodule(_) => "folder-remote",
    };
    let icon = gtk::Image::from_icon_name(icon_name);
    icon.set_pixel_size(64);
    icon.set_halign(gtk::Align::Center);
    vbox.append(&icon);

    let name = node
        .path()
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("");
    let label = gtk::Label::builder()
        .label(name)
        .halign(gtk::Align::Center)
        .justify(gtk::Justification::Center)
        .wrap(true)
        .wrap_mode(gtk::pango::WrapMode::WordChar)
        .max_width_chars(12)
        .lines(3)
        .ellipsize(gtk::pango::EllipsizeMode::End)
        .build();
    label.add_css_class("caption");
    vbox.append(&label);
    vbox
}
