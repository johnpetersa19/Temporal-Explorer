/* app_chooser_dialog.rs
 *
 * Copyright 2026 John Peter Sá
 * SPDX-License-Identifier: GPL-3.0-or-later
 */

use adw::prelude::*;
use adw::subclass::prelude::*;
use gettextrs::gettext;
use gtk::{gio, glib};
use std::cell::RefCell;

pub type OnAppSelected = Box<dyn Fn(gio::AppInfo) + 'static>;

mod imp {
    use super::*;

    #[derive(Default, gtk::CompositeTemplate)]
    #[template(resource = "/io/github/TemporalExplorer/app-chooser-dialog.ui")]
    pub struct AppChooserDialog {
        #[template_child]
        pub cancel_button: TemplateChild<gtk::Button>,
        #[template_child]
        pub open_button: TemplateChild<gtk::Button>,
        #[template_child]
        pub search_entry: TemplateChild<gtk::SearchEntry>,
        #[template_child]
        pub description_label: TemplateChild<gtk::Label>,
        #[template_child]
        pub apps_list: TemplateChild<gtk::ListBox>,

        pub all_apps: RefCell<Vec<gio::AppInfo>>,
        pub filtered_apps: RefCell<Vec<gio::AppInfo>>,
        pub on_selected: RefCell<Option<OnAppSelected>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for AppChooserDialog {
        const NAME: &'static str = "AppChooserDialog";
        type Type = super::AppChooserDialog;
        type ParentType = adw::Dialog;

        fn class_init(klass: &mut Self::Class) {
            klass.bind_template();
        }

        fn instance_init(obj: &glib::subclass::InitializingObject<Self>) {
            obj.init_template();
        }
    }

    impl ObjectImpl for AppChooserDialog {
        fn constructed(&self) {
            self.parent_constructed();

            let obj = self.obj().clone();
            self.cancel_button.connect_clicked(move |_| {
                obj.close();
            });

            let obj = self.obj().clone();
            self.open_button.connect_clicked(move |_| {
                obj.activate_selected();
            });

            let obj = self.obj().clone();
            self.apps_list.connect_row_selected(move |_, row| {
                obj.imp().open_button.set_sensitive(row.is_some());
            });

            let obj = self.obj().clone();
            self.apps_list.connect_row_activated(move |_, _| {
                obj.activate_selected();
            });

            let obj = self.obj().clone();
            self.search_entry.connect_search_changed(move |entry| {
                obj.populate_list(entry.text().as_str());
            });
        }
    }

    impl WidgetImpl for AppChooserDialog {}
    impl AdwDialogImpl for AppChooserDialog {}
}

glib::wrapper! {
    pub struct AppChooserDialog(ObjectSubclass<imp::AppChooserDialog>)
        @extends adw::Dialog, gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget, gtk::ShortcutManager;
}

impl Default for AppChooserDialog {
    fn default() -> Self {
        Self::new()
    }
}

impl AppChooserDialog {
    pub fn new() -> Self {
        glib::Object::new()
    }

    pub fn configure(&self, file_name: &str, content_type: Option<&str>, on_selected: OnAppSelected) {
        let imp = self.imp();
        imp.description_label.set_label(&format!(
            "{} “{}”",
            gettext("Choose an app to open"),
            file_name
        ));

        let mut apps: Vec<gio::AppInfo> = content_type
            .map(gio::AppInfo::all_for_type)
            .unwrap_or_default();

        // Append remaining visible desktop apps so the user can always pick any editor/viewer.
        let mut seen_ids: std::collections::HashSet<String> = apps
            .iter()
            .filter_map(|a| a.id().map(|s| s.to_string()))
            .collect();
        for app in gio::AppInfo::all() {
            if !app.should_show() {
                continue;
            }
            if let Some(id) = app.id() {
                if seen_ids.insert(id.to_string()) {
                    apps.push(app);
                }
            }
        }

        *imp.all_apps.borrow_mut() = apps;
        *imp.on_selected.borrow_mut() = Some(on_selected);
        self.populate_list("");
    }

    fn populate_list(&self, query: &str) {
        let imp = self.imp();
        while let Some(child) = imp.apps_list.first_child() {
            imp.apps_list.remove(&child);
        }

        let needle = query.trim().to_lowercase();
        let filtered: Vec<gio::AppInfo> = imp
            .all_apps
            .borrow()
            .iter()
            .filter(|app| {
                if needle.is_empty() {
                    return true;
                }
                let name = app.display_name().to_lowercase();
                let desc = app
                    .description()
                    .map(|d| d.to_lowercase())
                    .unwrap_or_default();
                name.contains(&needle) || desc.contains(&needle)
            })
            .cloned()
            .collect();

        for app in &filtered {
            let row = adw::ActionRow::builder()
                .title(app.display_name().as_str())
                .subtitle(app.description().as_deref().unwrap_or(""))
                .activatable(true)
                .use_markup(false)
                .build();

            let image = if let Some(icon) = app.icon() {
                gtk::Image::from_gicon(&icon)
            } else {
                gtk::Image::from_icon_name("application-x-executable-symbolic")
            };
            image.set_pixel_size(32);
            row.add_prefix(&image);

            imp.apps_list.append(&row);
        }

        *imp.filtered_apps.borrow_mut() = filtered;
        imp.open_button.set_sensitive(false);
    }

    fn activate_selected(&self) {
        let imp = self.imp();
        let Some(row) = imp.apps_list.selected_row() else {
            return;
        };
        let idx = row.index();
        if idx < 0 {
            return;
        }
        let app = imp.filtered_apps.borrow().get(idx as usize).cloned();
        if let Some(app) = app {
            if let Some(cb) = imp.on_selected.borrow().as_ref() {
                cb(app);
            }
            self.close();
        }
    }
}
