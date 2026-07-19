use adw::prelude::*;
use adw::subclass::prelude::*;
use gtk::{gio, glib, CompositeTemplate};
use std::cell::RefCell;
use std::path::PathBuf;
use std::sync::OnceLock;

mod imp {
    use super::*;

    #[derive(Debug, Default, CompositeTemplate)]
    #[template(resource = "/io/github/TemporalExplorer/clone-repository-dialog.ui")]
    pub struct CloneRepositoryDialog {
        #[template_child]
        pub url_entry: TemplateChild<adw::EntryRow>,
        #[template_child]
        pub destination_entry: TemplateChild<adw::EntryRow>,
        #[template_child]
        pub choose_destination_button: TemplateChild<gtk::Button>,
        #[template_child]
        pub include_files_row: TemplateChild<adw::SwitchRow>,
        #[template_child]
        pub cancel_button: TemplateChild<gtk::Button>,
        #[template_child]
        pub open_button: TemplateChild<gtk::Button>,
        #[template_child]
        pub save_open_button: TemplateChild<gtk::Button>,

        pub url: RefCell<String>,
        pub destination: RefCell<String>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for CloneRepositoryDialog {
        const NAME: &'static str = "CloneRepositoryDialog";
        type Type = super::CloneRepositoryDialog;
        type ParentType = adw::Dialog;

        fn class_init(klass: &mut Self::Class) {
            klass.bind_template();
        }

        fn instance_init(obj: &glib::subclass::InitializingObject<Self>) {
            obj.init_template();
        }
    }

    impl ObjectImpl for CloneRepositoryDialog {
        fn constructed(&self) {
            self.parent_constructed();

            let obj_weak = self.obj().downgrade();
            self.url_entry.connect_changed(move |entry| {
                if let Some(obj) = obj_weak.upgrade() {
                    *obj.imp().url.borrow_mut() = entry.text().trim().to_string();
                    obj.update_action_buttons();
                }
            });

            let obj_weak = self.obj().downgrade();
            self.destination_entry.connect_changed(move |entry| {
                if let Some(obj) = obj_weak.upgrade() {
                    *obj.imp().destination.borrow_mut() = entry.text().trim().to_string();
                    obj.update_action_buttons();
                }
            });

            let obj_weak = self.obj().downgrade();
            self.choose_destination_button.connect_clicked(move |_| {
                if let Some(obj) = obj_weak.upgrade() {
                    obj.choose_destination();
                }
            });

            let obj_weak = self.obj().downgrade();
            self.cancel_button.connect_clicked(move |_| {
                if let Some(obj) = obj_weak.upgrade() {
                    obj.close();
                }
            });

            let obj_weak = self.obj().downgrade();
            self.open_button.connect_clicked(move |_| {
                if let Some(obj) = obj_weak.upgrade() {
                    obj.emit_clone_requested(false);
                }
            });

            let obj_weak = self.obj().downgrade();
            self.save_open_button.connect_clicked(move |_| {
                if let Some(obj) = obj_weak.upgrade() {
                    obj.emit_clone_requested(true);
                }
            });

            let obj_weak = self.obj().downgrade();
            self.url_entry.connect_entry_activated(move |_| {
                if let Some(obj) = obj_weak.upgrade() {
                    obj.activate_open_if_ready();
                }
            });

            let obj_weak = self.obj().downgrade();
            self.destination_entry.connect_entry_activated(move |_| {
                if let Some(obj) = obj_weak.upgrade() {
                    obj.activate_save_open_if_ready();
                }
            });
        }

        fn signals() -> &'static [glib::subclass::Signal] {
            static SIGNALS: OnceLock<Vec<glib::subclass::Signal>> = OnceLock::new();
            SIGNALS.get_or_init(|| {
                vec![glib::subclass::Signal::builder("clone-requested")
                    .param_types([
                        String::static_type(),
                        String::static_type(),
                        bool::static_type(),
                        bool::static_type(),
                    ])
                    .build()]
            })
        }
    }

    impl WidgetImpl for CloneRepositoryDialog {}
    impl AdwDialogImpl for CloneRepositoryDialog {}
}

glib::wrapper! {
    pub struct CloneRepositoryDialog(ObjectSubclass<imp::CloneRepositoryDialog>)
        @extends adw::Dialog, gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl Default for CloneRepositoryDialog {
    fn default() -> Self {
        Self::new()
    }
}

impl CloneRepositoryDialog {
    pub fn new() -> Self {
        glib::Object::new()
    }

    pub fn connect_clone_requested<F>(&self, f: F) -> glib::SignalHandlerId
    where
        F: Fn(&Self, &str, &str, bool, bool) + 'static,
    {
        self.connect_local("clone-requested", false, move |values| {
            let dialog = values[0].get::<CloneRepositoryDialog>().unwrap();
            let url = values[1].get::<String>().unwrap_or_default();
            let destination = values[2].get::<String>().unwrap_or_default();
            let save = values[3].get::<bool>().unwrap_or(false);
            let include_files = values[4].get::<bool>().unwrap_or(true);
            f(&dialog, &url, &destination, save, include_files);
            None
        })
    }

    fn choose_destination(&self) {
        let dialog = gtk::FileDialog::builder()
            .title(gettextrs::gettext("Choose Clone Destination"))
            .modal(true)
            .build();

        let obj = self.clone();
        dialog.select_folder(gtk::Window::NONE, gio::Cancellable::NONE, move |result| {
            let Ok(file) = result else {
                return;
            };

            let Some(path) = file.path() else {
                return;
            };

            obj.set_destination(path);
        });
    }

    fn set_destination(&self, path: PathBuf) {
        let text = path.to_string_lossy().to_string();
        self.imp().destination_entry.set_text(&text);
        *self.imp().destination.borrow_mut() = text;
        self.update_action_buttons();
    }

    fn update_action_buttons(&self) {
        let imp = self.imp();
        let has_url = !imp.url.borrow().trim().is_empty();
        let has_destination = !imp.destination.borrow().trim().is_empty();
        imp.open_button.set_sensitive(has_url);
        imp.save_open_button
            .set_sensitive(has_url && has_destination);
    }

    fn activate_open_if_ready(&self) {
        if self.imp().open_button.is_sensitive() {
            self.emit_clone_requested(false);
        }
    }

    fn activate_save_open_if_ready(&self) {
        if self.imp().save_open_button.is_sensitive() {
            self.emit_clone_requested(true);
        }
    }

    fn emit_clone_requested(&self, save: bool) {
        let imp = self.imp();
        let url = imp.url.borrow().trim().to_string();
        let destination = imp.destination.borrow().trim().to_string();
        let include_files = imp.include_files_row.is_active();

        if url.is_empty() || (save && destination.is_empty()) {
            return;
        }

        self.close();
        self.emit_by_name::<()>(
            "clone-requested",
            &[&url, &destination, &save, &include_files],
        );
    }
}
