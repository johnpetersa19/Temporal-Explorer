/* views.rs
 *
 * Copyright 2026 John Peter Sá
 *
 * SPDX-License-Identifier: GPL-3.0-or-later
 */

//! List/grid views, selection, zoom, view preferences, rendering and grid metadata.

use super::*;
use super::util::*;

#[derive(Debug, Clone, Copy)]
pub(super) enum FileSelectionCommand {
    SelectAll,
    UnselectAll,
    Invert,
}

impl TemporalExplorerWindow {
    pub(super) fn find_flow_box(widget: &gtk::Widget) -> Option<gtk::FlowBox> {
        if let Ok(flow) = widget.clone().downcast::<gtk::FlowBox>() {
            return Some(flow);
        }

        let mut child = widget.first_child();

        while let Some(w) = child {
            let next = w.next_sibling();

            if let Some(found) = Self::find_flow_box(&w) {
                return Some(found);
            }

            child = next;
        }

        None
    }

    pub(super) fn find_grid_view(widget: &gtk::Widget) -> Option<gtk::GridView> {
        if let Ok(grid) = widget.clone().downcast::<gtk::GridView>() {
            return Some(grid);
        }

        let mut child = widget.first_child();

        while let Some(w) = child {
            let next = w.next_sibling();

            if let Some(found) = Self::find_grid_view(&w) {
                return Some(found);
            }

            child = next;
        }

        None
    }

    pub(super) fn find_list_box(widget: &gtk::Widget) -> Option<gtk::ListBox> {
        if let Ok(list) = widget.clone().downcast::<gtk::ListBox>() {
            return Some(list);
        }

        let mut child = widget.first_child();

        while let Some(w) = child {
            let next = w.next_sibling();

            if let Some(found) = Self::find_list_box(&w) {
                return Some(found);
            }

            child = next;
        }

        None
    }

    pub(super) fn set_file_selection(&self, command: FileSelectionCommand) -> Option<usize> {
        let Some(root) = self.active_tab_content() else {
            return None;
        };

        if let Some(list) = Self::find_list_box(root.upcast_ref()) {
            return Some(Self::set_list_box_selection(&list, command));
        }

        if let Some(grid) = Self::find_grid_view(root.upcast_ref()) {
            let command = match command {
                FileSelectionCommand::SelectAll => grid_view::GridSelectionCommand::SelectAll,
                FileSelectionCommand::UnselectAll => grid_view::GridSelectionCommand::UnselectAll,
                FileSelectionCommand::Invert => grid_view::GridSelectionCommand::Invert,
            };
            return Some(grid_view::set_grid_view_selection(&grid, command));
        }

        if let Some(flow) = Self::find_flow_box(root.upcast_ref()) {
            return Some(Self::set_flow_box_selection(&flow, command));
        }

        None
    }

    pub(super) fn set_list_box_selection(list: &gtk::ListBox, command: FileSelectionCommand) -> usize {
        match command {
            FileSelectionCommand::SelectAll => {
                list.select_all();
                list.selected_rows().len()
            }
            FileSelectionCommand::UnselectAll => {
                list.unselect_all();
                0
            }
            FileSelectionCommand::Invert => {
                let mut selected = 0usize;
                let mut child = list.first_child();

                while let Some(widget) = child {
                    child = widget.next_sibling();

                    if let Ok(row) = widget.downcast::<gtk::ListBoxRow>() {
                        if row.is_selected() {
                            list.unselect_row(&row);
                        } else {
                            list.select_row(Some(&row));
                            selected += 1;
                        }
                    }
                }

                selected
            }
        }
    }

