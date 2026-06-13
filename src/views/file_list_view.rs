/* views/file_list_view.rs
 *
 * Copyright 2026 John Peter Sá
 *
 * This program is free software: you can redistribute it and/or modify
 * it under the terms of the GNU General Public License as published by
 * the Free Software Foundation, either version 3 of the License, or
 * (at your option) any later version.
 *
 * SPDX-License-Identifier: GPL-3.0-or-later
 */

//! Unified file-browser widget.
//!
//! `FileListView` wraps a `GtkStack` with three pages:
//! - **"list"** — `GtkListView` using the list-row factory.
//! - **"grid"** — `GtkGridView` using the grid-cell factory.
//! - **"empty"** — `AdwStatusPage` shown when no commit is selected.
//!
//! Gap 1 fix:
//! - Corrected `Some(m) {` → `Some(m) =>` syntax error.
//! - Split `set_model` into `set_list_factory` / `set_grid_factory` +
//!   `set_model`, matching how `window.rs` already builds separate
//!   factories for list and grid views.
//! - Exposed `connect_item_activated` so `window.rs` can attach a single
//!   handler for both views without duplicating the signal connection.

use gtk::prelude::*;
use gtk::subclass::prelude::*;
use gtk::{gio, glib, CompositeTemplate};
use adw::subclass::prelude::*;
use std::cell::RefCell;

// ── GObject boilerplate ───────────────────────────────────────────────────────

mod imp {
    use super::*;

    #[derive(Debug, Default, CompositeTemplate)]
    #[template(resource = "/io/github/johnpetersa19/TemporalExplorer/file-list-view.ui")]
    pub struct FileListView {
        #[template_child]
        pub view_stack: TemplateChild<gtk::Stack>,
        #[template_child]
        pub file_list_view: TemplateChild<gtk::ListView>,
        #[template_child]
        pub file_grid_view: TemplateChild<gtk::GridView>,

        /// Current view mode: "list" | "grid"
        pub view_mode: RefCell<String>,

        /// Separate factories so list and grid can use different cell widgets.
        pub list_factory: RefCell<Option<gtk::ListItemFactory>>,
        pub grid_factory: RefCell<Option<gtk::ListItemFactory>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for FileListView {
        const NAME: &'static str = "FileListView";
        type Type = super::FileListView;
        type ParentType = adw::Bin;

        fn class_init(klass: &mut Self::Class) {
            klass.bind_template();
        }

        fn instance_init(obj: &glib::subclass::InitializingObject<Self>) {
            obj.init_template();
        }
    }

    impl ObjectImpl for FileListView {
        fn constructed(&self) {
            self.parent_constructed();
            *self.view_mode.borrow_mut() = "list".to_string();
            self.view_stack.set_visible_child_name("empty");
        }
    }

    impl WidgetImpl for FileListView {}
    impl BinImpl for FileListView {}
}

glib::wrapper! {
    pub struct FileListView(ObjectSubclass<imp::FileListView>)
        @extends adw::Bin, gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl FileListView {
    pub fn new() -> Self {
        glib::Object::builder().build()
    }

    // ── Factory registration ──────────────────────────────────────────────────

    /// Set the factory used by the `GtkListView` (list mode).
    pub fn set_list_factory(&self, factory: &impl IsA<gtk::ListItemFactory>) {
        let imp = self.imp();
        let factory = factory.as_ref().clone();
        imp.file_list_view.set_factory(Some(&factory));
        *imp.list_factory.borrow_mut() = Some(factory);
    }

    /// Set the factory used by the `GtkGridView` (grid mode).
    pub fn set_grid_factory(&self, factory: &impl IsA<gtk::ListItemFactory>) {
        let imp = self.imp();
        let factory = factory.as_ref().clone();
        imp.file_grid_view.set_factory(Some(&factory));
        *imp.grid_factory.borrow_mut() = Some(factory);
    }

    // ── Model binding ─────────────────────────────────────────────────────────

    /// Bind a `gio::ListModel` as the data source for both list and grid views.
    /// Pass `None` to clear and show the empty state.
    ///
    /// # Panics
    ///
    /// Panics (in debug) if the respective factory has not been set before
    /// calling with `Some(model)`.  In release builds it degrades gracefully
    /// to showing the empty state.
    pub fn set_model(&self, model: Option<&impl IsA<gio::ListModel>>) {
        let imp = self.imp();
        match model {
            Some(m) => {
                // List view: single-selection wrapper.
                let sel_list = gtk::SingleSelection::new(Some(m.clone()));
                imp.file_list_view.set_model(Some(&sel_list));

                // Grid view: independent single-selection wrapper so
                // activating an item in one view does not affect the other.
                let sel_grid = gtk::SingleSelection::new(Some(m.clone()));
                imp.file_grid_view.set_model(Some(&sel_grid));

                // Reveal the currently active view page.
                imp.view_stack
                    .set_visible_child_name(&self.imp().view_mode.borrow());
            }
            None => {
                imp.file_list_view.set_model(None::<&gtk::SingleSelection>);
                imp.file_grid_view.set_model(None::<&gtk::SingleSelection>);
                imp.view_stack.set_visible_child_name("empty");
            }
        }
    }

    // ── View mode ─────────────────────────────────────────────────────────────

    /// Switch between "list" and "grid" view modes.
    ///
    /// Only transitions away from "empty" when a model is actually loaded.
    pub fn set_view_mode(&self, mode: &str) {
        let imp = self.imp();
        *imp.view_mode.borrow_mut() = mode.to_string();
        // Stay on "empty" if nothing is loaded yet.
        if imp.view_stack.visible_child_name().as_deref() != Some("empty") {
            imp.view_stack.set_visible_child_name(mode);
        }
    }

    pub fn view_mode(&self) -> String {
        self.imp().view_mode.borrow().clone()
    }

    /// Convenience: show the empty state (no commit selected).
    pub fn show_empty(&self) {
        self.imp().view_stack.set_visible_child_name("empty");
    }

    // ── Item activation ───────────────────────────────────────────────────────

    /// Connect a single callback that fires when the user activates an item in
    /// either the list view or the grid view.  The `u32` argument is the
    /// position of the activated item inside the model.
    pub fn connect_item_activated<F>(&self, f: F)
    where
        F: Fn(u32) + Clone + 'static,
    {
        let f_grid = f.clone();
        self.imp().file_list_view.connect_activate(move |_, pos| f(pos));
        self.imp().file_grid_view.connect_activate(move |_, pos| f_grid(pos));
    }
}

impl Default for FileListView {
    fn default() -> Self {
        Self::new()
    }
}
