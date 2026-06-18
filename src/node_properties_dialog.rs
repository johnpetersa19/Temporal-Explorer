/* node_properties_dialog.rs
 *
 * Copyright 2026 John Peter Sá
 * SPDX-License-Identifier: GPL-3.0-or-later
 */

use gettextrs::gettext;
use gtk::glib;
use gtk::prelude::*;
use gtk::subclass::prelude::*;
use adw::prelude::*;
use adw::subclass::prelude::*;
use std::cell::RefCell;

#[derive(Debug, Clone, Default)]
pub struct NodeProperties {
    pub name: String,
    pub kind: String,
    pub icon_name: String,
    pub repository: String,
    pub repository_path: String,
    pub snapshot_commit: String,
    pub full_commit: String,
    pub git_object: String,
    pub size: String,
    pub system_status: String,
}

mod imp {
    use super::*;

    #[derive(Debug, Default, gtk::CompositeTemplate)]
    #[template(resource = "/io/github/johnpetersa19/TemporalExplorer/node-properties-dialog.ui")]
    pub struct NodePropertiesDialog {
        #[template_child] pub dialog_title: TemplateChild<adw::WindowTitle>,
        #[template_child] pub icon_image: TemplateChild<gtk::Image>,
        #[template_child] pub name_label: TemplateChild<gtk::Label>,
        #[template_child] pub path_label: TemplateChild<gtk::Label>,

        #[template_child] pub type_row: TemplateChild<adw::ActionRow>,
        #[template_child] pub size_row: TemplateChild<adw::ActionRow>,
        #[template_child] pub repository_path_row: TemplateChild<adw::ActionRow>,
        #[template_child] pub system_status_row: TemplateChild<adw::ActionRow>,

        #[template_child] pub repository_row: TemplateChild<adw::ActionRow>,
        #[template_child] pub snapshot_commit_row: TemplateChild<adw::ActionRow>,
        #[template_child] pub object_row: TemplateChild<adw::ActionRow>,
        #[template_child] pub full_commit_row: TemplateChild<adw::ActionRow>,

        #[template_child] pub copy_details_button: TemplateChild<gtk::Button>,

        pub details_text: RefCell<String>,
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

            let obj = self.obj();
            self.copy_details_button.connect_clicked(move |_| {
                obj.copy_details();
            });
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

        imp.dialog_title.set_title(&gettext("Properties"));
        imp.dialog_title.set_subtitle(&props.name);

        imp.icon_image.set_icon_name(Some(&props.icon_name));
        imp.name_label.set_label(&props.name);
        imp.path_label.set_label(&props.repository_path);
        imp.path_label.set_tooltip_text(Some(&props.repository_path));

        imp.type_row.set_subtitle(&props.kind);
        imp.size_row.set_subtitle(&props.size);
        imp.repository_path_row.set_subtitle(&props.repository_path);
        imp.repository_path_row.set_tooltip_text(Some(&props.repository_path));
        imp.system_status_row.set_subtitle(&props.system_status);

        imp.repository_row.set_subtitle(&props.repository);
        imp.snapshot_commit_row.set_subtitle(&props.snapshot_commit);
        imp.object_row.set_subtitle(&props.git_object);
        imp.object_row.set_tooltip_text(Some(&props.git_object));
        imp.full_commit_row.set_subtitle(&props.full_commit);
        imp.full_commit_row.set_tooltip_text(Some(&props.full_commit));

        let details = format!(
            "{}: {}\n{}: {}\n{}: {}\n{}: {}\n{}: {}\n{}: {}\n{}: {}\n{}: {}\n{}: {}",
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
            gettext("Snapshot Commit"),
            props.snapshot_commit,
            gettext("Full Commit"),
            props.full_commit,
            gettext("Git Object"),
            props.git_object,
            gettext("System Status"),
            props.system_status,
        );

        *imp.details_text.borrow_mut() = details;
    }

    fn copy_details(&self) {
        if let Some(display) = gtk::gdk::Display::default() {
            display.clipboard().set_text(&self.imp().details_text.borrow());
        }
    }
}
