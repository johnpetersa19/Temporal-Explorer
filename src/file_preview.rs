/* file_preview.rs
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

//! File preview controller.
//!
//! Connects the [`crate::git_engine::SnapshotMaterializer::read_file`]
//! capability to the UI by showing a modal dialog with the raw text
//! content of a file at a specific Git revision.
//!
//! Binary files are detected heuristically (null-byte scan) and an
//! informative message is shown instead of garbled text.

use gettextrs::gettext;
use gtk::prelude::*;
use gtk::glib;
use std::path::Path;
use crate::git_engine::SnapshotMaterializer;

/// Maximum number of bytes read for preview (64 KiB).
const MAX_PREVIEW_BYTES: usize = 64 * 1024;

/// Shows a modal dialog previewing the content of `file_path` at `revision`.
pub fn show_file_preview(
    parent: &impl IsA<gtk::Window>,
    repo: &git2::Repository,
    revision: &str,
    file_path: &Path,
) {
    let materializer = SnapshotMaterializer::new(repo);

    let (title, body_text) = match materializer.read_file(revision, file_path) {
        Err(e) => (
            file_path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("File")
                .to_owned(),
            format!("{}: {e}", gettext("Could not read file")),
        ),
        Ok(bytes) => {
            let name = file_path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("File")
                .to_owned();

            let preview_bytes = &bytes[..bytes.len().min(MAX_PREVIEW_BYTES)];

            if preview_bytes.contains(&0u8) {
                (name, gettext("Binary file -- preview not available."))
            } else {
                let mut text = String::from_utf8_lossy(preview_bytes).into_owned();
                if bytes.len() > MAX_PREVIEW_BYTES {
                    text.push_str(&format!(
                        "\n\n[{} {}]",
                        gettext("Preview truncated at"),
                        format_size(MAX_PREVIEW_BYTES),
                    ));
                }
                (name, text)
            }
        }
    };

    build_preview_dialog(parent, &title, &body_text, revision, file_path);
}

// ── Private helpers ───────────────────────────────────────────────────────

fn build_preview_dialog(
    parent: &impl IsA<gtk::Window>,
    title: &str,
    body_text: &str,
    revision: &str,
    file_path: &Path,
) {
    // 1. Build all leaf widgets first (no parents yet)
    let text_view = gtk::TextView::builder()
        .editable(false)
        .cursor_visible(false)
        .monospace(true)
        .wrap_mode(gtk::WrapMode::None)
        .left_margin(12)
        .right_margin(12)
        .top_margin(12)
        .bottom_margin(12)
        .build();
    text_view.buffer().set_text(body_text);

    let scrolled = gtk::ScrolledWindow::builder()
        .vexpand(true)
        .hexpand(true)
        .child(&text_view)
        .build();

    let copy_btn = gtk::Button::builder()
        .icon_name("edit-copy-symbolic")
        .tooltip_text(gettext("Copy content"))
        .build();
    copy_btn.add_css_class("flat");
    let text_clone = body_text.to_owned();
    copy_btn.connect_clicked(move |btn| {
        if let Some(display) = gtk::gdk::Display::default() {
            display.clipboard().set_text(&text_clone);
            btn.set_icon_name("object-select-symbolic");
            let btn_weak = btn.downgrade();
            glib::timeout_add_seconds_local(2, move || {
                if let Some(b) = btn_weak.upgrade() {
                    b.set_icon_name("edit-copy-symbolic");
                }
                glib::ControlFlow::Break
            });
        }
    });

    let win_title = adw::WindowTitle::builder()
        .title(title)
        .subtitle(&format!("{}\u2026", &revision[..revision.len().min(12)]))
        .build();

    let header = adw::HeaderBar::new();
    header.set_title_widget(Some(&win_title));
    header.pack_end(&copy_btn);

    let path_label = gtk::Label::builder()
        .label(file_path.to_string_lossy().as_ref())
        .selectable(true)
        .ellipsize(gtk::pango::EllipsizeMode::Start)
        .build();
    path_label.add_css_class("caption");
    path_label.add_css_class("dim-label");

    let action_bar = gtk::ActionBar::new();
    action_bar.pack_start(&path_label);

    // 2. Assemble ToolbarView — content set via builder so `scrolled` has no
    //    parent yet when passed in, avoiding the gtk_widget_get_parent assertion.
    let toolbar_view = adw::ToolbarView::builder()
        .content(&scrolled)
        .build();
    toolbar_view.add_top_bar(&header);
    toolbar_view.add_bottom_bar(&action_bar);

    // 3. Set the fully-assembled toolbar_view as the dialog content
    let dialog = adw::Window::builder()
        .title(title)
        .transient_for(parent)
        .modal(true)
        .default_width(760)
        .default_height(540)
        .content(&toolbar_view)
        .build();

    dialog.present();
}

fn format_size(bytes: usize) -> String {
    if bytes >= 1024 * 1024 {
        format!("{:.1} MiB", bytes as f64 / (1024.0 * 1024.0))
    } else {
        format!("{} KiB", bytes / 1024)
    }
}
