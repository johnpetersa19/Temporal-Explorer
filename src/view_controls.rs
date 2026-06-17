/* view_controls.rs
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

use gtk::glib;
use gtk::prelude::{ObjectExt, StaticType, ToggleButtonExt};
use gtk::subclass::prelude::*;
use std::sync::OnceLock;

// ── FileSortMode ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub enum FileSortMode {
    #[default] Name,
    Status,
    Extension,
}

// ── GObject subclass ──────────────────────────────────────────────────────────

mod imp {
    use super::*;

    #[derive(Debug, Default, gtk::CompositeTemplate)]
    #[template(resource = "/io/github/johnpetersa19/TemporalExplorer/view-controls.ui")]
    pub struct ViewControls {
        #[template_child] pub grid_view_button: TemplateChild<gtk::ToggleButton>,
        #[template_child] pub list_view_button: TemplateChild<gtk::ToggleButton>,
        #[template_child] pub sort_dropdown:    TemplateChild<gtk::DropDown>,
        #[template_child] pub zoom_dropdown:    TemplateChild<gtk::DropDown>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for ViewControls {
        const NAME: &'static str = "ViewControls";
        type Type = super::ViewControls;
        type ParentType = gtk::Box;

        fn class_init(klass: &mut Self::Class) {
            klass.bind_template();
            klass.bind_template_callbacks();
        }

        fn instance_init(obj: &glib::subclass::InitializingObject<Self>) {
            obj.init_template();
        }
    }

    impl ObjectImpl for ViewControls {
        fn constructed(&self) {
            self.parent_constructed();
        }

        fn signals() -> &'static [glib::subclass::Signal] {
            static SIGNALS: OnceLock<Vec<glib::subclass::Signal>> = OnceLock::new();
            SIGNALS.get_or_init(|| {
                vec![
                    glib::subclass::Signal::builder("view-mode-changed")
                        .param_types([bool::static_type()])
                        .build(),
                    glib::subclass::Signal::builder("sort-changed")
                        .param_types([u32::static_type()])
                        .build(),
                    glib::subclass::Signal::builder("zoom-changed")
                        .param_types([u32::static_type()])
                        .build(),
                ]
            })
        }
    }

    impl WidgetImpl for ViewControls {}
    impl BoxImpl   for ViewControls {}

    #[gtk::template_callbacks]
    impl ViewControls {
        #[template_callback]
        fn on_list_toggled(&self) {
            if self.list_view_button.get().is_active() {
                self.obj().emit_by_name::<()>("view-mode-changed", &[&false]);
            }
        }

        #[template_callback]
        fn on_grid_toggled(&self) {
            if self.grid_view_button.get().is_active() {
                self.obj().emit_by_name::<()>("view-mode-changed", &[&true]);
            }
        }

        #[template_callback]
        fn on_sort_changed(&self, _pspec: &glib::ParamSpec) {
            let selected = self.sort_dropdown.get().selected();
            self.obj().emit_by_name::<()>("sort-changed", &[&selected]);
        }

        #[template_callback]
        fn on_zoom_changed(&self, _pspec: &glib::ParamSpec) {
            let selected = self.zoom_dropdown.get().selected();
            self.obj().emit_by_name::<()>("zoom-changed", &[&selected]);
        }
    }
}

// ── Public wrapper ────────────────────────────────────────────────────────────

glib::wrapper! {
    pub struct ViewControls(ObjectSubclass<imp::ViewControls>)
        @extends gtk::Box, gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget,
                    gtk::Orientable;
}

impl Default for ViewControls {
    fn default() -> Self { Self::new() }
}

impl ViewControls {
    pub fn new() -> Self {
        glib::Object::new()
    }

    /// Sync toggle button states without emitting signals.
    pub fn set_view_mode(&self, is_grid: bool) {
        let imp = self.imp();
        imp.grid_view_button.get().set_active(is_grid);
        imp.list_view_button.get().set_active(!is_grid);
    }
}
