/* node_properties_dialog.rs
 *
 * Copyright 2026 John Peter Sá
 * SPDX-License-Identifier: GPL-3.0-or-later
 */

use adw::prelude::*;
use adw::subclass::prelude::*;
use gettextrs::gettext;
use gtk::glib;
use std::cell::{Cell, RefCell};
use std::sync::OnceLock;

#[derive(Debug, Clone, Default)]
pub struct NodeProperties {
    pub name: String,
    pub kind: String,
    pub icon_name: String,
    pub repository: String,
    pub repository_path: String,
    pub parent_folder: String,
    pub snapshot_commit: String,
    pub full_commit: String,
    pub snapshot_date: String,
    pub git_object: String,
    pub git_mode: String,
    pub size: String,
    pub system_status: String,
    pub favorite: bool,
}

mod imp {
    use super::*;

    #[derive(Debug, Default, gtk::CompositeTemplate)]
    #[template(resource = "/io/github/TemporalExplorer/node-properties-dialog.ui")]
    pub struct NodePropertiesDialog {
        #[template_child]
        pub toast_overlay: TemplateChild<adw::ToastOverlay>,
        #[template_child]
        pub nav_view: TemplateChild<adw::NavigationView>,
        #[template_child]
        pub icon_image: TemplateChild<gtk::Image>,
        #[template_child]
        pub name_label: TemplateChild<gtk::Label>,
        #[template_child]
        pub kind_label: TemplateChild<gtk::Label>,
        #[template_child]
        pub summary_label: TemplateChild<gtk::Label>,

        #[template_child]
        pub parent_folder_row: TemplateChild<adw::ActionRow>,
        #[template_child]
        pub repository_row: TemplateChild<adw::ActionRow>,
        #[template_child]
        pub system_status_row: TemplateChild<adw::ActionRow>,
        #[template_child]
        pub modified_row: TemplateChild<adw::ActionRow>,
        #[template_child]
        pub created_row: TemplateChild<adw::ActionRow>,
        #[template_child]
        pub permissions_row: TemplateChild<adw::ActionRow>,
        #[template_child]
        pub permissions_label: TemplateChild<gtk::Label>,
        #[template_child]
        pub snapshot_row: TemplateChild<adw::ActionRow>,
        #[template_child]
        pub object_row: TemplateChild<adw::ActionRow>,
        #[template_child]
        pub copy_details_row: TemplateChild<adw::ButtonRow>,

        #[template_child]
        pub mode_octal_row: TemplateChild<adw::ActionRow>,
        #[template_child]
        pub mode_type_row: TemplateChild<adw::ActionRow>,
        #[template_child]
        pub mode_exec_row: TemplateChild<adw::ActionRow>,
        #[template_child]
        pub perm_commit_row: TemplateChild<adw::ActionRow>,
        #[template_child]
        pub perm_object_row: TemplateChild<adw::ActionRow>,

        #[template_child]
        pub favorite_button: TemplateChild<gtk::Button>,
        #[template_child]
        pub open_location_button: TemplateChild<gtk::Button>,
        #[template_child]
        pub open_parent_folder_button: TemplateChild<gtk::Button>,
        #[template_child]
        pub copy_path_button: TemplateChild<gtk::Button>,
        #[template_child]
        pub copy_sha_button: TemplateChild<gtk::Button>,
        #[template_child]
        pub copy_object_button: TemplateChild<gtk::Button>,

        pub details_text: RefCell<String>,
        pub path_text: RefCell<String>,
        pub full_commit_text: RefCell<String>,
        pub git_object_text: RefCell<String>,
        pub favorite_active: Cell<bool>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for NodePropertiesDialog {
        const NAME: &'static str = "NodePropertiesDialog";
        type Type = super::NodePropertiesDialog;
        type ParentType = adw::Dialog;

        fn class_init(klass: &mut Self::Class) {
            klass.bind_template();
        }

        fn instance_init(obj: &glib::subclass::InitializingObject<Self>) {
            obj.init_template();
        }
    }

