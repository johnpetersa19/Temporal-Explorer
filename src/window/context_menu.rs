/* context_menu.rs
 *
 * Copyright 2026 John Peter Sá
 *
 * SPDX-License-Identifier: GPL-3.0-or-later
 */

//! File and background context menus.

use super::*;

impl TemporalExplorerWindow {
    pub(super) fn open_context_menu_from_action(&self) {
        if let Some((node, anchor)) = self.selected_file_view_node_and_anchor() {
            self.show_file_context_menu(node, &anchor);
        } else {
            self.show_toast(&gettext("No file selected"));
        }
    }

    pub(super) fn show_file_background_context_menu(&self, anchor: &gtk::Widget, x: f64, y: f64) {
        let menu = gio::Menu::new();

        let selection_section = gio::Menu::new();
        selection_section.append(Some(&gettext("Reload")), Some("win.reload-repository"));
        selection_section.append(Some(&gettext("Select All")), Some("win.select-all-files"));
        selection_section.append(
            Some(&gettext("Invert Selection")),
            Some("win.invert-file-selection"),
        );
        selection_section.append(
            Some(&gettext("Clear Selection")),
            Some("win.unselect-all-files"),
        );
        menu.append_section(None, &selection_section);

        let view_section = gio::Menu::new();
        view_section.append(
            Some(&gettext("Visible Columns…")),
            Some("win.show-column-chooser"),
        );
        view_section.append(Some(&gettext("Captions…")), Some("win.show-captions"));
        view_section.append(
            Some(&gettext("Select Commits by Pattern…")),
            Some("win.select-by-pattern"),
        );
        menu.append_section(None, &view_section);

        let folder_section = gio::Menu::new();
        folder_section.append(
            Some(&gettext("Open Repository in System")),
            Some("win.open-repository-system"),
        );
        folder_section.append(
            Some(&gettext("Open in Console")),
            Some("win.open-repository-console"),
        );
        folder_section.append(
            Some(&gettext("Copy Snapshot Location")),
            Some("win.copy-current-location"),
        );
        menu.append_section(None, &folder_section);

        let properties_section = gio::Menu::new();
        properties_section.append(Some(&gettext("Properties")), Some("win.current-properties"));
        menu.append_section(None, &properties_section);

        let popover = gtk::PopoverMenu::from_model(Some(&menu));
        popover.set_has_arrow(false);
        popover.set_halign(gtk::Align::Start);
        popover.set_position(gtk::PositionType::Bottom);
        popover.add_css_class("nautilus-context-menu");
        if let Some(point) = anchor.compute_point(
            self,
            &gtk::graphene::Point::new(x as f32, y as f32),
        ) {
            popover.set_parent(self.upcast_ref::<gtk::Widget>());
            popover.set_pointing_to(Some(&gtk::gdk::Rectangle::new(
                point.x().round() as i32,
                point.y().round() as i32,
                1,
                1,
            )));
        } else {
            popover.set_parent(anchor);
            popover.set_pointing_to(Some(&gtk::gdk::Rectangle::new(
                x.round() as i32,
                y.round() as i32,
                1,
                1,
            )));
        }
        self.set_active_context_popover(&popover);

        let win = self.clone();
        popover.connect_closed(move |p| {
            let popover = p.clone();
            if win
                .imp()
                .active_context_popover
                .borrow()
                .as_ref()
                .is_some_and(|active| active == &popover)
            {
                win.imp().active_context_popover.borrow_mut().take();
            }

            glib::idle_add_local_once(move || {
                if popover.parent().is_some() {
                    popover.unparent();
                }
            });
        });

        popover.popup();
    }