    pub(super) fn set_flow_box_selection(flow: &gtk::FlowBox, command: FileSelectionCommand) -> usize {
        match command {
            FileSelectionCommand::SelectAll => {
                flow.select_all();
                flow.selected_children().len()
            }
            FileSelectionCommand::UnselectAll => {
                flow.unselect_all();
                0
            }
            FileSelectionCommand::Invert => {
                let mut selected = 0usize;
                let mut child = flow.first_child();

                while let Some(widget) = child {
                    child = widget.next_sibling();

                    if let Ok(flow_child) = widget.downcast::<gtk::FlowBoxChild>() {
                        if flow_child.is_selected() {
                            flow.unselect_child(&flow_child);
                        } else {
                            flow.select_child(&flow_child);
                            selected += 1;
                        }
                    }
                }

                selected
            }
        }
    }

    pub(super) fn select_all_files(&self) {
        match self.set_file_selection(FileSelectionCommand::SelectAll) {
            Some(count) => {
                self.show_toast(&format!("{} {} item(s)", gettext("Selected"), count));
            }
            None => self.show_toast(&gettext("No file view available")),
        }
    }

    pub(super) fn unselect_all_files(&self) {
        match self.set_file_selection(FileSelectionCommand::UnselectAll) {
            Some(_) => self.show_toast(&gettext("Selection cleared")),
            None => self.show_toast(&gettext("No file view available")),
        }
    }

    pub(super) fn invert_file_selection(&self) {
        match self.set_file_selection(FileSelectionCommand::Invert) {
            Some(count) => {
                self.show_toast(&format!("{} {} item(s)", gettext("Selected"), count));
            }
            None => self.show_toast(&gettext("No file view available")),
        }
    }

    pub(super) fn toggle_sidebar(&self) {
        let current = self.imp().split_view.shows_sidebar();
        self.imp().split_view.set_show_sidebar(!current);
    }

    pub(super) fn switch_to_list_view(&self) {
        self.set_view_mode_from_action(false);
    }

    pub(super) fn switch_to_grid_view(&self) {
        self.set_view_mode_from_action(true);
    }

    pub(super) fn set_view_mode_from_action(&self, is_grid: bool) {
        *self.imp().view_mode.borrow_mut() = if is_grid {
            ViewMode::Grid
        } else {
            ViewMode::List
        };
        self.imp().toolbar.view_controls().set_view_mode(is_grid);

        if let Some(settings) = self.imp().settings.get() {
            settings
                .set_string("default-view", if is_grid { "grid" } else { "list" })
                .ok();
        }

        self.reload_current_dir_if_possible();
    }

    pub(super) fn zoom_in(&self) {
        let raw = match *self.imp().grid_zoom.borrow() {
            GridZoom::Small => 1,
            GridZoom::Normal => 2,
            GridZoom::Large => 2,
        };
        self.set_grid_zoom_from_action(raw);
    }

    pub(super) fn zoom_out(&self) {
        let raw = match *self.imp().grid_zoom.borrow() {
            GridZoom::Small => 0,
            GridZoom::Normal => 0,
            GridZoom::Large => 1,
        };
        self.set_grid_zoom_from_action(raw);
    }

    pub(super) fn reset_zoom(&self) {
        self.set_grid_zoom_from_action(1);
    }

    pub(super) fn set_grid_zoom_from_action(&self, raw: u32) {
        let raw = raw.min(2);
        let zoom = match raw {
            0 => GridZoom::Small,
            2 => GridZoom::Large,
            _ => GridZoom::Normal,
        };

        *self.imp().grid_zoom.borrow_mut() = zoom;
        self.imp().toolbar.view_controls().set_zoom_level(raw);

        if let Some(settings) = self.imp().settings.get() {
            settings.set_uint("grid-zoom-level", raw).ok();
        }

        self.reload_current_dir_if_possible();
    }

    pub(super) fn select_all_commits(&self) {
        let mut count = 0usize;
        let mut row = self.imp().commit_list.first_child();

        while let Some(widget) = row {
            row = widget.next_sibling();
            if let Ok(list_row) = widget.downcast::<gtk::ListBoxRow>() {
                if !list_row.widget_name().is_empty() {
                    list_row.add_css_class("pattern-match");
                    count += 1;
                }
            }
        }

        self.show_toast(&format!("{} {} commit(s)", gettext("Selected"), count));
    }

