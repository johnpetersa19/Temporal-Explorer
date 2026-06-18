use gtk::glib;
use gtk::prelude::*;
use gtk::subclass::prelude::*;
use adw::subclass::prelude::*;
use std::cell::RefCell;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

mod imp {
    use super::*;

    #[derive(Debug, Default, gtk::CompositeTemplate)]
    #[template(resource = "/io/github/johnpetersa19/TemporalExplorer/operation-progress-dialog.ui")]
    pub struct OperationProgressDialog {
        #[template_child]
        pub title_widget: TemplateChild<adw::WindowTitle>,
        #[template_child]
        pub status_label: TemplateChild<gtk::Label>,
        #[template_child]
        pub progress_bar: TemplateChild<gtk::ProgressBar>,
        #[template_child]
        pub cancel_button: TemplateChild<gtk::Button>,

        pub cancel_token: RefCell<Option<Arc<AtomicBool>>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for OperationProgressDialog {
        const NAME: &'static str = "OperationProgressDialog";
        type Type = super::OperationProgressDialog;
        type ParentType = adw::Dialog;

        fn class_init(klass: &mut Self::Class) {
            klass.bind_template();
        }

        fn instance_init(obj: &glib::subclass::InitializingObject<Self>) {
            obj.init_template();
        }
    }

    impl ObjectImpl for OperationProgressDialog {
        fn constructed(&self) {
            self.parent_constructed();

            let obj = self.obj().clone();
            let button = self.cancel_button.get();

            button.connect_clicked(move |_| {
                obj.cancel();
            });
        }
    }

    impl WidgetImpl for OperationProgressDialog {}
    impl AdwDialogImpl for OperationProgressDialog {}
}

glib::wrapper! {
    pub struct OperationProgressDialog(ObjectSubclass<imp::OperationProgressDialog>)
        @extends adw::Dialog, gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget, gtk::ShortcutManager;
}

impl Default for OperationProgressDialog {
    fn default() -> Self {
        Self::new()
    }
}

impl OperationProgressDialog {
    pub fn new() -> Self {
        glib::Object::new()
    }

    pub fn setup(&self, title: &str, status: &str, cancel_token: Arc<AtomicBool>) {
        let imp = self.imp();

        imp.title_widget.set_title(title);
        imp.status_label.set_label(status);
        imp.progress_bar.set_fraction(0.0);
        imp.progress_bar.set_text(Some("0%"));

        *imp.cancel_token.borrow_mut() = Some(cancel_token);
    }

    pub fn set_progress(&self, fraction: f64, status: &str) {
        let fraction = fraction.clamp(0.0, 1.0);

        self.imp().status_label.set_label(status);
        self.imp().progress_bar.set_fraction(fraction);
        self.imp()
            .progress_bar
            .set_text(Some(&format!("{:.0}%", fraction * 100.0)));
    }

    pub fn pulse(&self, status: &str) {
        self.imp().status_label.set_label(status);
        self.imp().progress_bar.pulse();
    }

    pub fn finish_and_close(&self) {
        self.close();
    }

    fn cancel(&self) {
        if let Some(token) = self.imp().cancel_token.borrow().as_ref() {
            token.store(true, Ordering::Relaxed);
        }

        self.imp().status_label.set_label("Cancelling…");
        self.imp().cancel_button.set_sensitive(false);
    }
}
