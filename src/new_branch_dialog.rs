/* new_branch_dialog.rs
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

//! `NewBranchDialog` — a simple `AdwDialog` that lets the user type a
//! new branch name and emits `::branch-created(name: &str)` when
//! confirmed, or `::cancelled` when dismissed.
//!
//! Port of the Nautilus `NautilusNewFolderDialog` pattern.

use adw::prelude::*;
use adw::subclass::prelude::*;
use gtk::{glib, CompositeTemplate};
use std::cell::RefCell;

// ── GObject implementation ─────────────────────────────────────────────────────

mod imp {
    use super::*;

    #[derive(Debug, Default, CompositeTemplate)]
    #[template(resource = "/io/github/johnpetersa19/TemporalExplorer/new-branch-dialog.ui")]
    pub struct NewBranchDialog {
        #[template_child]
        pub branch_name_entry: TemplateChild<adw::EntryRow>,
        #[template_child]
        pub cancel_button: TemplateChild<gtk::Button>,
        #[template_child]
        pub create_button: TemplateChild<gtk::Button>,

        /// Cached branch name so signal handlers can borrow it.
        pub branch_name: RefCell<String>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for NewBranchDialog {
        const NAME: &'static str = "NewBranchDialog";
        type Type = super::NewBranchDialog;
        type ParentType = adw::Dialog;

        fn class_init(klass: &mut Self::Class) {
            klass.bind_template();
            klass.bind_template_callbacks();
        }

        fn instance_init(obj: &glib::subclass::InitializingObject<Self>) {
            obj.init_template();
        }
    }

    #[gtk::template_callbacks]
    impl NewBranchDialog {
        /// Keep `create_button` sensitive only when entry is non-empty.
        #[template_callback]
        fn on_entry_changed(&self, entry: &adw::EntryRow) {
            let text = entry.text();
            let non_empty = !text.trim().is_empty();
            self.create_button.set_sensitive(non_empty);
            *self.branch_name.borrow_mut() = text.trim().to_string();
        }
    }

    impl ObjectImpl for NewBranchDialog {
        fn constructed(&self) {
            self.parent_constructed();

            // Wire entry ::changed → sensitivity callback.
            let obj_weak = self.obj().downgrade();
            self.branch_name_entry.connect_changed(move |entry| {
                if let Some(obj) = obj_weak.upgrade() {
                    obj.imp().on_entry_changed(entry);
                }
            });

            // Cancel button: close and emit ::cancelled.
            let obj_weak = self.obj().downgrade();
            self.cancel_button.connect_clicked(move |_| {
                if let Some(obj) = obj_weak.upgrade() {
                    obj.close();
                    obj.emit_by_name::<()>("cancelled", &[]);
                }
            });

            // Create button: close and emit ::branch-created(name).
            let obj_weak = self.obj().downgrade();
            self.create_button.connect_clicked(move |_| {
                if let Some(obj) = obj_weak.upgrade() {
                    let name = obj.imp().branch_name.borrow().clone();
                    if name.is_empty() {
                        return;
                    }
                    obj.close();
                    obj.emit_by_name::<()>("branch-created", &[&name]);
                }
            });

            // Allow pressing Enter in the entry to trigger Create.
            let obj_weak = self.obj().downgrade();
            self.branch_name_entry.connect_entry_activated(move |_| {
                if let Some(obj) = obj_weak.upgrade() {
                    let name = obj.imp().branch_name.borrow().clone();
                    if !name.is_empty() {
                        obj.close();
                        obj.emit_by_name::<()>("branch-created", &[&name]);
                    }
                }
            });
        }

        fn signals() -> &'static [glib::subclass::Signal] {
            use std::sync::OnceLock;
            static SIGNALS: OnceLock<Vec<glib::subclass::Signal>> = OnceLock::new();
            SIGNALS.get_or_init(|| {
                vec![
                    glib::subclass::Signal::builder("branch-created")
                        .param_types([String::static_type()])
                        .build(),
                    glib::subclass::Signal::builder("cancelled").build(),
                ]
            })
        }
    }

    impl WidgetImpl for NewBranchDialog {}
    impl AdwDialogImpl for NewBranchDialog {}
}

// ── Public wrapper ─────────────────────────────────────────────────────────────

glib::wrapper! {
    pub struct NewBranchDialog(ObjectSubclass<imp::NewBranchDialog>)
        @extends adw::Dialog, gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl NewBranchDialog {
    pub fn new() -> Self {
        glib::Object::builder().build()
    }

    /// Connect a callback that fires when the user confirms a branch name.
    ///
    /// ```ignore
    /// dialog.connect_branch_created(|_dialog, name| {
    ///     window.create_branch(name);
    /// });
    /// ```
    pub fn connect_branch_created<F>(&self, f: F) -> glib::SignalHandlerId
    where
        F: Fn(&Self, &str) + 'static,
    {
        self.connect_local("branch-created", false, move |args| {
            let dialog = args[0].get::<NewBranchDialog>().unwrap();
            let name = args[1].get::<String>().unwrap_or_default();
            f(&dialog, &name);
            None
        })
    }

    pub fn connect_cancelled<F>(&self, f: F) -> glib::SignalHandlerId
    where
        F: Fn(&Self) + 'static,
    {
        self.connect_local("cancelled", false, move |args| {
            let dialog = args[0].get::<NewBranchDialog>().unwrap();
            f(&dialog);
            None
        })
    }
}

impl Default for NewBranchDialog {
    fn default() -> Self {
        Self::new()
    }
}
