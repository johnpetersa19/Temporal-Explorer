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
    pub parent_folder: String,
    pub snapshot_commit: String,
    pub full_commit: String,
    pub snapshot_date: String,
    pub git_object: String,
    pub git_mode: String,
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
        #[template_child] pub kind_label: TemplateChild<gtk::Label>,
        #[template_child] pub summary_label: TemplateChild<gtk::Label>,

        #[template_child] pub parent_folder_label: TemplateChild<gtk::Label>,
        #[template_child] pub repository_label: TemplateChild<gtk::Label>,
        #[template_child] pub modified_label: TemplateChild<gtk::Label>,
        #[template_child] pub created_label: TemplateChild<gtk::Label>,
        #[template_child] pub permissions_label: TemplateChild<gtk::Label>,
        #[template_child] pub snapshot_label: TemplateChild<gtk::Label>,
        #[template_child] pub object_label: TemplateChild<gtk::Label>,

        #[template_child] pub copy_path_button: TemplateChild<gtk::Button>,
        #[template_child] pub copy_details_button: TemplateChild<gtk::Button>,

        pub details_text: RefCell<String>,
        pub path_text: RefCell<String>,
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
            let button = self.copy_details_button.get();

            button.connect_clicked(move |_| {
                obj.copy_details();
            });

            let obj = self.obj().clone();
            let button = self.copy_path_button.get();

            button.connect_clicked(move |_| {
                obj.copy_path();
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

        imp.dialog_title.set_title("");
        imp.dialog_title.set_subtitle("");

        imp.icon_image.set_icon_name(Some(&props.icon_name));

        imp.name_label.set_label(&props.name);
        imp.name_label.set_tooltip_text(Some(&props.name));

        imp.kind_label.set_label(&props.kind);
        imp.summary_label.set_label(&format!("{} · {}", props.size, props.snapshot_date));

        imp.parent_folder_label.set_label(&props.parent_folder);
        imp.parent_folder_label.set_tooltip_text(Some(&props.repository_path));

        imp.repository_label.set_label(&props.repository);
        imp.repository_label.set_tooltip_text(Some(&props.repository));

        // Git does not store a real per-file "modified" timestamp in the tree.
        // We display the selected snapshot commit date instead.
        imp.modified_label.set_label(&props.snapshot_date);
        imp.modified_label.set_tooltip_text(Some(&props.full_commit));

        // Git snapshots do not store a real file creation timestamp.
        imp.created_label.set_label(&gettext("Not available in Git snapshot"));
        imp.created_label
            .set_tooltip_text(Some(&gettext("Git stores content snapshots, not filesystem creation time")));

        // Nautilus shows filesystem permissions here. For a Git snapshot,
        // the closest equivalent is the Git tree mode.
        imp.permissions_label.set_label(&props.git_mode);
        imp.snapshot_label.set_label(&props.snapshot_commit);
        imp.snapshot_label.set_tooltip_text(Some(&props.full_commit));

        imp.object_label.set_label(&shorten_middle(&props.git_object, 18));
        imp.object_label.set_tooltip_text(Some(&props.git_object));

        *imp.path_text.borrow_mut() = props.repository_path.clone();

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

    fn copy_path(&self) {
        if let Some(display) = gtk::gdk::Display::default() {
            display.clipboard().set_text(&self.imp().path_text.borrow());
        }
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
