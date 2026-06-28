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
use std::rc::Rc;

use crate::file_list_row::FileListRow;
use crate::git_engine::TreeNode;

/// Callback type invoked when the user activates a directory entry.
pub type OnEnterDir = Box<dyn Fn(PathBuf) + 'static>;
/// Callback type invoked when the user activates a file entry.
pub type OnOpenFile = Box<dyn Fn(&std::path::Path, &str) + 'static>;
/// Callback type invoked when the user opens the context menu for a row.
pub type OnContextMenu = Box<dyn Fn(&TreeNode, &gtk::Widget) + 'static>;
/// Callback type invoked when the user opens the context menu on the list background.
pub type OnBackgroundContextMenu = Box<dyn Fn(&gtk::Widget, f64, f64) + 'static>;

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
    on_context_menu: OnContextMenu,
    on_background_context_menu: OnBackgroundContextMenu,
) -> gtk::Widget {
    let on_context_menu: Rc<dyn Fn(&TreeNode, &gtk::Widget)> = Rc::from(on_context_menu);
    let on_background_context_menu: Rc<dyn Fn(&gtk::Widget, f64, f64)> =
        Rc::from(on_background_context_menu);

    let scrolled = gtk::ScrolledWindow::builder()
        .vexpand(true)
        .hexpand(true)
        .hadjustment(&gtk::Adjustment::new(0.0, 0.0, 0.0, 0.0, 0.0, 0.0))
        .vadjustment(&gtk::Adjustment::new(0.0, 0.0, 0.0, 0.0, 0.0, 0.0))
        .hscrollbar_policy(gtk::PolicyType::Never)
        .build();

    let list = gtk::ListBox::builder()
        .selection_mode(gtk::SelectionMode::Multiple)
        .build();
    list.add_css_class("boxed-list");

    let background_click = gtk::GestureClick::builder().button(0).build();
    background_click.connect_pressed({
        let on_background_context_menu = Rc::clone(&on_background_context_menu);

        glib::clone!(
            #[weak]
            list,
            move |gesture, _, x, y| {
                if point_hits_list_row(&list, x, y) {
                    return;
                }

                list.grab_focus();

                let modifiers = gesture.current_event_state();
                let selection_mode = modifiers.intersects(
                    gtk::gdk::ModifierType::CONTROL_MASK | gtk::gdk::ModifierType::SHIFT_MASK,
                );

                match gesture.current_button() {
                    gtk::gdk::BUTTON_PRIMARY if !selection_mode => list.unselect_all(),
                    gtk::gdk::BUTTON_SECONDARY => {
                        on_background_context_menu(list.upcast_ref(), x, y);
                        gesture.set_state(gtk::EventSequenceState::Claimed);
                    }
                    _ => {}
                }
            }
        )
    });
    list.add_controller(background_click);

    if children.is_empty() {
        let placeholder = gtk::Label::builder()
            .label(gettext("Empty directory"))
            .margin_top(24)
            .margin_bottom(24)
            .can_target(false)
            .build();
        placeholder.add_css_class("dim-label");
        list.append(&gtk::ListBoxRow::builder().child(&placeholder).build());
    } else {
        for node in children {
            let row = build_file_row(node);
            let node_for_menu = node.clone();
            let on_context_menu = Rc::clone(&on_context_menu);
            let gesture = gtk::GestureClick::builder()
                .button(gtk::gdk::BUTTON_SECONDARY)
                .build();

            gesture.connect_pressed(glib::clone!(
                #[strong]
                row,
                move |gesture, _, _, _| {
                    if let Some(list) = row.parent().and_downcast::<gtk::ListBox>() {
                        if !row.is_selected() {
                            list.unselect_all();
                            list.select_row(Some(&row));
                        }
                    }

                    on_context_menu(&node_for_menu, row.upcast_ref());
                    gesture.set_state(gtk::EventSequenceState::Claimed);
                }
            ));

            row.add_controller(gesture);
            list.append(&row);
        }
    }

    let children_clone = children.to_vec();
    let hash_clone = hash.to_owned();
    // Keep a WeakRef to the list so we can detect when it has been
    // un-parented (panel replaced) before the queued signal fires.
    // If the widget is gone or has no parent the event is stale — bail out.
    let list_weak = list.downgrade();
    list.connect_row_activated(glib::clone!(move |_, row| {
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
    }));

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
    let row = FileListRow::new();
    row.configure(node);
    row.upcast()
}

fn point_hits_list_row(list: &gtk::ListBox, x: f64, y: f64) -> bool {
    let Some(mut widget) = list.pick(x, y, gtk::PickFlags::DEFAULT) else {
        return false;
    };

    loop {
        if widget.clone().downcast::<gtk::ListBoxRow>().is_ok() {
            return true;
        }

        if widget == *list {
            return false;
        }

        let Some(parent) = widget.parent() else {
            return false;
        };
        widget = parent;
    }
}