    pub(super) fn show_file_context_menu(&self, node: TreeNode, anchor: &gtk::Widget) {
        *self.imp().context_node.borrow_mut() = Some(node.clone());

        let is_file = !node.is_dir() && !node.is_submodule();

        let menu = gio::Menu::new();

        let open_section = gio::Menu::new();
        open_section.append(Some(&gettext("Open")), Some("ctx.context-open"));
        if !is_file {
            open_section.append(
                Some(&gettext("Open in New Tab")),
                Some("ctx.context-open-new-tab"),
            );
            open_section.append(
                Some(&gettext("Open in New Window")),
                Some("ctx.context-open-new-window"),
            );
        }
        if is_file {
            open_section.append(Some(&gettext("Preview")), Some("ctx.context-preview"));
        }
        open_section.append(Some(&gettext("Open With…")), Some("ctx.context-open-with"));
        menu.append_section(None, &open_section);

        let edit_section = gio::Menu::new();
        edit_section.append(Some(&gettext("Export File…")), Some("ctx.context-export"));
        edit_section.append(
            Some(&gettext("Copy Snapshot Path")),
            Some("ctx.context-copy-path"),
        );
        edit_section.append(
            Some(&gettext("Copy Content")),
            Some("ctx.context-copy-content"),
        );
        edit_section.append(
            Some(&gettext("Show in System")),
            Some("ctx.context-show-system"),
        );
        menu.append_section(None, &edit_section);

        let properties_section = gio::Menu::new();
        properties_section.append(Some(&gettext("Properties")), Some("ctx.context-properties"));
        menu.append_section(None, &properties_section);

        let group = gio::SimpleActionGroup::new();

        let open_action = gio::SimpleAction::new("context-open", None);
        {
            let win = self.clone();
            let node = node.clone();

            open_action.connect_activate(move |_, _| {
                win.close_active_context_popover();
                if node.is_dir() || node.is_submodule() {
                    win.push_dir(node.path().to_path_buf());
                } else {
                    win.open_snapshot_node_with_default_app(&node);
                }
            });
        }
        group.add_action(&open_action);

        let open_new_tab_action = gio::SimpleAction::new("context-open-new-tab", None);
        {
            let win = self.clone();
            let node = node.clone();

            open_new_tab_action.connect_activate(move |_, _| {
                win.close_active_context_popover();
                win.open_snapshot_node_in_new_tab(&node);
            });
        }
        group.add_action(&open_new_tab_action);

        let open_new_window_action = gio::SimpleAction::new("context-open-new-window", None);
        {
            let win = self.clone();
            let node = node.clone();

            open_new_window_action.connect_activate(move |_, _| {
                win.close_active_context_popover();
                win.open_snapshot_node_in_new_window(&node);
            });
        }
        group.add_action(&open_new_window_action);

        let open_with_action = gio::SimpleAction::new("context-open-with", None);
        {
            let win = self.clone();
            let node = node.clone();

            open_with_action.connect_activate(move |_, _| {
                win.close_active_context_popover();
                win.open_snapshot_node_with_app_chooser(&node);
            });
        }
        group.add_action(&open_with_action);

        let preview_action = gio::SimpleAction::new("context-preview", None);
        {
            let win = self.clone();
            let node = node.clone();

            preview_action.connect_activate(move |_, _| {
                win.close_active_context_popover();
                if node.is_dir() || node.is_submodule() {
                    win.open_snapshot_node_with_default_app(&node);
                } else {
                    win.preview_file(node.path());
                }
            });
        }
        group.add_action(&preview_action);

        let open_location_action = gio::SimpleAction::new("context-open-location", None);
        let has_parent = node
            .path()
            .parent()
            .is_some_and(|parent| parent != node.path());
        open_location_action.set_enabled(has_parent || is_file);
        {
            let win = self.clone();
            let node = node.clone();

            open_location_action.connect_activate(move |_, _| {
                win.close_active_context_popover();
                win.open_snapshot_node_location(&node);
            });
        }
        group.add_action(&open_location_action);

        let export_action = gio::SimpleAction::new("context-export", None);
        export_action.set_enabled(is_file);
        {
            let win = self.clone();
            let node = node.clone();

            export_action.connect_activate(move |_, _| {
                win.close_active_context_popover();
                win.export_snapshot_node(&node);
            });
        }
        group.add_action(&export_action);

        let copy_path_action = gio::SimpleAction::new("context-copy-path", None);
        {
            let win = self.clone();
            let node = node.clone();

            copy_path_action.connect_activate(move |_, _| {
                win.close_active_context_popover();
                win.copy_repository_path(&node);
            });
        }
        group.add_action(&copy_path_action);

        let copy_content_action = gio::SimpleAction::new("context-copy-content", None);
        copy_content_action.set_enabled(is_file);
        {
            let win = self.clone();
            let node = node.clone();

            copy_content_action.connect_activate(move |_, _| {
                win.close_active_context_popover();
                win.copy_snapshot_node_content(&node);
            });
        }
        group.add_action(&copy_content_action);

        let show_system_action = gio::SimpleAction::new("context-show-system", None);
        {
            let win = self.clone();
            let node = node.clone();

            show_system_action.connect_activate(move |_, _| {
                win.close_active_context_popover();
                win.show_node_in_system(&node);
            });
        }
        group.add_action(&show_system_action);

        let properties_action = gio::SimpleAction::new("context-properties", None);
        {
            let win = self.clone();
            let node = node.clone();

            properties_action.connect_activate(move |_, _| {
                win.close_active_context_popover();
                win.show_node_properties(&node);
            });
        }
        group.add_action(&properties_action);

        // Keep context actions in their own namespace.  Using "win" here made
        // GTK resolve identically named window actions instead of these actions.
        let popover = gtk::PopoverMenu::from_model(Some(&menu));
        popover.insert_action_group("ctx", Some(&group));
        popover.set_has_arrow(false);
        popover.set_halign(gtk::Align::Start);
        popover.set_position(gtk::PositionType::Bottom);
        popover.add_css_class("nautilus-context-menu");
        // Parent to the top-level window with a 1x1 pointing rectangle so GTK4
        // uses the full window height (and flips upward near the bottom) instead
        // of clipping/scrolling the menu inside the item row or scrolled view.
        let anchor_point = gtk::graphene::Point::new(
            (anchor.width() as f32 * 0.5).max(1.0),
            (anchor.height() as f32 * 0.5).max(1.0),
        );
        if let Some(point) = anchor.compute_point(self, &anchor_point) {
            popover.set_parent(self.upcast_ref::<gtk::Widget>());
            popover.set_pointing_to(Some(&gtk::gdk::Rectangle::new(
                point.x().round() as i32,
                point.y().round() as i32,
                1,
                1,
            )));
        } else {
            popover.set_parent(anchor);
        }
        self.set_active_context_popover(&popover);

        let win = self.clone();
        popover.connect_closed(move |p| {
            let popover = p.clone();
            if win
                .imp()
                .active_context_popover
                .borrow()
                .as_ref()
                .is_some_and(|active| active == &popover)
            {
                win.imp().active_context_popover.borrow_mut().take();
            }

            glib::idle_add_local_once(move || {
                if popover.parent().is_some() {
                    popover.unparent();
                }
            });
        });

        popover.popup();
    }

