/* view_controls.rs
 *
 * Copyright 2026 John Peter Sá
 *
 * SPDX-License-Identifier: GPL-3.0-or-later
 */

use gettextrs::gettext;
use gtk::{gio, glib};
use gtk::prelude::*;
use gtk::subclass::prelude::*;
use std::cell::{Cell, RefCell};
use std::sync::OnceLock;

// ── FileSortMode ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub enum FileSortMode {
    #[default]
    Name,
    NameDescending,
    LastModified,
    FirstModified,
    Size,
    Status,
    Extension,
}

// ── GObject subclass ──────────────────────────────────────────────────────────

mod imp {
    use super::*;

    #[derive(Debug, Default, gtk::CompositeTemplate)]
    #[template(resource = "/io/github/johnpetersa19/TemporalExplorer/view-controls.ui")]
    pub struct ViewControls {
        #[template_child] pub grid_view_button:   TemplateChild<gtk::ToggleButton>,
        #[template_child] pub list_view_button:   TemplateChild<gtk::ToggleButton>,
        #[template_child] pub view_options_label: TemplateChild<gtk::Label>,
        #[template_child] pub zoom_out_button:    TemplateChild<gtk::Button>,
        #[template_child] pub zoom_in_button:     TemplateChild<gtk::Button>,

        pub zoom_level: Cell<u32>,
        pub sort_action: RefCell<Option<gio::SimpleAction>>,
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

            self.obj().setup_menu_actions();
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

    fn setup_menu_actions(&self) {
        let group = gio::SimpleActionGroup::new();

        let sort_action = gio::SimpleAction::new_stateful(
            "sort",
            Some(&String::static_variant_type()),
            &"name".to_variant(),
        );

        {
            let obj = self.clone();
            sort_action.connect_activate(move |action, param| {
                let Some(param) = param else {
                    return;
                };

                let key: String = param.get().unwrap_or_else(|| "name".to_string());

                action.set_state(&key.to_variant());
                obj.set_sort_label_for_key(&key);

                let raw = match key.as_str() {
                    "name-desc" => 1u32,
                    "last-modified" => 2u32,
                    "first-modified" => 3u32,
                    "size" => 4u32,
                    "type" => 5u32,
                    _ => 0u32,
                };

                obj.emit_by_name::<()>("sort-changed", &[&raw]);
            });
        }

        group.add_action(&sort_action);
        self.imp().sort_action.replace(Some(sort_action));

        let captions_action = gio::SimpleAction::new("captions", None);
        {
            let obj = self.clone();
            captions_action.connect_activate(move |_, _| {
                obj.emit_by_name::<()>("captions-requested", &[]);
            });
        }
        group.add_action(&captions_action);

        let hidden_action = gio::SimpleAction::new("show-hidden-files", None);
        hidden_action.set_enabled(false);
        group.add_action(&hidden_action);

        self.insert_action_group("viewctrl", Some(&group));
    }

    /// Sync toggle button states.
    pub fn set_view_mode(&self, is_grid: bool) {
        let imp = self.imp();
        imp.grid_view_button.get().set_active(is_grid);
        imp.list_view_button.get().set_active(!is_grid);
    }

    /// Restore zoom button state from saved settings.
    pub fn set_zoom_level(&self, level: u32) {
        let imp = self.imp();
        let level = level.min(2);
        imp.zoom_level.set(level);
        imp.zoom_out_button.get().set_sensitive(level > 0);
        imp.zoom_in_button.get().set_sensitive(level < 2);
    }

    /// Restore the selected sort option from saved settings.
    pub fn set_sort_mode(&self, mode: FileSortMode) {
        let key = match mode {
            FileSortMode::Name | FileSortMode::Status => "name",
            FileSortMode::NameDescending => "name-desc",
            FileSortMode::LastModified => "last-modified",
            FileSortMode::FirstModified => "first-modified",
            FileSortMode::Size => "size",
            FileSortMode::Extension => "type",
        };

        self.set_sort_label_for_key(key);

        if let Some(action) = self.imp().sort_action.borrow().as_ref() {
            action.set_state(&key.to_variant());
        }
    }

    fn set_sort_label_for_key(&self, key: &str) {
        let label = match key {
            "name-desc" => gettext("Z-A"),
            "last-modified" => gettext("Last Modified"),
            "first-modified" => gettext("First Modified"),
            "size" => gettext("Size"),
            "type" => gettext("Type"),
            _ => gettext("Name"),
        };

        self.imp().view_options_label.get().set_label(&label);
    }
}