    pub(super) fn invert_commit_selection(&self) {
        let mut count = 0usize;
        let mut row = self.imp().commit_list.first_child();

        while let Some(widget) = row {
            row = widget.next_sibling();
            if let Ok(list_row) = widget.downcast::<gtk::ListBoxRow>() {
                if list_row.widget_name().is_empty() {
                    continue;
                }

                if list_row.has_css_class("pattern-match") {
                    list_row.remove_css_class("pattern-match");
                } else {
                    list_row.add_css_class("pattern-match");
                    count += 1;
                }
            }
        }

        self.show_toast(&format!("{} {} commit(s)", gettext("Selected"), count));
    }

    pub(super) fn select_first_commit(&self) {
        let hash = self
            .imp()
            .all_commits
            .borrow()
            .last()
            .map(|commit| commit.hash.clone());

        if let Some(hash) = hash {
            self.on_commit_selected(hash);
        } else {
            self.show_toast(&gettext("No commits loaded"));
        }
    }

    pub(super) fn select_latest_commit(&self) {
        let hash = self
            .imp()
            .all_commits
            .borrow()
            .first()
            .map(|commit| commit.hash.clone());

        if let Some(hash) = hash {
            self.on_commit_selected(hash);
        } else {
            self.show_toast(&gettext("No commits loaded"));
        }
    }

    pub(super) fn setup_saved_view_preferences(&self) {
        let imp = self.imp();
        let Some(settings) = imp.settings.get() else {
            return;
        };

        let saved_view = settings.string("default-view");
        let is_grid = saved_view.as_str() == "grid";
        *imp.view_mode.borrow_mut() = if is_grid {
            ViewMode::Grid
        } else {
            ViewMode::List
        };
        imp.toolbar.view_controls().set_view_mode(is_grid);

        let zoom_level = settings.uint("grid-zoom-level").min(2);
        *imp.grid_zoom.borrow_mut() = match zoom_level {
            0 => GridZoom::Small,
            2 => GridZoom::Large,
            _ => GridZoom::Normal,
        };
        imp.toolbar.view_controls().set_zoom_level(zoom_level);

        let sort_mode = match settings.string("file-sort-mode").as_str() {
            "name-desc" => FileSortMode::NameDescending,
            "last-modified" => FileSortMode::LastModified,
            "first-modified" => FileSortMode::FirstModified,
            "size" => FileSortMode::Size,
            "type" => FileSortMode::Extension,
            _ => FileSortMode::Name,
        };
        *imp.sort_mode.borrow_mut() = sort_mode;
        imp.toolbar.view_controls().set_sort_mode(sort_mode);

        *imp.grid_caption_flags.borrow_mut() =
            CaptionFlags::from_bits_truncate(settings.uint("grid-caption-flags"));

        let show_hidden = settings.boolean("show-hidden-files");
        imp.show_hidden_files.set(show_hidden);
        imp.toolbar
            .view_controls()
            .set_show_hidden_files(show_hidden);
    }

