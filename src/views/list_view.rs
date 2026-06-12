/* views/list_view.rs
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

//! List-view builder for the file-browser panel.
//!
//! Extracted from `window.rs` to keep the main window module focused on
//! layout orchestration and signal wiring.  All list-view widget
//! construction lives here.
//!
//! # Public API
//!
//! - [`build_list_view`] – creates a [`gtk::ScrolledWindow`] containing a
//!   [`gtk::ListBox`] of [`TreeNode`] rows.
//! - [`build_file_row`]  – builds a single [`gtk::ListBoxRow`] for a node.

use gettextrs::gettext;
use gtk::glib;
use gtk::prelude::*;
use std::path::PathBuf;

use crate::git_engine::TreeNode;
use crate::icon_helpers::{folder_icon_symbolic, mime_icon};

/// Callback type invoked when the user activates a directory entry.
pub type OnEnterDir = Box<dyn Fn(PathBuf) + 'static>;
/// Callback type invoked when the user activates a file entry.
pub type OnOpenFile = Box<dyn Fn(&std::path::Path, &str) + 'static>;

/// Builds a scrollable list view for `children` at the given `hash`.
///
/// `on_enter_dir` is called when the user double-clicks / activates a
/// directory row.  `on_open_file` is called for file rows.
/// Submodule entries reuse `on_enter_dir` — the caller navigates into
/// the submodule path the same way it navigates into a directory.
///
/// Returns a [`gtk::Widget`] (upcast from [`gtk::ScrolledWindow`]) ready
/// to be inserted as the right-panel content.
pub fn build_list_view(
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

    let list = gtk::ListBox::builder()
        .selection_mode(gtk::SelectionMode::Single)
        .build();
    list.add_css_class("boxed-list");

    if children.is_empty() {
        let placeholder = gtk::Label::builder()
            .label(gettext("Empty directory"))
            .margin_top(24)
            .margin_bottom(24)
            .build();
        placeholder.add_css_class("dim-label");
        list.append(&gtk::ListBoxRow::builder().child(&placeholder).build());
    } else {
        for node in children {
            list.append(&build_file_row(node));
        }
    }

    let children_clone = children.to_vec();
    let hash_clone = hash.to_owned();
    // Keep a WeakRef to the list so we can detect when it has been
    // un-parented (panel replaced) before the queued signal fires.
    // If the widget is gone or has no parent the event is stale — bail out.
    let list_weak = list.downgrade();
    list.connect_row_activated(glib::clone!(
        move |_, row| {
            // Guard: if the ListBox has already been un-parented (the panel
            // was swapped while this signal was still in the event queue)
            // then gtk_widget_get_layout_manager would fire a CRITICAL.
            // Upgrading the WeakRef and checking parent() short-circuits
            // that path safely without removing any existing behaviour.
            if list_weak.upgrade().and_then(|w| w.parent()).is_none() {
                return;
            }
            // gtk::ListBoxRow::index() returns -1 when the row is not
            // attached to a ListBox (removal animations, re-render edge
            // cases).  Casting -1i32 as usize wraps to
            // 18_446_744_073_709_551_615 — guard against it here.
            let idx = match row.index() {
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

    scrolled.set_child(Some(&list));
    scrolled.upcast()
}

/// Builds a single [`gtk::ListBoxRow`] for a [`TreeNode`].
///
/// Layout: `[icon] [name label] [chevron / submodule badge / ext badge]`
///
/// | Variant | Icon | Right badge |
/// |---|---|---|
/// | `Dir` | `folder-*-symbolic` | `go-next-symbolic` chevron |
/// | `Submodule` | `folder-remote-symbolic` | `vcs-branch-symbolic` chain badge |
/// | `File` | mime icon | extension label |
pub fn build_file_row(node: &TreeNode) -> gtk::ListBoxRow {
    let hbox = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(10)
        .margin_top(5)
        .margin_bottom(5)
        .margin_start(12)
        .margin_end(12)
        .build();
    hbox.add_css_class("nautilus-list-row");

    let icon_name = match node {
        TreeNode::Dir(p) => {
            folder_icon_symbolic(p.file_name().and_then(|n| n.to_str()).unwrap_or(""))
        }
        TreeNode::File(p) => mime_icon(p),
        // Submodules use the "remote folder" symbolic icon so they are
        // visually distinct from plain directories at a glance.
        TreeNode::Submodule(_) => "folder-remote-symbolic",
    };
    let icon = gtk::Image::from_icon_name(icon_name);
    icon.set_pixel_size(16);
    hbox.append(&icon);

    let name = node
        .path()
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("");
    let label = gtk::Label::builder()
        .label(name)
        .xalign(0.0)
        .hexpand(true)
        .ellipsize(gtk::pango::EllipsizeMode::End)
        .build();
    hbox.append(&label);

    if node.is_dir() {
        let chevron = gtk::Image::from_icon_name("go-next-symbolic");
        chevron.add_css_class("dim-label");
        chevron.set_pixel_size(12);
        hbox.append(&chevron);
    } else if node.is_submodule() {
        // Show a small chain/branch icon to signal "this is a submodule".
        let badge = gtk::Image::from_icon_name("vcs-branch-symbolic");
        badge.add_css_class("dim-label");
        badge.set_pixel_size(12);
        hbox.append(&badge);
    } else if let Some(ext) = node.path().extension().and_then(|e| e.to_str()) {
        let type_label = gtk::Label::builder()
            .label(&ext.to_uppercase())
            .build();
        type_label.add_css_class("caption");
        type_label.add_css_class("dim-label");
        hbox.append(&type_label);
    }

    gtk::ListBoxRow::builder().child(&hbox).build()
}