    impl ObjectImpl for NodePropertiesDialog {
        fn constructed(&self) {
            self.parent_constructed();

            let obj = self.obj().clone();
            self.favorite_button.connect_clicked(move |_| {
                obj.toggle_favorite();
            });

            let obj = self.obj().clone();
            self.open_location_button.connect_clicked(move |_| {
                obj.emit_open_location_requested();
            });

            let obj = self.obj().clone();
            self.open_parent_folder_button.connect_clicked(move |_| {
                obj.emit_open_location_requested();
            });

            let obj = self.obj().clone();
            self.copy_path_button.connect_clicked(move |_| {
                obj.copy_text(&obj.imp().path_text.borrow(), &gettext("Path copied"));
            });

            let obj = self.obj().clone();
            self.copy_sha_button.connect_clicked(move |_| {
                obj.copy_text(
                    &obj.imp().full_commit_text.borrow(),
                    &gettext("Commit SHA copied"),
                );
            });

            let obj = self.obj().clone();
            self.copy_object_button.connect_clicked(move |_| {
                obj.copy_text(
                    &obj.imp().git_object_text.borrow(),
                    &gettext("Object hash copied"),
                );
            });

            let obj = self.obj().clone();
            self.copy_details_row.connect_activated(move |_| {
                obj.copy_text(
                    &obj.imp().details_text.borrow(),
                    &gettext("Details copied"),
                );
            });

            let nav = self.nav_view.get();
            self.permissions_row.connect_activated(move |_| {
                nav.push_by_tag("permissions");
            });
        }

        fn signals() -> &'static [glib::subclass::Signal] {
            static SIGNALS: OnceLock<Vec<glib::subclass::Signal>> = OnceLock::new();
            SIGNALS.get_or_init(|| {
                vec![
                    glib::subclass::Signal::builder("favorite-toggled")
                        .param_types([bool::static_type()])
                        .build(),
                    glib::subclass::Signal::builder("open-location-requested")
                        .param_types([String::static_type()])
                        .build(),
                    glib::subclass::Signal::builder("icon-edit-requested").build(),
                ]
            })
        }
    }

    impl WidgetImpl for NodePropertiesDialog {}
    impl AdwDialogImpl for NodePropertiesDialog {}
}

glib::wrapper! {
    pub struct NodePropertiesDialog(ObjectSubclass<imp::NodePropertiesDialog>)
        @extends adw::Dialog, gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget, gtk::ShortcutManager;
}

impl Default for NodePropertiesDialog {
    fn default() -> Self {
        Self::new()
    }
}

impl NodePropertiesDialog {
    pub fn new() -> Self {
        glib::Object::new()
    }

    pub fn set_properties(&self, props: &NodeProperties) {
        let imp = self.imp();

        imp.icon_image.set_icon_name(Some(&props.icon_name));

        imp.name_label.set_label(&props.name);
        imp.name_label.set_tooltip_text(Some(&props.name));

        imp.kind_label.set_label(&props.kind);
        imp.summary_label
            .set_label(&format!("{} · {}", props.size, props.snapshot_date));

        imp.parent_folder_row.set_subtitle(&props.parent_folder);
        imp.parent_folder_row
            .set_tooltip_text(Some(&props.repository_path));

        imp.repository_row.set_subtitle(&props.repository);
        imp.system_status_row.set_subtitle(&props.system_status);

        imp.modified_row.set_subtitle(&props.snapshot_date);
        imp.modified_row
            .set_tooltip_text(Some(&props.full_commit));

        imp.created_row
            .set_subtitle(&gettext("Not available in Git snapshot"));

        imp.permissions_label.set_label(&props.git_mode);
        imp.snapshot_row.set_subtitle(&props.snapshot_commit);
        imp.snapshot_row
            .set_tooltip_text(Some(&props.full_commit));

        imp.object_row
            .set_subtitle(&shorten_middle(&props.git_object, 22));
        imp.object_row.set_tooltip_text(Some(&props.git_object));

        // Populate Permissions subpage (Nautilus-style drill-down)
        let (octal, kind_desc) = props
            .git_mode
            .split_once(" · ")
            .unwrap_or((props.git_mode.as_str(), props.kind.as_str()));
        imp.mode_octal_row.set_subtitle(octal);
        imp.mode_type_row.set_subtitle(kind_desc);
        let is_exec = octal == "100755";
        imp.mode_exec_row.set_subtitle(&if is_exec {
            gettext("Yes (executable bit set)")
        } else {
            gettext("No")
        });
        imp.perm_commit_row.set_subtitle(&props.full_commit);
        imp.perm_object_row.set_subtitle(&props.git_object);

        *imp.path_text.borrow_mut() = props.repository_path.clone();
        *imp.full_commit_text.borrow_mut() = props.full_commit.clone();
        *imp.git_object_text.borrow_mut() = props.git_object.clone();

        imp.favorite_active.set(props.favorite);
        imp.favorite_button.set_icon_name(if props.favorite {
            "starred-symbolic"
        } else {
            "non-starred-symbolic"
        });
        if props.favorite {
            imp.favorite_button.add_css_class("starred");
        } else {
            imp.favorite_button.remove_css_class("starred");
        }
        imp.favorite_button
            .set_tooltip_text(Some(&if props.favorite {
                gettext("Unstar")
            } else {
                gettext("Star")
            }));

        let details = format!(
            "{}: {}\n{}: {}\n{}: {}\n{}: {}\n{}: {}\n{}: {}\n{}: {}\n{}: {}\n{}: {}\n{}: {}\n{}: {}",
            gettext("Name"),
            props.name,
            gettext("Type"),
            props.kind,
            gettext("Size"),
            props.size,
            gettext("Repository"),
            props.repository,
            gettext("Repository Path"),
            props.repository_path,
            gettext("Parent Folder"),
            props.parent_folder,
            gettext("Snapshot Commit"),
            props.snapshot_commit,
            gettext("Snapshot Date"),
            props.snapshot_date,
            gettext("Full Commit"),
            props.full_commit,
            gettext("Git Object"),
            props.git_object,
            gettext("Git Mode"),
            props.git_mode,
        );

        *imp.details_text.borrow_mut() = details;
    }

