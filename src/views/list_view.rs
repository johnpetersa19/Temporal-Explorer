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
    list.connect_row_activated(glib::clone!(
        move |_, row| {
            let idx = row.index() as usize;
            if let Some(node) = children_clone.get(idx) {
                if node.is_dir() {
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
/// Layout: `[icon] [name label] [chevron or ext badge]`
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