    pub(super) fn setup_settings_change_listeners(&self) {
        let Some(settings) = self.imp().settings.get() else {
            return;
        };

        let win = self.clone();
        settings.connect_changed(Some("default-view"), move |settings, _| {
            let is_grid = settings.string("default-view").as_str() == "grid";
            let next = if is_grid {
                ViewMode::Grid
            } else {
                ViewMode::List
            };

            if *win.imp().view_mode.borrow() != next {
                *win.imp().view_mode.borrow_mut() = next;
                win.imp().toolbar.view_controls().set_view_mode(is_grid);
                win.reload_current_dir_if_possible();
            }
        });

        let win = self.clone();
        settings.connect_changed(Some("grid-zoom-level"), move |settings, _| {
            let raw = settings.uint("grid-zoom-level").min(2);
            let next = match raw {
                0 => GridZoom::Small,
                2 => GridZoom::Large,
                _ => GridZoom::Normal,
            };

            if *win.imp().grid_zoom.borrow() != next {
                *win.imp().grid_zoom.borrow_mut() = next;
                win.imp().toolbar.view_controls().set_zoom_level(raw);
                win.reload_current_dir_if_possible();
            }
        });

        let win = self.clone();
        settings.connect_changed(Some("file-sort-mode"), move |settings, _| {
            let next = match settings.string("file-sort-mode").as_str() {
                "name-desc" => FileSortMode::NameDescending,
                "last-modified" => FileSortMode::LastModified,
                "first-modified" => FileSortMode::FirstModified,
                "size" => FileSortMode::Size,
                "type" => FileSortMode::Extension,
                _ => FileSortMode::Name,
            };

            if *win.imp().sort_mode.borrow() != next {
                *win.imp().sort_mode.borrow_mut() = next;
                win.imp().toolbar.view_controls().set_sort_mode(next);
                win.reload_current_dir_if_possible();
            }
        });

        let win = self.clone();
        settings.connect_changed(Some("grid-caption-flags"), move |settings, _| {
            let next = CaptionFlags::from_bits_truncate(settings.uint("grid-caption-flags"));

            if *win.imp().grid_caption_flags.borrow() != next {
                *win.imp().grid_caption_flags.borrow_mut() = next;
                win.reload_current_dir_if_possible();
            }
        });

        let win = self.clone();
        settings.connect_changed(Some("show-hidden-files"), move |settings, _| {
            let next = settings.boolean("show-hidden-files");

            if win.imp().show_hidden_files.get() != next {
                win.imp().show_hidden_files.set(next);
                win.imp()
                    .toolbar
                    .view_controls()
                    .set_show_hidden_files(next);
                win.reload_current_dir_if_possible();
            }
        });
    }

    pub(super) fn setup_view_controls(&self) {
        let win = self.clone();
        self.imp()
            .toolbar
            .view_controls()
            .connect_local("view-mode-changed", false, move |args| {
                let is_grid = args[1].get::<bool>().unwrap_or(false);
                *win.imp().view_mode.borrow_mut() = if is_grid {
                    ViewMode::Grid
                } else {
                    ViewMode::List
                };
                if let Some(settings) = win.imp().settings.get() {
                    settings
                        .set_string("default-view", if is_grid { "grid" } else { "list" })
                        .ok();
                }
                let dir = win.imp().current_dir.borrow().clone();
                if win.imp().current_hash.borrow().is_some() {
                    win.navigate_to_dir(dir);
                }
                None
            });

        let win = self.clone();
        self.imp()
            .toolbar
            .view_controls()
            .connect_local("sort-changed", false, move |args| {
                let raw = args[1].get::<u32>().unwrap_or(0);
                let mode = match raw {
                    1 => FileSortMode::NameDescending,
                    2 => FileSortMode::LastModified,
                    3 => FileSortMode::FirstModified,
                    4 => FileSortMode::Size,
                    5 => FileSortMode::Extension,
                    _ => FileSortMode::Name,
                };
                *win.imp().sort_mode.borrow_mut() = mode;
                let sort_key = match mode {
                    FileSortMode::Name => "name",
                    FileSortMode::NameDescending => "name-desc",
                    FileSortMode::LastModified => "last-modified",
                    FileSortMode::FirstModified => "first-modified",
                    FileSortMode::Size => "size",
                    FileSortMode::Status => "status",
                    FileSortMode::Extension => "type",
                };
                if let Some(settings) = win.imp().settings.get() {
                    settings.set_string("file-sort-mode", sort_key).ok();
                }
                let dir = win.imp().current_dir.borrow().clone();
                if win.imp().current_hash.borrow().is_some() {
                    win.navigate_to_dir(dir);
                }
                None
            });

        let win = self.clone();
        self.imp()
            .toolbar
            .view_controls()
            .connect_local("zoom-changed", false, move |args| {
                let raw = args[1].get::<u32>().unwrap_or(1);
                let zoom = match raw {
                    0 => GridZoom::Small,
                    2 => GridZoom::Large,
                    _ => GridZoom::Normal,
                };
                *win.imp().grid_zoom.borrow_mut() = zoom;
                if let Some(settings) = win.imp().settings.get() {
                    settings.set_uint("grid-zoom-level", raw.min(2)).ok();
                }
                let dir = win.imp().current_dir.borrow().clone();
                if win.imp().current_hash.borrow().is_some() {
                    win.navigate_to_dir(dir);
                }
                None
            });

        let win = self.clone();
        self.imp().toolbar.view_controls().connect_local(
            "hidden-files-changed",
            false,
            move |args| {
                let show_hidden = args[1].get::<bool>().unwrap_or(false);
                win.imp().show_hidden_files.set(show_hidden);

                if let Some(settings) = win.imp().settings.get() {
                    settings.set_boolean("show-hidden-files", show_hidden).ok();
                }

                win.reload_current_dir_if_possible();
                None
            },
        );

        let win = self.clone();
        self.imp().toolbar.view_controls().connect_local(
            "visible-columns-requested",
            false,
            move |_| {
                win.show_column_chooser();
                None
            },
        );

        let win = self.clone();
        self.imp()
            .toolbar
            .view_controls()
            .connect_local("captions-requested", false, move |_| {
                win.show_file_grid_captions_dialog();
                None
            });
    }

