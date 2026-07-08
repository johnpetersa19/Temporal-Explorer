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

use gettextrs::gettext;
use gtk::prelude::*;
use std::path::PathBuf;

/// Removes all children from a `gtk::Box` safely.
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
#[allow(dead_code)]
pub fn rebuild_address_bar(
    bar: &gtk::Box,
    repo_name: &str,
    dir: &PathBuf,
    on_segment_clicked: impl Fn(PathBuf) + Clone + 'static,
    on_current_clicked: impl Fn() + Clone + 'static,
) {
    rebuild_address_bar_with_context(
        bar,
        repo_name,
        dir,
        on_segment_clicked,
        on_current_clicked,
        |_| {},
        |_| {},
    );
}

/// Rebuilds the path bar with Nautilus-like per-segment context actions.
///
/// The context menu is intentionally adapted to Temporal Explorer snapshots:
/// it exposes navigation, copying the relative snapshot path, and properties.
pub fn rebuild_address_bar_with_context(
    bar: &gtk::Box,
    repo_name: &str,
    dir: &PathBuf,
    on_segment_clicked: impl Fn(PathBuf) + Clone + 'static,
    on_current_clicked: impl Fn() + Clone + 'static,
    on_copy_path: impl Fn(PathBuf) + Clone + 'static,
    on_properties: impl Fn(PathBuf) + Clone + 'static,
) {
    clear_box(bar);

    struct Seg {
        label: String,
        icon: Option<&'static str>,
        target: PathBuf,
    }

    let mut segs: Vec<Seg> = Vec::new();
    segs.push(Seg {
        label: repo_name.to_owned(),
        icon: Some("user-home-symbolic"),
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
            let sep = gtk::Label::new(Some("/"));
            sep.add_css_class("nautilus-path-separator");
            bar.append(&sep);
        }

        let btn = gtk::Button::new();
        btn.add_css_class("flat");
        btn.add_css_class("nautilus-path-button");
        btn.set_tooltip_text(Some(&seg.target.to_string_lossy()));
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
            .ellipsize(gtk::pango::EllipsizeMode::Middle)
            .max_width_chars(if is_current { 28 } else { 18 })
            .single_line_mode(true)
            .build();
        lbl.set_tooltip_text(Some(&seg.target.to_string_lossy()));
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

        let target = seg.target.clone();
        let nav_cb = on_segment_clicked.clone();
        let edit_cb = on_current_clicked.clone();
        let copy_cb = on_copy_path.clone();
        let properties_cb = on_properties.clone();
        let gesture = gtk::GestureClick::builder()
            .button(gtk::gdk::BUTTON_SECONDARY)
            .build();
        gesture.connect_pressed(move |gesture, _, x, y| {
            let Some(widget) = gesture.widget() else {
                return;
            };

            let popover = gtk::Popover::new();
            popover.set_has_arrow(false);
            popover.add_css_class("pathbar-context-menu");
            popover.set_parent(&widget);
            popover.set_pointing_to(Some(&gtk::gdk::Rectangle::new(
                x.round() as i32,
                y.round() as i32,
                1,
                1,
            )));

            let box_ = gtk::Box::new(gtk::Orientation::Vertical, 2);
            box_.add_css_class("toolbar-popover-list");

            let open_button = gtk::Button::with_label(&gettext("Open Snapshot Location"));
            open_button.add_css_class("flat");
            {
                let popover = popover.clone();
                let target = target.clone();
                let nav_cb = nav_cb.clone();
                let edit_cb = edit_cb.clone();
                open_button.connect_clicked(move |_| {
                    if is_current {
                        edit_cb();
                    } else {
                        nav_cb(target.clone());
                    }
                    popover.popdown();
                });
            }
            box_.append(&open_button);

            let copy_button = gtk::Button::with_label(&gettext("Copy Snapshot Path"));
            copy_button.add_css_class("flat");
            {
                let popover = popover.clone();
                let target = target.clone();
                let copy_cb = copy_cb.clone();
                copy_button.connect_clicked(move |_| {
                    copy_cb(target.clone());
                    popover.popdown();
                });
            }
            box_.append(&copy_button);

            let properties_button = gtk::Button::with_label(&gettext("Properties"));
            properties_button.add_css_class("flat");
            {
                let popover = popover.clone();
                let target = target.clone();
                let properties_cb = properties_cb.clone();
                properties_button.connect_clicked(move |_| {
                    properties_cb(target.clone());
                    popover.popdown();
                });
            }
            box_.append(&properties_button);

            popover.set_child(Some(&box_));
            popover.connect_closed(|p| {
                let popover = p.clone();
                gtk::glib::idle_add_local_once(move || {
                    popover.unparent();
                });
            });
            popover.popup();
            gesture.set_state(gtk::EventSequenceState::Claimed);
        });
        btn.add_controller(gesture);

        bar.append(&btn);
    }
}