    pub(super) fn with_context_node<F>(&self, f: F)
    where
        F: FnOnce(&Self, TreeNode),
    {
        let node = self.imp().context_node.borrow().clone();

        if let Some(node) = node {
            f(self, node);
        }
    }

    pub(super) fn context_open_selected(&self) {
        self.with_context_node(|win, node| {
            if node.is_dir() || node.is_submodule() {
                win.push_dir(node.path().to_path_buf());
            } else {
                win.open_snapshot_node_with_default_app(&node);
            }
        });
    }

    pub(super) fn context_open_with_selected(&self) {
        self.with_context_node(|win, node| {
            win.open_snapshot_node_with_app_chooser(&node);
        });
    }

    pub(super) fn context_export_selected(&self) {
        self.with_context_node(|win, node| {
            win.export_snapshot_node(&node);
        });
    }

    pub(super) fn context_copy_path_selected(&self) {
        self.with_context_node(|win, node| {
            win.copy_repository_path(&node);
        });
    }

    pub(super) fn context_copy_content_selected(&self) {
        self.with_context_node(|win, node| {
            win.copy_snapshot_node_content(&node);
        });
    }

    pub(super) fn context_show_system_selected(&self) {
        self.with_context_node(|win, node| {
            win.show_node_in_system(&node);
        });
    }

    pub(super) fn context_properties_selected(&self) {
        self.with_context_node(|win, node| {
            win.show_node_properties(&node);
        });
    }

    pub(super) fn close_active_context_popover(&self) {
        if let Some(popover) = self.imp().active_context_popover.borrow_mut().take() {
            popover.popdown();
            if popover.parent().is_some() {
                popover.unparent();
            }
        }
    }

    pub(super) fn set_active_context_popover(&self, popover: &gtk::PopoverMenu) {
        self.close_active_context_popover();
        self.imp()
            .active_context_popover
            .replace(Some(popover.clone()));
    }
}