    pub fn show_file_grid_captions_dialog(&self) {
        let dialog = FileGridCaptionsDialog::new();
        dialog.set_flags(*self.imp().grid_caption_flags.borrow());

        let win = self.clone();
        dialog.connect_captions_changed(move |_, flags| {
            *win.imp().grid_caption_flags.borrow_mut() = flags;
            if let Some(settings) = win.imp().settings.get() {
                settings.set_uint("grid-caption-flags", flags.bits()).ok();
            }

            let dir = win.imp().current_dir.borrow().clone();
            if win.imp().current_hash.borrow().is_some() {
                win.navigate_to_dir(dir);
            }
        });

        AdwDialogExt::present(&dialog, Some(self.upcast_ref::<gtk::Widget>()));
    }

    pub fn show_column_chooser(&self) {
        let dialog = ColumnChooser::new();
        dialog.apply_visibility(&self.imp().column_visibility.borrow());

        let win = self.clone();
        let dlg_ref = dialog.clone();
        dialog.connect_local("columns-changed", false, move |_| {
            *win.imp().column_visibility.borrow_mut() = dlg_ref.visibility();
            let dir = win.imp().current_dir.borrow().clone();
            if win.imp().current_hash.borrow().is_some() {
                win.navigate_to_dir(dir);
            }
            None
        });

        AdwDialogExt::present(&dialog, Some(self.upcast_ref::<gtk::Widget>()));
    }