    fn copy_text(&self, text: &str, toast_msg: &str) {
        if let Some(display) = gtk::gdk::Display::default() {
            display.clipboard().set_text(text);
            self.imp().toast_overlay.add_toast(adw::Toast::new(toast_msg));
        }
    }

    fn toggle_favorite(&self) {
        let imp = self.imp();
        let active = !imp.favorite_active.get();
        imp.favorite_active.set(active);

        let icon = if active {
            "starred-symbolic"
        } else {
            "non-starred-symbolic"
        };
        imp.favorite_button.set_icon_name(icon);
        if active {
            imp.favorite_button.add_css_class("starred");
        } else {
            imp.favorite_button.remove_css_class("starred");
        }
        imp.favorite_button.add_css_class("interacted");
        let tooltip = if active {
            gettext("Unstar")
        } else {
            gettext("Star")
        };
        imp.favorite_button.set_tooltip_text(Some(&tooltip));
        self.emit_by_name::<()>("favorite-toggled", &[&active]);
    }

    fn emit_open_location_requested(&self) {
        let path = self.imp().path_text.borrow().clone();
        self.emit_by_name::<()>("open-location-requested", &[&path]);
    }

    pub fn connect_favorite_toggled<F>(&self, f: F) -> glib::SignalHandlerId
    where
        F: Fn(&Self, bool) + 'static,
    {
        self.connect_local("favorite-toggled", false, move |values| {
            let dialog = values[0].get::<NodePropertiesDialog>().unwrap();
            let active = values[1].get::<bool>().unwrap_or(false);
            f(&dialog, active);
            None
        })
    }

    pub fn connect_open_location_requested<F>(&self, f: F) -> glib::SignalHandlerId
    where
        F: Fn(&Self, &str) + 'static,
    {
        self.connect_local("open-location-requested", false, move |values| {
            let dialog = values[0].get::<NodePropertiesDialog>().unwrap();
            let path = values[1].get::<String>().unwrap_or_default();
            f(&dialog, &path);
            None
        })
    }

    pub fn connect_icon_edit_requested<F>(&self, f: F) -> glib::SignalHandlerId
    where
        F: Fn(&Self) + 'static,
    {
        self.connect_local("icon-edit-requested", false, move |values| {
            let dialog = values[0].get::<NodePropertiesDialog>().unwrap();
            f(&dialog);
            None
        })
    }
}

fn shorten_middle(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }

    let keep = max.saturating_sub(1) / 2;
    let start: String = text.chars().take(keep).collect();
    let end: String = text
        .chars()
        .rev()
        .take(keep)
        .collect::<String>()
        .chars()
        .rev()
        .collect();

    format!("{start}…{end}")
}
