use gtk::glib;
use gtk::prelude::*;
use gtk::subclass::prelude::*;

use crate::git_engine::TreeNode;
use crate::icon_helpers::{folder_icon_symbolic, mime_icon};

mod imp {
    use super::*;

    #[derive(Debug, Default, gtk::CompositeTemplate)]
    #[template(resource = "/io/github/johnpetersa19/TemporalExplorer/file-list-row.ui")]
    pub struct FileListRow {
        #[template_child]
        pub file_row_icon: TemplateChild<gtk::Image>,
        #[template_child]
        pub file_row_emblems_box: TemplateChild<gtk::Box>,
        #[template_child]
        pub file_row_name_label: TemplateChild<gtk::Label>,
        #[template_child]
        pub file_row_snippet_label: TemplateChild<gtk::Label>,
        #[template_child]
        pub file_row_spinner: TemplateChild<gtk::Spinner>,
        #[template_child]
        pub file_row_badge_label: TemplateChild<gtk::Label>,
        #[template_child]
        pub file_row_chevron: TemplateChild<gtk::Image>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for FileListRow {
        const NAME: &'static str = "FileListRow";
        type Type = super::FileListRow;
        type ParentType = gtk::ListBoxRow;

        fn class_init(klass: &mut Self::Class) {
            klass.bind_template();
        }

        fn instance_init(obj: &glib::subclass::InitializingObject<Self>) {
            obj.init_template();
        }
    }

    impl ObjectImpl for FileListRow {}
    impl WidgetImpl for FileListRow {}
    impl ListBoxRowImpl for FileListRow {}
}

glib::wrapper! {
    pub struct FileListRow(ObjectSubclass<imp::FileListRow>)
        @extends gtk::ListBoxRow, gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget, gtk::Actionable;
}

impl Default for FileListRow {
    fn default() -> Self {
        Self::new()
    }
}

impl FileListRow {
    pub fn new() -> Self {
        glib::Object::new()
    }

    pub fn configure(&self, node: &TreeNode) {
        let name = node
            .path()
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("");

        let repo_path = node.path().display().to_string();

        let icon_name = match node {
            TreeNode::Dir(path) => {
                folder_icon_symbolic(path.file_name().and_then(|n| n.to_str()).unwrap_or(""))
            }
            TreeNode::File(path) => mime_icon(path),
            TreeNode::Submodule(_) => "folder-remote-symbolic",
        };

        self.set_icon_name(icon_name);
        self.set_name(name);
        self.set_tooltip_text(Some(&repo_path));
        self.set_loading(false);
        self.clear_emblems();

        if node.is_dir() {
            self.set_snippet(&format!("Folder · {repo_path}"));
            self.set_badge("");
            self.set_chevron_visible(true);
        } else if node.is_submodule() {
            self.set_snippet(&format!("Git submodule · {repo_path}"));
            self.set_badge("submodule");
            self.set_chevron_visible(true);
            self.add_emblem("emblem-symbolic-link-symbolic");
        } else {
            let ext = node
                .path()
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("");

            if ext.is_empty() {
                self.set_snippet(&repo_path);
                self.set_badge("");
            } else {
                self.set_snippet(&format!("{} file · {}", ext.to_uppercase(), repo_path));
                self.set_badge(&ext.to_uppercase());
            }

            self.set_chevron_visible(false);
        }
    }

    pub fn set_icon_name(&self, icon_name: &str) {
        self.imp().file_row_icon.set_icon_name(Some(icon_name));
    }

    pub fn set_name(&self, name: &str) {
        self.imp().file_row_name_label.set_label(name);
        self.imp().file_row_name_label.set_tooltip_text(Some(name));
    }

    pub fn set_snippet(&self, snippet: &str) {
        self.imp().file_row_snippet_label.set_label(snippet);
        self.imp()
            .file_row_snippet_label
            .set_visible(!snippet.trim().is_empty());
    }

    pub fn set_loading(&self, loading: bool) {
        self.imp().file_row_spinner.set_visible(loading);

        if loading {
            self.imp().file_row_spinner.start();
        } else {
            self.imp().file_row_spinner.stop();
        }
    }

    pub fn set_badge(&self, badge: &str) {
        self.imp().file_row_badge_label.set_label(badge);
        self.imp()
            .file_row_badge_label
            .set_visible(!badge.trim().is_empty());
    }

    pub fn set_chevron_visible(&self, visible: bool) {
        self.imp().file_row_chevron.set_visible(visible);
    }

    pub fn clear_emblems(&self) {
        let box_ = self.imp().file_row_emblems_box.get();

        while let Some(child) = box_.first_child() {
            box_.remove(&child);
        }
    }

    pub fn add_emblem(&self, icon_name: &str) {
        let emblem = gtk::Image::from_icon_name(icon_name);
        emblem.set_pixel_size(10);
        emblem.add_css_class("nautilus-emblem");

        self.imp().file_row_emblems_box.append(&emblem);
    }
}
