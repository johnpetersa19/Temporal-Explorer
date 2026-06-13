/* column_chooser.rs
 *
 * Copyright 2026 John Peter Sá
 *
 * SPDX-License-Identifier: GPL-3.0-or-later
 */

//! ColumnChooser — AdwDialog for toggling visible columns in list view.
//!
//! Emits:
//!   - `"columns-changed"` (no args) — user pressed Apply

use adw::prelude::*;
use adw::subclass::prelude::*;
use gtk::{glib, glib::subclass::Signal};
use std::sync::OnceLock;

// ── Column visibility bitflags ─────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ColumnVisibility {
    pub name:      bool,
    pub status:    bool,
    pub size:      bool,
    pub extension: bool,
}

impl Default for ColumnVisibility {
    fn default() -> Self {
        Self { name: true, status: true, size: false, extension: false }
    }
}

// ── Private implementation ─────────────────────────────────────────────────────

mod imp {
    use super::*;

    #[derive(Debug, Default, gtk::CompositeTemplate)]
    #[template(resource = "/io/github/johnpetersa19/TemporalExplorer/column-chooser.ui")]
    pub struct ColumnChooser {
        #[template_child] pub col_name_row:   gtk::TemplateChild<adw::SwitchRow>,
        #[template_child] pub col_status_row: gtk::TemplateChild<adw::SwitchRow>,
        #[template_child] pub col_size_row:   gtk::TemplateChild<adw::SwitchRow>,
        #[template_child] pub col_ext_row:    gtk::TemplateChild<adw::SwitchRow>,
        #[template_child] pub apply_button:   gtk::TemplateChild<gtk::Button>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for ColumnChooser {
        const NAME: &'static str = "ColumnChooser";
        type Type = super::ColumnChooser;
        type ParentType = adw::Dialog;

        fn class_init(klass: &mut Self::Class) {
            klass.bind_template();
        }
        fn instance_init(obj: &glib::subclass::InitializingObject<Self>) {
            obj.init_template();
        }
    }

    impl ObjectImpl for ColumnChooser {
        fn signals() -> &'static [Signal] {
            static SIGNALS: OnceLock<Vec<Signal>> = OnceLock::new();
            SIGNALS.get_or_init(|| {
                vec![Signal::builder("columns-changed").build()]
            })
        }

        fn constructed(&self) {
            self.parent_constructed();
            let obj = self.obj().clone();
            self.apply_button.connect_clicked(move |_| {
                obj.emit_by_name::<()>("columns-changed", &[]);
                obj.close();
            });
        }
    }

    impl WidgetImpl for ColumnChooser {}
    impl AdwDialogImpl for ColumnChooser {}
}

// ── Public wrapper ─────────────────────────────────────────────────────────────

glib::wrapper! {
    pub struct ColumnChooser(ObjectSubclass<imp::ColumnChooser>)
        @extends adw::Dialog, gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl ColumnChooser {
    pub fn new() -> Self {
        glib::Object::new()
    }

    /// Read current column visibility state from the switches.
    pub fn visibility(&self) -> ColumnVisibility {
        let imp = self.imp();
        ColumnVisibility {
            name:      imp.col_name_row.is_active(),
            status:    imp.col_status_row.is_active(),
            size:      imp.col_size_row.is_active(),
            extension: imp.col_ext_row.is_active(),
        }
    }

    /// Apply a visibility state to the switches (without emitting signal).
    pub fn apply_visibility(&self, vis: &ColumnVisibility) {
        let imp = self.imp();
        imp.col_name_row.set_active(vis.name);
        imp.col_status_row.set_active(vis.status);
        imp.col_size_row.set_active(vis.size);
        imp.col_ext_row.set_active(vis.extension);
    }
}

impl Default for ColumnChooser {
    fn default() -> Self { Self::new() }
}