    pub(super) fn render_dir(&self, nodes: Vec<TreeNode>) {
        let imp = self.imp();
        let mode = *imp.view_mode.borrow();
        let sort_mode = *imp.sort_mode.borrow();
        let grid_zoom = *imp.grid_zoom.borrow();
        let grid_caption_flags = *imp.grid_caption_flags.borrow();
        let hash = imp.current_hash.borrow().clone().unwrap_or_default();
        let nodes = self.visible_nodes_for_current_settings(nodes);

        // Metadata can be expensive, especially First/Last Modified because it
        // scans commit history. Only build it when the active sort/captions need it.
        let metadata = if self.file_grid_metadata_needed(sort_mode, grid_caption_flags) {
            self.build_file_grid_metadata(&nodes, &hash, sort_mode, grid_caption_flags)
        } else {
            HashMap::new()
        };

        let mut decorated: Vec<(TreeNode, String, String)> = nodes
            .into_iter()
            .map(|node| {
                let name = node
                    .path()
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("")
                    .to_lowercase();

                let ext = std::path::Path::new(&name)
                    .extension()
                    .and_then(|e| e.to_str())
                    .unwrap_or("")
                    .to_string();

                (node, name, ext)
            })
            .collect();

        match sort_mode {
            FileSortMode::Name | FileSortMode::Status => {
                decorated.sort_by(|a, b| b.0.is_dir().cmp(&a.0.is_dir()).then(a.1.cmp(&b.1)));
            }
            FileSortMode::NameDescending => {
                decorated.sort_by(|a, b| b.0.is_dir().cmp(&a.0.is_dir()).then(b.1.cmp(&a.1)));
            }
            FileSortMode::LastModified => {
                decorated.sort_by(|a, b| {
                    let ma = metadata
                        .get(a.0.path())
                        .and_then(|m| m.last_modified)
                        .unwrap_or(i64::MIN);
                    let mb = metadata
                        .get(b.0.path())
                        .and_then(|m| m.last_modified)
                        .unwrap_or(i64::MIN);
                    b.0.is_dir()
                        .cmp(&a.0.is_dir())
                        .then(mb.cmp(&ma))
                        .then(a.1.cmp(&b.1))
                });
            }
            FileSortMode::FirstModified => {
                decorated.sort_by(|a, b| {
                    let ma = metadata
                        .get(a.0.path())
                        .and_then(|m| m.first_modified)
                        .unwrap_or(i64::MAX);
                    let mb = metadata
                        .get(b.0.path())
                        .and_then(|m| m.first_modified)
                        .unwrap_or(i64::MAX);
                    b.0.is_dir()
                        .cmp(&a.0.is_dir())
                        .then(ma.cmp(&mb))
                        .then(a.1.cmp(&b.1))
                });
            }
            FileSortMode::Size => {
                decorated.sort_by(|a, b| {
                    let sa = metadata.get(a.0.path()).and_then(|m| m.size).unwrap_or(0);
                    let sb = metadata.get(b.0.path()).and_then(|m| m.size).unwrap_or(0);
                    b.0.is_dir()
                        .cmp(&a.0.is_dir())
                        .then(sb.cmp(&sa))
                        .then(a.1.cmp(&b.1))
                });
            }
            FileSortMode::Extension => {
                decorated.sort_by(|a, b| {
                    b.0.is_dir()
                        .cmp(&a.0.is_dir())
                        .then(a.2.cmp(&b.2))
                        .then(a.1.cmp(&b.1))
                });
            }
        }

        let nodes: Vec<TreeNode> = decorated.into_iter().map(|(node, _, _)| node).collect();
        *imp.current_nodes.borrow_mut() = nodes.clone();

        let win1 = self.clone();
        let win2 = self.clone();
        let win3 = self.clone();
        let win4 = self.clone();
        let win5 = self.clone();
        let win6 = self.clone();
        let on_enter_dir: OnEnterDir = Box::new(move |path: PathBuf| {
            win1.push_dir(path);
        });
        let on_open_file: OnOpenFile = Box::new(move |path: &std::path::Path, _h: &str| {
            win2.open_snapshot_node_with_default_app(&TreeNode::File(path.to_path_buf()));
        });
        let on_context_menu: grid_view::OnContextMenu =
            Box::new(move |node: &TreeNode, anchor: &gtk::Widget| {
                win3.show_file_context_menu(node.clone(), anchor);
            });
        let on_background_context_menu: grid_view::OnBackgroundContextMenu =
            Box::new(move |anchor: &gtk::Widget, x, y| {
                win4.show_file_background_context_menu(anchor, x, y);
            });
        let on_list_context_menu: list_view::OnContextMenu =
            Box::new(move |node: &TreeNode, anchor: &gtk::Widget| {
                win5.show_file_context_menu(node.clone(), anchor);
            });
        let on_list_background_context_menu: list_view::OnBackgroundContextMenu =
            Box::new(move |anchor: &gtk::Widget, x, y| {
                win6.show_file_background_context_menu(anchor, x, y);
            });

        let widget: gtk::Widget = match mode {
            ViewMode::List => list_view::build_list_view(
                &nodes,
                &hash,
                on_enter_dir,
                on_open_file,
                on_list_context_menu,
                on_list_background_context_menu,
            )
            .upcast(),
            ViewMode::Grid => {
                let thumbnails = self.build_file_grid_thumbnails(&nodes, &hash);
                grid_view::build_grid_view(
                    &nodes,
                    &hash,
                    grid_zoom,
                    grid_caption_flags,
                    &metadata,
                    &thumbnails,
                    on_enter_dir,
                    on_open_file,
                    on_context_menu,
                    on_background_context_menu,
                )
                .upcast()
            }
        };
        self.replace_right_panel(widget);
        self.save_active_tab_state();
    }

