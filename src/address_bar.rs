/* address_bar.rs
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

//! Address-bar / path-bar helpers.
//!
//! Extracted from `window.rs` to keep the main window module focused on
//! orchestration.  This module owns the widget-construction and signal
//! wiring for the Nautilus-style breadcrumb path bar.
//!
//! # Public API
//!
//! - [`rebuild_address_bar`] – clears and re-populates the path-bar [`gtk::Box`]
//!   with pill-shaped segment buttons.
//! - [`switch_to_location_entry`] – updates the text entry with the current path
//!   and switches the toolbar stack to the `"location"` page.

use gtk::prelude::*;
use std::path::PathBuf;

/// Removes all children from a `gtk::Box` safely.
///
/// Snapshots the child list first, then calls `unparent()` on each captured
/// widget. This prevents iterator-invalidation races where a concurrent GTK
/// idle frame observes a partially-mutated sibling chain, causing
/// `gtk_widget_insert_after` assertion failures.
fn clear_box(container: &gtk::Box) {
    let mut children: Vec<gtk::Widget> = Vec::new();
    let mut child = container.first_child();
    while let Some(w) = child {
        child = w.next_sibling();
        children.push(w);
    }
    for w in children {
        w.unparent();
    }
}

/// Clears `bar` and rebuilds the breadcrumb segment buttons for `dir`.
///
/// `repo_name` is shown as the root segment.  Each path component of `dir`
/// becomes a clickable segment that calls `on_segment_clicked` with its
/// full accumulated path.  The last (current) segment calls
/// `on_current_clicked` instead (which typically opens the location entry).
///
/// # Example layout
/// ```text
///  [📁 my-repo]  ›  [src]  ›  [views]   ← last segment is current-dir
/// ```
pub fn rebuild_address_bar(
    bar: &gtk::Box,
    repo_name: &str,
    dir: &PathBuf,
    on_segment_clicked: impl Fn(PathBuf) + Clone + 'static,
    on_current_clicked: impl Fn() + Clone + 'static,
) {
    // Safe snapshot-then-unparent: never iterate the live widget tree while
    // mutating it, as a concurrent idle frame may be holding a sibling ref.
    clear_box(bar);

    struct Seg {
        label: String,
        icon: Option<&'static str>,
        target: PathBuf,
    }

    let mut segs: Vec<Seg> = Vec::new();
    segs.push(Seg {
        label: repo_name.to_owned(),
        icon: Some("folder-symbolic"),
        target: PathBuf::new(),
    });

    let mut acc = PathBuf::new();
    for comp in dir.components() {
        let s = comp.as_os_str().to_string_lossy().to_string();
        acc.push(&s);
        segs.push(Seg {
            label: s,
            icon: None,
            target: acc.clone(),
        });
    }

    let total = segs.len();
    for (idx, seg) in segs.iter().enumerate() {
        let is_current = idx == total - 1;

        if idx > 0 {
            let sep = gtk::Image::from_icon_name("go-next-symbolic");
            sep.add_css_class("nautilus-path-separator");
            bar.append(&sep);
        }

        let btn = gtk::Button::new();
        btn.add_css_class("flat");
        btn.add_css_class("nautilus-path-button");
        if is_current {
            btn.add_css_class("current-dir");
        }

        let row = gtk::Box::new(gtk::Orientation::Horizontal, 4);
        if let Some(ic) = seg.icon {
            let img = gtk::Image::from_icon_name(ic);
            img.set_pixel_size(16);
            row.append(&img);
        }
        let lbl = gtk::Label::builder()
            .label(&seg.label)
            .single_line_mode(true)
            .build();
        row.append(&lbl);
        btn.set_child(Some(&row));

        let target = seg.target.clone();
        if is_current {
            let cb = on_current_clicked.clone();
            btn.connect_clicked(move |_| cb());
        } else {
            let cb = on_segment_clicked.clone();
            btn.connect_clicked(move |_| cb(target.clone()));
        }

        bar.append(&btn);
    }
}

/// Prepares and focuses the location-entry widget.
///
/// Sets the entry text to the stringified `current_dir`, switches the
/// toolbar stack to the `"location"` page, grabs focus, and selects all
/// text so the user can type immediately.
pub fn switch_to_location_entry(
    toolbar_switcher: &gtk::Stack,
    location_entry: &gtk::Entry,
    current_dir: &PathBuf,
) {
    let path_text = if current_dir.as_os_str().is_empty() {
        String::new()
    } else {
        current_dir.to_string_lossy().to_string()
    };
    location_entry.set_text(&path_text);
    toolbar_switcher.set_visible_child_name("location");
    location_entry.grab_focus();
    location_entry.select_region(0, -1);
}
