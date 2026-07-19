use adw::prelude::ActionRowExt;
use adw::subclass::prelude::*;
use gtk::glib;
use gtk::prelude::*;
use std::cell::RefCell;

#[derive(Debug, Clone, Default)]
pub struct CommitDetails {
    pub summary: String,
    pub sha: String,
    pub parents: String,
    pub author: String,
    pub email: String,
    pub date: String,
    pub message: String,
}

mod imp {
    use super::*;

    #[derive(Debug, Default, gtk::CompositeTemplate)]
    #[template(resource = "/io/github/TemporalExplorer/commit-details-dialog.ui")]
    pub struct CommitDetailsDialog {
        #[template_child]
        pub summary_row: TemplateChild<adw::ActionRow>,
        #[template_child]
        pub sha_row: TemplateChild<adw::ActionRow>,
        #[template_child]
        pub parents_row: TemplateChild<adw::ActionRow>,
        #[template_child]
        pub author_row: TemplateChild<adw::ActionRow>,
        #[template_child]
        pub email_row: TemplateChild<adw::ActionRow>,
        #[template_child]
        pub date_row: TemplateChild<adw::ActionRow>,
        #[template_child]
        pub message_label: TemplateChild<gtk::Label>,

        pub sha: RefCell<String>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for CommitDetailsDialog {
        const NAME: &'static str = "CommitDetailsDialog";
        type Type = super::CommitDetailsDialog;
        type ParentType = adw::Dialog;

        fn class_init(klass: &mut Self::Class) {
            klass.bind_template();
            klass.bind_template_callbacks();
        }

        fn instance_init(obj: &glib::subclass::InitializingObject<Self>) {
            obj.init_template();
        }
    }

    impl ObjectImpl for CommitDetailsDialog {}
    impl WidgetImpl for CommitDetailsDialog {}
    impl AdwDialogImpl for CommitDetailsDialog {}

    #[gtk::template_callbacks]
    impl CommitDetailsDialog {
        #[template_callback]
        fn on_copy_sha_clicked(&self) {
            if let Some(display) = gtk::gdk::Display::default() {
                display.clipboard().set_text(&self.sha.borrow());
            }
        }
    }
}

glib::wrapper! {
    pub struct CommitDetailsDialog(ObjectSubclass<imp::CommitDetailsDialog>)
        @extends adw::Dialog, gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl CommitDetailsDialog {
    pub fn new() -> Self {
        glib::Object::new()
    }

    pub fn set_details(&self, details: &CommitDetails) {
        let imp = self.imp();
        imp.summary_row.set_subtitle(&details.summary);
        imp.sha_row.set_subtitle(&details.sha);
        imp.parents_row.set_subtitle(&details.parents);
        imp.author_row.set_subtitle(&details.author);
        imp.email_row.set_subtitle(&details.email);
        imp.date_row.set_subtitle(&details.date);
        imp.message_label.set_label(&details.message);
        *imp.sha.borrow_mut() = details.sha.clone();
    }
}

impl Default for CommitDetailsDialog {
    fn default() -> Self {
        Self::new()
    }
}
