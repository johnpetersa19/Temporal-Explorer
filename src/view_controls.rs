/* view_controls.rs
 *
 * Copyright 2026 John Peter Sá
 *
 * SPDX-License-Identifier: GPL-3.0-or-later
 */

use adw::prelude::*;
use gettextrs::gettext;
use gtk::subclass::prelude::*;
use gtk::{gio, glib};
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
        #[template_child]
        pub view_split_button: TemplateChild<adw::SplitButton>,
        #[template_child]
        pub zoom_out_button: TemplateChild<gtk::Button>,
        #[template_child]
        pub zoom_in_button: TemplateChild<gtk::Button>,

        pub zoom_level: Cell<u32>,
        pub is_grid: Cell<bool>,
        pub sort_action: RefCell<Option<gio::SimpleAction>>,
        pub hidden_action: RefCell<Option<gio::SimpleAction>>,
        pub visible_columns_action: RefCell<Option<gio::SimpleAction>>,
        pub captions_action: RefCell<Option<gio::SimpleAction>>,
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
            self.is_grid.set(false);
            self.update_zoom_sensitivity();
            self.update_view_split_button();

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
                    glib::subclass::Signal::builder("hidden-files-changed")
                        .param_types([bool::static_type()])
                        .build(),
                    glib::subclass::Signal::builder("visible-columns-requested").build(),
                    glib::subclass::Signal::builder("captions-requested").build(),
                ]
            })
        }
    }

    impl WidgetImpl for ViewControls {}
    impl BoxImpl for ViewControls {}

    #[gtk::template_callbacks]
    impl ViewControls {
        #[template_callback]
        fn on_view_split_clicked(&self) {
            let next_is_grid = !self.is_grid.get();

            self.is_grid.set(next_is_grid);
            self.update_view_split_button();

            self.obj()
                .emit_by_name::<()>("view-mode-changed", &[&next_is_grid]);
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

        pub(super) fn update_zoom_sensitivity(&self) {
            let current = self.zoom_level.get();
            let zoom_available = self.is_grid.get();

            self.zoom_out_button
                .get()
                .set_sensitive(zoom_available && current > 0);
            self.zoom_in_button
                .get()
                .set_sensitive(zoom_available && current < 2);
        }

        fn update_view_split_button(&self) {
            let button = self.view_split_button.get();

            if self.is_grid.get() {
                // Current view is grid, so the main button offers list view.
                button.set_icon_name("view-list-symbolic");
                button.set_tooltip_text(Some(&gettext("List view")));
            } else {
                // Current view is list, so the main button offers grid view.
                button.set_icon_name("view-grid-symbolic");
                button.set_tooltip_text(Some(&gettext("Grid view")));
            }

            if let Some(action) = self.visible_columns_action.borrow().as_ref() {
                action.set_enabled(!self.is_grid.get());
            }
            if let Some(action) = self.captions_action.borrow().as_ref() {
                action.set_enabled(self.is_grid.get());
            }
            self.update_zoom_sensitivity();
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
    fn default() -> Self {
        Self::new()
    }
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

        let visible_columns_action = gio::SimpleAction::new("visible-columns", None);
        {
            let obj = self.clone();

            visible_columns_action.connect_activate(move |_, _| {
                obj.emit_by_name::<()>("visible-columns-requested", &[]);
            });
        }
        group.add_action(&visible_columns_action);
        self.imp()
            .visible_columns_action
            .replace(Some(visible_columns_action));

        let captions_action = gio::SimpleAction::new("captions", None);
        {
            let obj = self.clone();

            captions_action.connect_activate(move |_, _| {
                obj.emit_by_name::<()>("captions-requested", &[]);
            });
        }
        group.add_action(&captions_action);
        self.imp().captions_action.replace(Some(captions_action));

        let hidden_action =
            gio::SimpleAction::new_stateful("show-hidden-files", None, &false.to_variant());
        {
            let obj = self.clone();

            hidden_action.connect_activate(move |action, _| {
                let current = action
                    .state()
                    .and_then(|state| state.get::<bool>())
                    .unwrap_or(false);
                let next = !current;

                action.set_state(&next.to_variant());
                obj.emit_by_name::<()>("hidden-files-changed", &[&next]);
            });
        }
        group.add_action(&hidden_action);
        self.imp().hidden_action.replace(Some(hidden_action));

        self.insert_action_group("viewctrl", Some(&group));
        self.set_view_mode(self.imp().is_grid.get());
    }

    /// Sync view mode from window.rs.
    pub fn set_view_mode(&self, is_grid: bool) {
        let imp = self.imp();

        imp.is_grid.set(is_grid);

        let button = imp.view_split_button.get();

        if is_grid {
            // Current view is grid, so the main button offers list view.
            button.set_icon_name("view-list-symbolic");
            button.set_tooltip_text(Some(&gettext("List view")));
        } else {
            // Current view is list, so the main button offers grid view.
            button.set_icon_name("view-grid-symbolic");
            button.set_tooltip_text(Some(&gettext("Grid view")));
        }

        if let Some(action) = imp.visible_columns_action.borrow().as_ref() {
            action.set_enabled(!is_grid);
        }
        if let Some(action) = imp.captions_action.borrow().as_ref() {
            action.set_enabled(is_grid);
        }
    }

    /// Restore zoom button state from saved settings.
    pub fn set_zoom_level(&self, level: u32) {
        let imp = self.imp();
        let level = level.min(2);

        imp.zoom_level.set(level);
        imp.update_zoom_sensitivity();
    }

    pub fn set_show_hidden_files(&self, show: bool) {
        if let Some(action) = self.imp().hidden_action.borrow().as_ref() {
            action.set_state(&show.to_variant());
        }
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

    fn set_sort_label_for_key(&self, _key: &str) {
        // Nautilus does not show the active sort text in the header button.
        // The selected option is represented by the stateful menu item.
    }
}
