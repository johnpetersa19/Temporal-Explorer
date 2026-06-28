use adw::prelude::*;
use adw::subclass::prelude::*;
use gtk::glib;
use std::cell::RefCell;

mod imp {
    use super::*;
    use gtk::CompositeTemplate;

    #[derive(Debug, Default, CompositeTemplate)]
    #[template(resource = "/io/github/johnpetersa19/TemporalExplorer/ssh-passphrase-dialog.ui")]
    pub struct SshPassphraseDialog {
        #[template_child]
        pub passphrase_entry: TemplateChild<adw::PasswordEntryRow>,
        #[template_child]
        pub cancel_button: TemplateChild<gtk::Button>,
        #[template_child]
        pub confirm_button: TemplateChild<gtk::Button>,

        pub passphrase: RefCell<String>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for SshPassphraseDialog {
        const NAME: &'static str = "SshPassphraseDialog";
        type Type = super::SshPassphraseDialog;
        type ParentType = adw::Dialog;

        fn class_init(klass: &mut Self::Class) {
            klass.bind_template();
        }

        fn instance_init(obj: &glib::subclass::InitializingObject<Self>) {
            obj.init_template();
        }
    }

    impl ObjectImpl for SshPassphraseDialog {
        fn signals() -> &'static [glib::subclass::Signal] {
            use std::sync::OnceLock;
            static SIGNALS: OnceLock<Vec<glib::subclass::Signal>> = OnceLock::new();
            SIGNALS.get_or_init(|| {
                vec![
                    glib::subclass::Signal::builder("passphrase-confirmed")
                        .param_types([String::static_type()])
                        .build(),
                    glib::subclass::Signal::builder("cancelled").build(),
                ]
            })
        }

        fn constructed(&self) {
            self.parent_constructed();
            let obj = self.obj();
            obj.setup_signals();
        }
    }

    impl WidgetImpl for SshPassphraseDialog {}
    impl AdwDialogImpl for SshPassphraseDialog {}
}

glib::wrapper! {
    pub struct SshPassphraseDialog(ObjectSubclass<imp::SshPassphraseDialog>)
        @extends adw::Dialog, gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl SshPassphraseDialog {
    pub fn new() -> Self {
        glib::Object::builder().build()
    }

    fn setup_signals(&self) {
        let imp = self.imp();

        // Enable confirm button only when entry is non-empty
        imp.passphrase_entry.connect_changed(glib::clone!(
            #[weak(rename_to = dialog)]
            self,
            move |entry| {
                let text = entry.text();
                dialog.imp().confirm_button.set_sensitive(!text.is_empty());
            }
        ));
        imp.confirm_button.set_sensitive(false);

        // Enter key in password entry triggers confirm
        imp.passphrase_entry.connect_entry_activated(glib::clone!(
            #[weak(rename_to = dialog)]
            self,
            move |_| {
                dialog.on_confirm();
            }
        ));

        imp.confirm_button.connect_clicked(glib::clone!(
            #[weak(rename_to = dialog)]
            self,
            move |_| {
                dialog.on_confirm();
            }
        ));

        imp.cancel_button.connect_clicked(glib::clone!(
            #[weak(rename_to = dialog)]
            self,
            move |_| {
                dialog.emit_by_name::<()>("cancelled", &[]);
                dialog.force_close();
            }
        ));
    }

    fn on_confirm(&self) {
        let passphrase = self.imp().passphrase_entry.text().to_string();
        *self.imp().passphrase.borrow_mut() = passphrase.clone();
        self.emit_by_name::<()>("passphrase-confirmed", &[&passphrase]);
        self.force_close();
    }

    /// Returns the passphrase entered by the user (after confirmation).
    pub fn get_passphrase(&self) -> String {
        self.imp().passphrase.borrow().clone()
    }

    pub fn connect_passphrase_confirmed<F: Fn(&Self, String) + 'static>(
        &self,
        f: F,
    ) -> glib::SignalHandlerId {
        self.connect_closure(
            "passphrase-confirmed",
            false,
            glib::closure_local!(move |dialog: &Self, passphrase: String| {
                f(dialog, passphrase);
            }),
        )
    }

    pub fn connect_cancelled<F: Fn(&Self) + 'static>(&self, f: F) -> glib::SignalHandlerId {
        self.connect_closure(
            "cancelled",
            false,
            glib::closure_local!(move |dialog: &Self| {
                f(dialog);
            }),
        )
    }
}

impl Default for SshPassphraseDialog {
    fn default() -> Self {
        Self::new()
    }
}
