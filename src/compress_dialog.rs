/* compress_dialog.rs
 *
 * Copyright 2026 John Peter Sá
 * SPDX-License-Identifier: GPL-3.0-or-later
 */

use adw::prelude::*;
use adw::subclass::prelude::*;
use gtk::glib;
use std::cell::RefCell;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArchiveFormat {
    Zip,
    TarGz,
    Tar,
}

impl ArchiveFormat {
    pub fn git_format(self) -> &'static str {
        match self {
            ArchiveFormat::Zip => "zip",
            ArchiveFormat::TarGz => "tar.gz",
            ArchiveFormat::Tar => "tar",
        }
    }

    pub fn extension(self) -> &'static str {
        match self {
            ArchiveFormat::Zip => ".zip",
            ArchiveFormat::TarGz => ".tar.gz",
            ArchiveFormat::Tar => ".tar",
        }
    }
}

pub type OnExportArchive = Box<dyn Fn(String, ArchiveFormat) + 'static>;

mod imp {
    use super::*;

    #[derive(Default, gtk::CompositeTemplate)]
    #[template(resource = "/io/github/TemporalExplorer/compress-dialog.ui")]
    pub struct CompressDialog {
        #[template_child]
        pub cancel_button: TemplateChild<gtk::Button>,
        #[template_child]
        pub export_button: TemplateChild<gtk::Button>,
        #[template_child]
        pub name_entry: TemplateChild<adw::EntryRow>,
        #[template_child]
        pub format_combo_row: TemplateChild<adw::ComboRow>,

        pub on_export: RefCell<Option<OnExportArchive>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for CompressDialog {
        const NAME: &'static str = "CompressDialog";
        type Type = super::CompressDialog;
        type ParentType = adw::Dialog;

        fn class_init(klass: &mut Self::Class) {
            klass.bind_template();
        }

        fn instance_init(obj: &glib::subclass::InitializingObject<Self>) {
            obj.init_template();
        }
    }

    impl ObjectImpl for CompressDialog {
        fn constructed(&self) {
            self.parent_constructed();

            let obj = self.obj().clone();
            self.cancel_button.connect_clicked(move |_| {
                obj.close();
            });

            let obj = self.obj().clone();
            self.name_entry.connect_changed(move |entry| {
                let valid = !entry.text().trim().is_empty();
                obj.imp().export_button.set_sensitive(valid);
            });

            let obj = self.obj().clone();
            self.export_button.connect_clicked(move |_| {
                obj.submit();
            });

            let obj = self.obj().clone();
            self.name_entry.connect_entry_activated(move |_| {
                obj.submit();
            });
        }
    }

    impl WidgetImpl for CompressDialog {}
    impl AdwDialogImpl for CompressDialog {}
}

glib::wrapper! {
    pub struct CompressDialog(ObjectSubclass<imp::CompressDialog>)
        @extends adw::Dialog, gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget, gtk::ShortcutManager;
}

impl Default for CompressDialog {
    fn default() -> Self {
        Self::new()
    }
}

impl CompressDialog {
    pub fn new() -> Self {
        glib::Object::new()
    }

    pub fn configure(&self, default_name: &str, on_export: OnExportArchive) {
        let imp = self.imp();
        imp.name_entry.set_text(default_name);
        imp.export_button
            .set_sensitive(!default_name.trim().is_empty());
        *imp.on_export.borrow_mut() = Some(on_export);
    }

    fn selected_format(&self) -> ArchiveFormat {
        match self.imp().format_combo_row.selected() {
            1 => ArchiveFormat::TarGz,
            2 => ArchiveFormat::Tar,
            _ => ArchiveFormat::Zip,
        }
    }

    fn submit(&self) {
        let name = self.imp().name_entry.text().trim().to_string();
        if name.is_empty() {
            return;
        }
        let format = self.selected_format();
        if let Some(cb) = self.imp().on_export.borrow().as_ref() {
            cb(name, format);
        }
        self.close();
    }
}