    pub(super) fn visible_nodes_for_current_settings(&self, nodes: Vec<TreeNode>) -> Vec<TreeNode> {
        if self.imp().show_hidden_files.get() {
            return nodes;
        }

        nodes
            .into_iter()
            .filter(|node| {
                node.path()
                    .file_name()
                    .and_then(|name| name.to_str())
                    .map_or(true, |name| !name.starts_with('.'))
            })
            .collect()
    }

    pub(super) fn selected_file_view_node(&self) -> Option<TreeNode> {
        self.selected_file_view_node_and_anchor()
            .map(|(node, _)| node)
    }

    pub(super) fn selected_file_view_node_and_anchor(&self) -> Option<(TreeNode, gtk::Widget)> {
        let root = self.active_tab_content()?;

        if let Some(list) = Self::find_list_box(root.upcast_ref()) {
            if let Some(row) = list.selected_rows().first() {
                let idx = row.index();
                if idx >= 0 {
                    return self
                        .imp()
                        .current_nodes
                        .borrow()
                        .get(idx as usize)
                        .cloned()
                        .map(|node| (node, row.clone().upcast::<gtk::Widget>()));
                }
            }
        }

        if let Some(grid) = Self::find_grid_view(root.upcast_ref()) {
            if let Some((item, anchor)) = grid_view::grid_view_selected_item(&grid) {
                return Some((item.node, anchor));
            }
        }

        if let Some(flow) = Self::find_flow_box(root.upcast_ref()) {
            if let Some(child) = flow.selected_children().first() {
                let idx = child.index();
                if idx >= 0 {
                    return self
                        .imp()
                        .current_nodes
                        .borrow()
                        .get(idx as usize)
                        .cloned()
                        .map(|node| (node, child.clone().upcast::<gtk::Widget>()));
                }
            }
        }

        None
    }

    pub(super) fn file_grid_metadata_needed(
        &self,
        sort_mode: FileSortMode,
        caption_flags: CaptionFlags,
    ) -> bool {
        matches!(
            sort_mode,
            FileSortMode::Size | FileSortMode::LastModified | FileSortMode::FirstModified
        ) || caption_flags.intersects(CaptionFlags::SIZE | CaptionFlags::DATE)
    }

    pub(super) fn build_file_grid_thumbnails(
        &self,
        nodes: &[TreeNode],
        hash: &str,
    ) -> HashMap<PathBuf, gdk::Texture> {
        const MAX_THUMBNAIL_BYTES: usize = 8 * 1024 * 1024;

        let mut out = HashMap::new();
        let imp = self.imp();
        let repo_ref = imp.repository.borrow();
        let Some(repo_wrapper) = repo_ref.as_ref() else {
            return out;
        };
        let repo: &git2::Repository = &repo_wrapper.0;

        let Some(tree) = repo
            .revparse_single(hash)
            .ok()
            .and_then(|obj| obj.peel_to_commit().ok())
            .and_then(|commit| commit.tree().ok())
        else {
            return out;
        };

        for node in nodes {
            let thumbnail = match node {
                TreeNode::File(path) if is_thumbnail_image_path(path) => {
                    texture_for_tree_file(repo, &tree, path, MAX_THUMBNAIL_BYTES)
                }
                _ => None,
            };

            if let Some(texture) = thumbnail {
                out.insert(node.path().to_path_buf(), texture);
            }
        }

        out
    }

