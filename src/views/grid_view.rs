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
//!   [`gtk::GridView`] backed by [`gtk::MultiSelection`].
//! - [`build_grid_cell`] – builds a single icon+label cell box for a node.

use gettextrs::gettext;
use gtk::prelude::*;
use gtk::{gdk, gio, glib};
use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::Rc;

use crate::file_grid_captions_dialog::CaptionFlags;
use crate::file_grid_cell::FileGridCell;
use crate::git_engine::TreeNode;
use crate::icon_helpers::{file_icon, folder_icon};

/// Callback type invoked when the user activates a directory cell.
pub type OnEnterDir = Box<dyn Fn(PathBuf) + 'static>;
/// Callback type invoked when the user activates a file cell.
pub type OnOpenFile = Box<dyn Fn(&std::path::Path, &str) + 'static>;

/// Callback type invoked when the user opens the context menu for a file cell.
pub type OnContextMenu = Box<dyn Fn(&TreeNode, &gtk::Widget) + 'static>;
/// Callback type invoked when the user opens the context menu on the grid background.
pub type OnBackgroundContextMenu = Box<dyn Fn(&gtk::Widget, f64, f64) + 'static>;

#[derive(Debug, Clone, Default)]
pub struct FileGridMetadata {
    pub size: Option<u64>,
    pub size_label: Option<String>,
    pub last_modified: Option<i64>,
    pub last_modified_label: Option<String>,
    pub first_modified: Option<i64>,
    pub first_modified_label: Option<String>,
}

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
                cell_width: 108,
                label_width_chars: 13,
            },
            GridZoom::Normal => GridMetrics {
                icon_size: 96,
                cell_width: 132,
                label_width_chars: 14,
            },
            GridZoom::Large => GridMetrics {
                icon_size: 128,
                cell_width: 164,
                label_width_chars: 17,
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

#[derive(Debug, Clone)]
pub struct GridFileItem {
    pub node: TreeNode,
    pub metadata: Option<FileGridMetadata>,
    pub thumbnail: Option<gdk::Texture>,
}

pub fn build_grid_view(
    children: &[TreeNode],
    hash: &str,
    zoom: GridZoom,
    caption_flags: CaptionFlags,
    metadata: &HashMap<PathBuf, FileGridMetadata>,
    thumbnails: &HashMap<PathBuf, gdk::Texture>,
    on_enter_dir: OnEnterDir,
    on_open_file: OnOpenFile,
    on_context_menu: OnContextMenu,
    on_background_context_menu: OnBackgroundContextMenu,
) -> gtk::Widget {
    let metrics = zoom.metrics();
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

    let model = gio::ListStore::new::<glib::BoxedAnyObject>();
    for node in children {
        model.append(&glib::BoxedAnyObject::new(GridFileItem {
            node: node.clone(),
            metadata: metadata.get(node.path()).cloned(),
            thumbnail: thumbnails.get(node.path()).cloned(),
        }));
    }

    let selection = gtk::MultiSelection::new(Some(model.clone()));
    let factory = gtk::SignalListItemFactory::new();

    factory.connect_setup({
        let selection = selection.clone();
        let on_context_menu = Rc::clone(&on_context_menu);

        move |_, item| {
            let Some(list_item) = item.downcast_ref::<gtk::ListItem>() else {
                return;
            };

            let cell = FileGridCell::new();
            cell.add_css_class("nautilus-grid-view-item");

            let gesture = gtk::GestureClick::builder()
                .button(gtk::gdk::BUTTON_SECONDARY)
                .build();

            gesture.connect_pressed({
                let cell = cell.clone();
                let list_item = list_item.clone();
                let selection = selection.clone();
                let on_context_menu = Rc::clone(&on_context_menu);

                move |gesture, _, _, _| {
                    let position = list_item.position();
                    if position == gtk::INVALID_LIST_POSITION {
                        return;
                    }

                    if !list_item.is_selected() {
                        selection.select_item(position, true);
                    }

                    if let Some(item) = grid_file_item_at(selection.upcast_ref(), position) {
                        on_context_menu(&item.node, cell.upcast_ref());
                        gesture.set_state(gtk::EventSequenceState::Claimed);
                    }
                }
            });

            cell.add_controller(gesture);
            list_item.set_child(Some(&cell));
        }
    });

    factory.connect_bind(move |_, item| {
        let Some(list_item) = item.downcast_ref::<gtk::ListItem>() else {
            return;
        };
        let Some(cell) = list_item.child().and_downcast::<FileGridCell>() else {
            return;
        };
        let Some(item) = list_item_grid_file_item(list_item) else {
            return;
        };

        configure_grid_cell(
            &cell,
            &item.node,
            metrics,
            caption_flags,
            item.metadata.as_ref(),
            item.thumbnail.as_ref(),
        );
    });

    factory.connect_unbind(|_, item| {
        let Some(list_item) = item.downcast_ref::<gtk::ListItem>() else {
            return;
        };
        if let Some(cell) = list_item.child().and_downcast::<FileGridCell>() {
            cell.clear_captions();
            cell.clear_emblems();
            cell.set_paintable(None::<&gdk::Texture>);
        }
    });

    let grid = gtk::GridView::new(Some(selection.clone()), Some(factory.clone()));
    grid.set_enable_rubberband(true);
    grid.set_single_click_activate(false);
    grid.set_max_columns(64);
    grid.set_min_columns(1);
    grid.set_tab_behavior(gtk::ListTabBehavior::Item);
    grid.set_halign(gtk::Align::Fill);
    grid.set_valign(gtk::Align::Start);
    grid.set_hexpand(true);
    grid.set_vexpand(true);
    grid.set_margin_top(12);
    grid.set_margin_bottom(12);
    grid.set_margin_start(12);
    grid.set_margin_end(12);
    grid.add_css_class("view");
    grid.add_css_class("nautilus-grid-view");

    let background_click = gtk::GestureClick::builder().button(0).build();
    background_click.connect_pressed({
        let on_background_context_menu = Rc::clone(&on_background_context_menu);

        glib::clone!(
            #[weak]
            grid,
            #[strong]
            selection,
            move |gesture, _, x, y| {
                if point_hits_grid_cell(&grid, x, y) {
                    return;
                }

                grid.grab_focus();

                let modifiers = gesture.current_event_state();
                let selection_mode = modifiers.intersects(
                    gtk::gdk::ModifierType::CONTROL_MASK | gtk::gdk::ModifierType::SHIFT_MASK,
                );

                match gesture.current_button() {
                    gtk::gdk::BUTTON_PRIMARY if !selection_mode => {
                        selection.unselect_all();
                    }
                    gtk::gdk::BUTTON_SECONDARY => {
                        on_background_context_menu(grid.upcast_ref(), x, y);
                        gesture.set_state(gtk::EventSequenceState::Claimed);
                    }
                    _ => {}
                }
            }
        )
    });
    grid.add_controller(background_click);

    if children.is_empty() {
        let placeholder = gtk::Label::builder()
            .label(gettext("Empty directory"))
            .margin_top(24)
            .margin_bottom(24)
            .can_target(false)
            .build();
        placeholder.add_css_class("dim-label");

        let overlay = gtk::Overlay::new();
        overlay.set_child(Some(&grid));
        overlay.add_overlay(&placeholder);
        scrolled.set_child(Some(&overlay));
        return scrolled.upcast();
    }

    let hash_clone = hash.to_owned();
    grid.connect_activate(move |grid, position| {
        if grid.parent().is_none() {
            return;
        }

        let Some(model) = grid.model() else {
            return;
        };
        let Some(item) = grid_file_item_at(&model, position) else {
            return;
        };

        if item.node.is_dir() || item.node.is_submodule() {
            on_enter_dir(item.node.path().to_path_buf());
        } else {
            on_open_file(item.node.path(), &hash_clone);
        }
    });

    scrolled.set_child(Some(&grid));
    scrolled.upcast()
}

pub fn grid_view_selected_item(grid: &gtk::GridView) -> Option<(GridFileItem, gtk::Widget)> {
    let model = grid.model()?;
    let selection = model.selection();
    if selection.size() == 0 {
        return None;
    }

    let item = grid_file_item_at(&model, selection.nth(0))?;
    Some((item, grid.clone().upcast::<gtk::Widget>()))
}

pub fn set_grid_view_selection(grid: &gtk::GridView, command: GridSelectionCommand) -> usize {
    let Some(model) = grid.model() else {
        return 0;
    };
    let total = model.n_items();

    match command {
        GridSelectionCommand::SelectAll => {
            model.select_all();
            model.selection().size() as usize
        }
        GridSelectionCommand::UnselectAll => {
            model.unselect_all();
            0
        }
        GridSelectionCommand::Invert => {
            let mut selected = 0usize;

            for position in 0..total {
                if model.is_selected(position) {
                    model.unselect_item(position);
                } else {
                    model.select_item(position, false);
                    selected += 1;
                }
            }

            selected
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub enum GridSelectionCommand {
    SelectAll,
    UnselectAll,
    Invert,
}

fn grid_file_item_at(model: &gtk::SelectionModel, position: u32) -> Option<GridFileItem> {
    let list_model = model.dynamic_cast_ref::<gio::ListModel>()?;
    let item = list_model.item(position)?;
    let boxed = item.downcast::<glib::BoxedAnyObject>().ok()?;
    let grid_item = boxed.borrow::<GridFileItem>().clone();
    Some(grid_item)
}

fn list_item_grid_file_item(list_item: &gtk::ListItem) -> Option<GridFileItem> {
    let item = list_item.item()?;
    let boxed = item.downcast::<glib::BoxedAnyObject>().ok()?;
    let grid_item = boxed.borrow::<GridFileItem>().clone();
    Some(grid_item)
}

fn point_hits_grid_cell(grid: &gtk::GridView, x: f64, y: f64) -> bool {
    let Some(mut widget) = grid.pick(x, y, gtk::PickFlags::DEFAULT) else {
        return false;
    };

    loop {
        if widget.has_css_class("nautilus-grid-view-item") {
            return true;
        }

        if widget == *grid {
            return false;
        }

        let Some(parent) = widget.parent() else {
            return false;
        };
        widget = parent;
    }
}

fn configure_grid_cell(
    cell: &FileGridCell,
    node: &TreeNode,
    metrics: GridMetrics,
    caption_flags: CaptionFlags,
    metadata: Option<&FileGridMetadata>,
    thumbnail: Option<&gdk::Texture>,
) {
    cell.set_cell_width(metrics.cell_width);
    cell.set_icon_size(metrics.icon_size);
    cell.set_label_width_chars(metrics.label_width_chars);

    if let Some(thumbnail) = thumbnail {
        cell.set_paintable(Some(thumbnail));
    } else {
        cell.set_paintable(None::<&gdk::Texture>);
        match node {
            TreeNode::Dir(_) => cell.set_gicon(&folder_icon()),
            TreeNode::File(path) => cell.set_gicon(&file_icon(path)),
            TreeNode::Submodule(_) => cell.set_icon_name("folder-remote"),
        }
    }

    let name = node
        .path()
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("");

    cell.set_name(name);
    cell.clear_captions();
    cell.clear_emblems();

    if node.is_submodule() {
        cell.add_emblem("emblem-symbolic-link-symbolic");
    }

    for caption in build_caption_lines(node, caption_flags, metadata) {
        cell.add_caption(&caption);
    }
}

#[allow(dead_code)]
fn build_grid_cell(
    node: &TreeNode,
    metrics: GridMetrics,
    caption_flags: CaptionFlags,
    metadata: Option<&FileGridMetadata>,
) -> FileGridCell {
    let cell = FileGridCell::new();
    configure_grid_cell(&cell, node, metrics, caption_flags, metadata, None);
    cell
}

fn build_caption_lines(
    node: &TreeNode,
    flags: CaptionFlags,
    metadata: Option<&FileGridMetadata>,
) -> Vec<String> {
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
        let size = metadata
            .and_then(|m| m.size_label.clone())
            .unwrap_or_else(|| gettext("Unknown size"));
        lines.push(size);
    }

    if flags.contains(CaptionFlags::DATE) {
        if let Some(label) = metadata.and_then(|m| m.last_modified_label.clone()) {
            lines.push(label);
        } else {
            lines.push(gettext("Unknown date"));
        }
    }

    lines
}
