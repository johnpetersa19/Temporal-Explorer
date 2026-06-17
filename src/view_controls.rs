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

use gettextrs::gettext;
use gtk::glib;
use gtk::prelude::*;
use gtk::subclass::prelude::*;
use std::cell::Cell;
use std::sync::OnceLock;

// ── FileSortMode ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub enum FileSortMode {
    #[default]
    Name,
    NameDescending,
    Status,
    Extension,
}

// ── GObject subclass ──────────────────────────────────────────────────────────

mod imp {
    use super::*;

    #[derive(Debug, Default, gtk::CompositeTemplate)]
    #[template(resource = "/io/github/johnpetersa19/TemporalExplorer/view-controls.ui")]
    pub struct ViewControls {
        #[template_child] pub grid_view_button:       TemplateChild<gtk::ToggleButton>,
        #[template_child] pub list_view_button:       TemplateChild<gtk::ToggleButton>,
        #[template_child] pub view_options_label:     TemplateChild<gtk::Label>,
        #[template_child] pub zoom_out_button:        TemplateChild<gtk::Button>,
        #[template_child] pub zoom_in_button:         TemplateChild<gtk::Button>,
        #[template_child] pub captions_button:        TemplateChild<gtk::Button>,
        #[template_child] pub sort_name_button:       TemplateChild<gtk::CheckButton>,
        #[template_child] pub sort_name_desc_button:  TemplateChild<gtk::CheckButton>,
        #[template_child] pub sort_type_button:       TemplateChild<gtk::CheckButton>,

        pub zoom_level: Cell<u32>,
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
            // 0 = small, 1 = normal, 2 = large.
            self.zoom_level.set(1);
            self.update_zoom_sensitivity();
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
                    glib::subclass::Signal::builder("captions-requested")
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
        fn on_sort_name_toggled(&self) {
            if self.sort_name_button.get().is_active() {
                self.view_options_label.get().set_label(&gettext("Name"));
                self.obj().emit_by_name::<()>("sort-changed", &[&0u32]);
            }
        }

        #[template_callback]
        fn on_sort_name_desc_toggled(&self) {
            if self.sort_name_desc_button.get().is_active() {
                self.view_options_label.get().set_label(&gettext("Z-A"));
                self.obj().emit_by_name::<()>("sort-changed", &[&1u32]);
            }
        }

        #[template_callback]
        fn on_sort_type_toggled(&self) {
            if self.sort_type_button.get().is_active() {
                self.view_options_label.get().set_label(&gettext("Type"));
                self.obj().emit_by_name::<()>("sort-changed", &[&2u32]);
            }
        }

        #[template_callback]
        fn on_zoom_out_clicked(&self) {
            let current = self.zoom_level.get();
            if current > 0 {
                let next = current - 1;
                self.zoom_level.set(next);
                self.update_zoom_sensitivity();
                self.obj().emit_by_name::<()>("zoom-changed", &[&next]);
            }
        }

        #[template_callback]
        fn on_zoom_in_clicked(&self) {
            let current = self.zoom_level.get();
            if current < 2 {
                let next = current + 1;
                self.zoom_level.set(next);
                self.update_zoom_sensitivity();
                self.obj().emit_by_name::<()>("zoom-changed", &[&next]);
            }
        }

        #[template_callback]
        fn on_captions_clicked(&self) {
            self.obj().emit_by_name::<()>("captions-requested", &[]);
        }

        fn update_zoom_sensitivity(&self) {
            let current = self.zoom_level.get();
            self.zoom_out_button.get().set_sensitive(current > 0);
            self.zoom_in_button.get().set_sensitive(current < 2);
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
