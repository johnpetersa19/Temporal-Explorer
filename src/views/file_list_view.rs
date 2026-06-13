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

use gtk::prelude::*;
use gtk::subclass::prelude::*;
use gtk::{gio, glib, CompositeTemplate};
use adw::subclass::prelude::*;

// ── GObject boilerplate ───────────────────────────────────────────────────────

mod imp {
    use super::*;
    use std::cell::RefCell;

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

    /// Switch between "list" and "grid" view modes.
    pub fn set_view_mode(&self, mode: &str) {
        let imp = self.imp();
        *imp.view_mode.borrow_mut() = mode.to_string();
        // Only switch if files are loaded; otherwise stay on "empty".
        if imp.view_stack.visible_child_name().as_deref() != Some("empty") {
            imp.view_stack.set_visible_child_name(mode);
        }
    }

    pub fn view_mode(&self) -> String {
        self.imp().view_mode.borrow().clone()
    }

    /// Bind a `gio::ListModel` as the data source for both list and grid views.
    /// Pass `None` to clear and show the empty state.
    pub fn set_model(
        &self,
        model: Option<&impl IsA<gio::ListModel>>,
        factory: &impl IsA<gtk::ListItemFactory>,
    ) {
        let imp = self.imp();
        match model {
            Some(m) {
                let sel = gtk::SingleSelection::new(Some(m.clone()));
                imp.file_list_view.set_model(Some(&sel));
                imp.file_list_view.set_factory(Some(factory));

                // Grid uses the same model — build a new selection wrapper.
                let sel_grid = gtk::SingleSelection::new(Some(m.clone()));
                imp.file_grid_view.set_model(Some(&sel_grid));
                imp.file_grid_view.set_factory(Some(factory));

                imp.view_stack
                    .set_visible_child_name(&self.imp().view_mode.borrow());
            }
            None => {
                imp.view_stack.set_visible_child_name("empty");
            }
        }
    }

    /// Convenience: show the empty state (no commit selected).
    pub fn show_empty(&self) {
        self.imp().view_stack.set_visible_child_name("empty");
    }
}

impl Default for FileListView {
    fn default() -> Self {
        Self::new()
    }
}
