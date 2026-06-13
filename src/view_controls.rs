/* view_controls.rs
 *
 * Copyright 2026 John Peter Sá
 *
 * SPDX-License-Identifier: GPL-3.0-or-later
 */

//! ViewControls — list/grid toggle + sort selector.
//!
//! Emits:
//!   - `"view-mode-changed"` (bool is_grid) — user toggled view
//!   - `"sort-changed"`      (u32 FileSortMode) — user changed sort

use adw::subclass::prelude::*;
use gtk::{glib, glib::subclass::Signal};
use gtk::prelude::{ObjectExt, StaticType, ToggleButtonExt, CheckButtonExt};
use std::sync::OnceLock;

#[derive(Debug, Clone, Copy, Default, PartialEq, glib::Enum)]
#[enum_type(name = "FileSortMode")]
pub enum FileSortMode {
    #[default]
    Name,
    Status,
    Extension,
}

// ── Private implementation ─────────────────────────────────────────────────────

mod imp {
    use super::*;

    #[derive(Debug, Default, gtk::CompositeTemplate)]
    #[template(resource = "/io/github/johnpetersa19/TemporalExplorer/view-controls.ui")]
    pub struct ViewControls {
        #[template_child] pub list_button:      gtk::TemplateChild<gtk::ToggleButton>,
        #[template_child] pub grid_button:      gtk::TemplateChild<gtk::ToggleButton>,
        #[template_child] pub sort_menu_button: gtk::TemplateChild<gtk::MenuButton>,
        #[template_child] pub sort_popover:     gtk::TemplateChild<gtk::Popover>,
        #[template_child] pub sort_by_name:     gtk::TemplateChild<gtk::CheckButton>,
        #[template_child] pub sort_by_status:   gtk::TemplateChild<gtk::CheckButton>,
        #[template_child] pub sort_by_ext:      gtk::TemplateChild<gtk::CheckButton>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for ViewControls {
        const NAME: &'static str = "ViewControls";
        type Type = super::ViewControls;
        type ParentType = gtk::Box;

        fn class_init(klass: &mut Self::Class) {
            klass.bind_template();
        }
        fn instance_init(obj: &glib::subclass::InitializingObject<Self>) {
            obj.init_template();
        }
    }

    impl ObjectImpl for ViewControls {
        fn signals() -> &'static [Signal] {
            static SIGNALS: OnceLock<Vec<Signal>> = OnceLock::new();
            SIGNALS.get_or_init(|| {
                vec![
                    Signal::builder("view-mode-changed")
                        .param_types([bool::static_type()])
                        .build(),
                    Signal::builder("sort-changed")
                        .param_types([u32::static_type()])
                        .build(),
                ]
            })
        }

        fn constructed(&self) {
            self.parent_constructed();

            // Grid toggle
            let obj = self.obj().clone();
            self.grid_button.connect_toggled(move |btn| {
                if btn.is_active() {
                    obj.emit_by_name::<()>("view-mode-changed", &[&true]);
                }
            });
            let obj = self.obj().clone();
            self.list_button.connect_toggled(move |btn| {
                if btn.is_active() {
                    obj.emit_by_name::<()>("view-mode-changed", &[&false]);
                }
            });

            // Sort buttons
            let obj = self.obj().clone();
            self.sort_by_name.connect_toggled(move |btn| {
                if btn.is_active() {
                    obj.emit_by_name::<()>("sort-changed", &[&(FileSortMode::Name as u32)]);
                }
            });
            let obj = self.obj().clone();
            self.sort_by_status.connect_toggled(move |btn| {
                if btn.is_active() {
                    obj.emit_by_name::<()>("sort-changed", &[&(FileSortMode::Status as u32)]);
                }
            });
            let obj = self.obj().clone();
            self.sort_by_ext.connect_toggled(move |btn| {
                if btn.is_active() {
                    obj.emit_by_name::<()>("sort-changed", &[&(FileSortMode::Extension as u32)]);
                }
            });
        }
    }

    impl WidgetImpl for ViewControls {}
    impl BoxImpl for ViewControls {}
}

// ── Public wrapper ─────────────────────────────────────────────────────────────

glib::wrapper! {
    pub struct ViewControls(ObjectSubclass<imp::ViewControls>)
        @extends gtk::Box, gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget,
                    gtk::Orientable;
}

impl ViewControls {
    pub fn new() -> Self {
        glib::Object::new()
    }

    /// Programmatically set active view without emitting the signal.
    pub fn set_grid_active(&self, is_grid: bool) {
        let imp = self.imp();
        imp.grid_button.set_active(is_grid);
        imp.list_button.set_active(!is_grid);
    }
}

impl Default for ViewControls {
    fn default() -> Self { Self::new() }
}
