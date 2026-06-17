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

use crate::file_grid_captions_dialog::CaptionFlags;
use crate::git_engine::TreeNode;
use crate::icon_helpers::{folder_icon, mime_icon_full};

/// Callback type invoked when the user activates a directory cell.
pub type OnEnterDir = Box<dyn Fn(PathBuf) + 'static>;
/// Callback type invoked when the user activates a file cell.
pub type OnOpenFile = Box<dyn Fn(&std::path::Path, &str) + 'static>;

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub enum GridZoom {
    Small,
    #[default]
    Normal,
    Large,
}

impl GridZoom {
    fn metrics(self) -> GridMetrics {
        match self {
            GridZoom::Small => GridMetrics {
                icon_size: 64,
                cell_width: 112,
                label_width_chars: 14,
            },
            GridZoom::Normal => GridMetrics {
                icon_size: 80,
                cell_width: 132,
                label_width_chars: 16,
            },
            GridZoom::Large => GridMetrics {
                icon_size: 96,
                cell_width: 156,
                label_width_chars: 18,
            },
        }
    }
}


#[derive(Debug, Clone, Copy)]
struct GridMetrics {
    icon_size: i32,
    cell_width: i32,
    label_width_chars: i32,
}

pub fn build_grid_view(
    children: &[TreeNode],
    hash: &str,
    zoom: GridZoom,
    caption_flags: CaptionFlags,
    on_enter_dir: OnEnterDir,
    on_open_file: OnOpenFile,
) -> gtk::Widget {
    let metrics = zoom.metrics();
    let scrolled = gtk::ScrolledWindow::builder()
        .vexpand(true)
        .hexpand(true)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .build();

    let flow = gtk::FlowBox::builder()
        .selection_mode(gtk::SelectionMode::Single)
        // Nautilus-like compact grid: keep the whole flow aligned to the start.
        // The previous version changed only the children, but the FlowBox itself
        // was still expanding to the full viewport width, leaving huge gaps.
        .halign(gtk::Align::Start)
        .valign(gtk::Align::Start)
        .hexpand(false)
        .vexpand(false)
        .homogeneous(false)
        .column_spacing(12)
        .row_spacing(12)
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
            let cell = build_grid_cell(node, metrics, caption_flags);
            let child = gtk::FlowBoxChild::builder()
                .child(&cell)
                .valign(gtk::Align::Start)
                .halign(gtk::Align::Start)
                .hexpand(false)
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
fn build_grid_cell(node: &TreeNode, metrics: GridMetrics, caption_flags: CaptionFlags) -> gtk::Box {
    let vbox = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(6)
        .margin_top(6)
        .margin_bottom(6)
        .margin_start(6)
        .margin_end(6)
        .width_request(metrics.cell_width)
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
    icon.set_pixel_size(metrics.icon_size);
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
        .max_width_chars(metrics.label_width_chars)
        .lines(3)
        // Nautilus uses middle ellipsize for long filenames.
        .ellipsize(gtk::pango::EllipsizeMode::Middle)
        .build();
    label.add_css_class("caption");
    vbox.append(&label);

    for caption in build_caption_lines(node, caption_flags) {
        let caption_label = gtk::Label::builder()
            .label(&caption)
            .halign(gtk::Align::Center)
            .justify(gtk::Justification::Center)
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .max_width_chars(metrics.label_width_chars)
            .build();
        caption_label.add_css_class("caption");
        caption_label.add_css_class("dim-label");
        vbox.append(&caption_label);
    }

    vbox
}

fn build_caption_lines(node: &TreeNode, flags: CaptionFlags) -> Vec<String> {
    if flags.is_empty() {
        return Vec::new();
    }

    let mut lines = Vec::new();
    let path = node.path();

    if flags.contains(CaptionFlags::STATUS) {
        let kind = match node {
            TreeNode::Dir(_) => gettext("Folder"),
            TreeNode::File(_) => gettext("File"),
            TreeNode::Submodule(_) => gettext("Submodule"),
        };
        lines.push(kind);
    }

    if flags.contains(CaptionFlags::EXTENSION) {
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .filter(|e| !e.is_empty())
            .map(|e| format!(".{e}"))
            .unwrap_or_else(|| {
                if node.is_dir() {
                    gettext("Directory")
                } else {
                    gettext("No extension")
                }
            });

        lines.push(ext);
    }

    if flags.contains(CaptionFlags::SIZE) {
        // Snapshot tree nodes currently do not carry blob size.
        // Keep the caption visible so the option has immediate UI feedback.
        lines.push(gettext("Size unavailable"));
    }

    if flags.contains(CaptionFlags::DATE) {
        // Per-file modification time is not stored in a Git tree object.
        // The real timestamp belongs to the selected commit/snapshot.
        lines.push(gettext("Snapshot date"));
    }

    lines
}