    pub(super) fn build_file_grid_metadata(
        &self,
        nodes: &[TreeNode],
        hash: &str,
        sort_mode: FileSortMode,
        caption_flags: CaptionFlags,
    ) -> HashMap<PathBuf, FileGridMetadata> {
        let mut out = HashMap::new();

        let imp = self.imp();
        let repo_ref = imp.repository.borrow();
        let Some(repo_wrapper) = repo_ref.as_ref() else {
            return out;
        };
        let repo: &git2::Repository = &repo_wrapper.0;

        let tree = repo
            .revparse_single(hash)
            .ok()
            .and_then(|obj| obj.peel_to_commit().ok())
            .and_then(|commit| commit.tree().ok());

        let needs_size =
            matches!(sort_mode, FileSortMode::Size) || caption_flags.contains(CaptionFlags::SIZE);

        let needs_dates = matches!(
            sort_mode,
            FileSortMode::LastModified | FileSortMode::FirstModified
        ) || caption_flags.contains(CaptionFlags::DATE);

        for node in nodes {
            let mut meta = FileGridMetadata::default();
            let path = node.path().to_path_buf();

            if needs_size {
                if let Some(ref tree) = tree {
                    meta.size = self.git_blob_size(repo, tree, node);
                    meta.size_label = meta.size.map(format_file_size);
                }
            }

            out.insert(path, meta);
        }

        if needs_dates {
            self.fill_file_grid_dates(repo, nodes, &mut out);
        }

        out
    }

    pub(super) fn git_blob_size(
        &self,
        repo: &git2::Repository,
        tree: &git2::Tree<'_>,
        node: &TreeNode,
    ) -> Option<u64> {
        if !matches!(node, TreeNode::File(_)) {
            return None;
        }

        let entry = tree.get_path(node.path()).ok()?;
        let blob = repo.find_blob(entry.id()).ok()?;
        Some(blob.size() as u64)
    }

    pub(super) fn fill_file_grid_dates(
        &self,
        repo: &git2::Repository,
        nodes: &[TreeNode],
        metadata: &mut HashMap<PathBuf, FileGridMetadata>,
    ) {
        // This replaces the old O(nodes × commits × diff) path.
        // We now scan the history once and reuse each commit diff for all
        // currently visible nodes.
        let commits = self.imp().all_commits.borrow();

        let visible_paths: Vec<(PathBuf, bool)> = nodes
            .iter()
            .map(|node| (node.path().to_path_buf(), node.is_dir()))
            .collect();

        for commit_info in commits.iter() {
            let Ok(commit) = repo.find_commit(commit_info.oid()) else {
                continue;
            };

            let Ok(tree) = commit.tree() else {
                continue;
            };

            let parent_tree = commit.parent(0).ok().and_then(|parent| parent.tree().ok());

            let Ok(diff) = repo.diff_tree_to_tree(parent_tree.as_ref(), Some(&tree), None) else {
                continue;
            };

            for delta in diff.deltas() {
                let old_path = delta.old_file().path();
                let new_path = delta.new_file().path();

                for (path, is_dir) in &visible_paths {
                    let matches_path = |candidate: Option<&std::path::Path>| -> bool {
                        let Some(candidate) = candidate else {
                            return false;
                        };

                        if *is_dir {
                            candidate == path || candidate.starts_with(path)
                        } else {
                            candidate == path
                        }
                    };

                    if matches_path(old_path) || matches_path(new_path) {
                        if let Some(meta) = metadata.get_mut(path) {
                            // all_commits is newest -> oldest.
                            // First hit is the last modification.
                            if meta.last_modified.is_none() {
                                meta.last_modified = Some(commit_info.timestamp);
                                meta.last_modified_label =
                                    Some(Self::format_timestamp(commit_info.timestamp));
                            }

                            // Keep updating; after the loop, this is the oldest hit.
                            meta.first_modified = Some(commit_info.timestamp);
                            meta.first_modified_label =
                                Some(Self::format_timestamp(commit_info.timestamp));
                        }
                    }
                }
            }
        }
    }
}
