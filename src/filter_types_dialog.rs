/* filter_types_dialog.rs
 *
 * Copyright 2026 John Peter Sá
 *
 * SPDX-License-Identifier: GPL-3.0-or-later
 */

//! `FilterTypesDialog` — file-extension picker for the search filter popover.

use adw::prelude::*;
use adw::subclass::prelude::*;
use gettextrs::gettext;
use gtk::{gio, glib};
use std::cell::RefCell;
use std::sync::OnceLock;

// ── Known file types ─────────────────────────────────────────────────────────────────────────

/// (extension, human label, mime-type icon-name)
const KNOWN_TYPES: &[(&str, &str, &str)] = &[
    ("rs", "Rust source", "text-x-generic"),
    ("toml", "TOML config", "text-x-script"),
    ("blp", "Blueprint UI", "text-xml"),
    ("py", "Python script", "text-x-script"),
    ("js", "JavaScript", "text-x-script"),
    ("ts", "TypeScript", "text-x-script"),
    ("c", "C source", "text-x-generic"),
    ("cpp", "C++ source", "text-x-generic"),
    ("h", "C/C++ header", "text-x-generic"),
    ("go", "Go source", "text-x-generic"),
    ("java", "Java source", "text-x-generic"),
    ("kt", "Kotlin source", "text-x-generic"),
    ("swift", "Swift source", "text-x-generic"),
    ("sh", "Shell script", "text-x-script"),
    ("md", "Markdown", "text-x-generic"),
    ("json", "JSON data", "text-x-script"),
    ("yaml", "YAML config", "text-x-script"),
    ("xml", "XML document", "text-xml"),
    ("sql", "SQL query", "text-x-generic"),
    ("html", "HTML document", "text-html"),
    ("css", "CSS stylesheet", "text-html"),
    ("txt", "Plain text", "text-x-generic"),
    ("lock", "Lock file (Cargo/npm)", "text-x-generic"),
    ("png", "PNG image", "image-x-generic"),
    ("svg", "SVG vector image", "image-x-generic"),
];

/// Extensions shown on the start state as quick chips.
const COMMON_EXTENSIONS: &[&str] = &["rs", "toml", "blp", "py", "js", "md", "json", "sh"];

fn translated_file_type_label(label: &str) -> String {
    match label {
        "Rust source" => gettext("Rust source"),
        "TOML config" => gettext("TOML config"),
        "Blueprint UI" => gettext("Blueprint UI"),
        "Python script" => gettext("Python script"),
        "JavaScript" => gettext("JavaScript"),
        "TypeScript" => gettext("TypeScript"),
        "C source" => gettext("C source"),
        "C++ source" => gettext("C++ source"),
        "C/C++ header" => gettext("C/C++ header"),
        "Go source" => gettext("Go source"),
        "Java source" => gettext("Java source"),
        "Kotlin source" => gettext("Kotlin source"),
        "Swift source" => gettext("Swift source"),
        "Shell script" => gettext("Shell script"),
        "Markdown" => gettext("Markdown"),
        "JSON data" => gettext("JSON data"),
        "YAML config" => gettext("YAML config"),
        "XML document" => gettext("XML document"),
        "SQL query" => gettext("SQL query"),
        "HTML document" => gettext("HTML document"),
        "CSS stylesheet" => gettext("CSS stylesheet"),
        "Plain text" => gettext("Plain text"),
        "Lock file (Cargo/npm)" => gettext("Lock file (Cargo/npm)"),
        "PNG image" => gettext("PNG image"),
        "SVG vector image" => gettext("SVG vector image"),
        other => other.to_string(),
    }
}

// ── GObject subclass ─────────────────────────────────────────────────────────────────────────

mod imp {
    use super::*;

    #[derive(Debug, Default, gtk::CompositeTemplate)]
    #[template(resource = "/io/github/johnpetersa19/TemporalExplorer/filter-types-dialog.ui")]
    pub struct FilterTypesDialog {
        #[template_child]
        pub search_entry: TemplateChild<gtk::SearchEntry>,
        #[template_child]
        pub search_stack: TemplateChild<gtk::Stack>,
        #[template_child]
        pub common_types_box: TemplateChild<gtk::FlowBox>,
        #[template_child]
        pub results_list: TemplateChild<gtk::ListView>,
        #[template_child]
        pub add_button: TemplateChild<gtk::Button>,
        #[template_child]
        pub cancel_button: TemplateChild<gtk::Button>,

        /// Currently selected extension (drives `add_button` sensitivity).
        pub selected_ext: RefCell<Option<String>>,

        /// String list model backing the ListView.
        pub model: gtk::StringList,

        /// Parallel vec of `(ext, label, icon)` matching the filtered model.
        pub filtered: RefCell<Vec<(&'static str, &'static str, &'static str)>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for FilterTypesDialog {
        const NAME: &'static str = "FilterTypesDialog";
        type Type = super::FilterTypesDialog;
        type ParentType = adw::Dialog;

        fn class_init(klass: &mut Self::Class) {
            klass.bind_template();
            klass.bind_template_callbacks();
        }

        fn instance_init(obj: &glib::subclass::InitializingObject<Self>) {
            obj.init_template();
        }
    }

    impl ObjectImpl for FilterTypesDialog {
        fn constructed(&self) {
            self.parent_constructed();
            self.obj().setup();
        }

        fn signals() -> &'static [glib::subclass::Signal] {
            static SIGNALS: OnceLock<Vec<glib::subclass::Signal>> = OnceLock::new();
            SIGNALS.get_or_init(|| {
                vec![glib::subclass::Signal::builder("file-type-selected")
                    .param_types([String::static_type()])
                    .build()]
            })
        }
    }

    impl WidgetImpl for FilterTypesDialog {}
    impl AdwDialogImpl for FilterTypesDialog {}

    #[gtk::template_callbacks]
    impl FilterTypesDialog {
        #[template_callback]
        fn on_cancel_clicked(&self) {
            self.obj().close();
        }

        #[template_callback]
        fn on_add_clicked(&self) {
            self.obj().emit_selected();
        }

        #[template_callback]
        fn on_search_changed(&self) {
            let query = self.search_entry.text().to_lowercase();
            self.obj().update_results(&query);
        }

        #[template_callback]
        fn on_search_activate(&self) {
            let filtered = self.filtered.borrow();
            if filtered.len() == 1 {
                drop(filtered);
                self.obj().emit_selected();
            } else {
                let query = self.search_entry.text().to_string();
                let ext = query.trim_start_matches('.');
                if !ext.is_empty() {
                    self.obj().emit_by_name::<()>(
                        "file-type-selected",
                        &[&ext.to_lowercase().to_value()],
                    );
                    self.obj().close();
                }
            }
        }

        #[template_callback]
        fn on_list_activate(&self, pos: u32) {
            let filtered = self.filtered.borrow();
            if let Some((ext, _, _)) = filtered.get(pos as usize) {
                *self.selected_ext.borrow_mut() = Some(ext.to_string());
                drop(filtered);
                self.obj().emit_selected();
            }
        }

        #[template_callback]
        fn on_common_type_activated(&self, child: &gtk::FlowBoxChild) {
            if let Some(btn) = child.child().and_downcast::<gtk::Button>() {
                if let Some(ext) = btn.label() {
                    let ext_str = ext.trim_start_matches('.').to_string();
                    self.obj()
                        .emit_by_name::<()>("file-type-selected", &[&ext_str.to_value()]);
                    self.obj().close();
                }
            }
        }
    }
}

// ── Public wrapper ───────────────────────────────────────────────────────────────────────

glib::wrapper! {
    pub struct FilterTypesDialog(ObjectSubclass<imp::FilterTypesDialog>)
        @extends adw::Dialog, gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl Default for FilterTypesDialog {
    fn default() -> Self {
        Self::new()
    }
}

impl FilterTypesDialog {
    pub fn new() -> Self {
        glib::Object::new()
    }

    pub fn connect_file_type_selected<F>(&self, f: F) -> glib::SignalHandlerId
    where
        F: Fn(&Self, &str) + 'static,
    {
        self.connect_local("file-type-selected", false, move |values| {
            let dialog = values[0].get::<FilterTypesDialog>().unwrap();
            let ext = values[1].get::<String>().unwrap();
            f(&dialog, &ext);
            None
        })
    }

    // ── Internal setup ──────────────────────────────────────────────────────────────────────

    fn setup(&self) {
        let imp = self.imp();

        // Equal-width cancel/add buttons
        let size_group = gtk::SizeGroup::new(gtk::SizeGroupMode::Horizontal);
        size_group.add_widget(&imp.cancel_button.get());
        size_group.add_widget(&imp.add_button.get());

        // Wire model to ListView
        let selection_model = gtk::NoSelection::new(Some(imp.model.clone()));
        imp.results_list.set_model(Some(&selection_model));

        // Build row factory via SignalListItemFactory
        self.setup_factory();

        self.setup_common_chips();
        self.update_results("");
    }

    fn setup_factory(&self) {
        let imp = self.imp();
        let factory = gtk::SignalListItemFactory::new();

        // setup: create the row widget skeleton once per recycled slot
        factory.connect_setup(|_, list_item| {
            let list_item = list_item.downcast_ref::<gtk::ListItem>().unwrap();

            let row_icon = gtk::Image::builder().pixel_size(32).build();

            let row_description = gtk::Label::builder()
                .halign(gtk::Align::Start)
                .hexpand(true)
                .ellipsize(gtk::pango::EllipsizeMode::End)
                .build();
            row_description.add_css_class("body");

            let row_subtitle = gtk::Label::builder()
                .halign(gtk::Align::Start)
                .hexpand(true)
                .ellipsize(gtk::pango::EllipsizeMode::End)
                .build();
            row_subtitle.add_css_class("caption");
            row_subtitle.add_css_class("dim-label");

            let text_box = gtk::Box::builder()
                .orientation(gtk::Orientation::Vertical)
                .spacing(4)
                .hexpand(true)
                .build();
            text_box.append(&row_description);
            text_box.append(&row_subtitle);

            let row_checkmark = gtk::Image::builder()
                .icon_name("object-select-symbolic")
                .pixel_size(16)
                .visible(false)
                .build();
            row_checkmark.add_css_class("success");

            let hbox = gtk::Box::builder()
                .orientation(gtk::Orientation::Horizontal)
                .spacing(12)
                .margin_start(16)
                .margin_end(16)
                .margin_top(14)
                .margin_bottom(14)
                .build();
            hbox.append(&row_icon);
            hbox.append(&text_box);
            hbox.append(&row_checkmark);

            list_item.set_child(Some(&hbox));
        });

        // bind: fill data from the model item into the row widgets
        let filtered_ref = imp.filtered.clone();
        let selected_ref = imp.selected_ext.clone();
        factory.connect_bind(move |_, list_item| {
            let list_item = list_item.downcast_ref::<gtk::ListItem>().unwrap();
            let pos = list_item.position() as usize;

            let filtered = filtered_ref.borrow();
            let Some(&(ext, label, _icon)) = filtered.get(pos) else {
                return;
            };

            let hbox = list_item.child().and_downcast::<gtk::Box>().unwrap();

            // row_icon (index 0)
            if let Some(img) = hbox.first_child().and_downcast::<gtk::Image>() {
                let filename = format!("file.{ext}");
                let content_type =
                    gio::content_type_guess(Some(std::path::Path::new(&filename)), &[]).0;
                let icon = gio::content_type_get_icon(&content_type);
                img.set_from_gicon(&icon);
            }

            // text_box (index 1) → description + subtitle
            if let Some(text_box) = hbox
                .first_child()
                .and_then(|w| w.next_sibling())
                .and_downcast::<gtk::Box>()
            {
                if let Some(desc) = text_box.first_child().and_downcast::<gtk::Label>() {
                    let translated = translated_file_type_label(label);
                    desc.set_label(&format!("{translated} (.{ext})"));
                }
                if let Some(sub) = text_box
                    .first_child()
                    .and_then(|w| w.next_sibling())
                    .and_downcast::<gtk::Label>()
                {
                    sub.set_label(&format!(".{ext}"));
                }
            }

            // row_checkmark (index 2): show when this ext is the selected one
            if let Some(checkmark) = hbox
                .first_child()
                .and_then(|w| w.next_sibling())
                .and_then(|w| w.next_sibling())
                .and_downcast::<gtk::Image>()
            {
                let is_selected = selected_ref.borrow().as_deref().map_or(false, |s| s == ext);
                checkmark.set_visible(is_selected);
            }
        });

        imp.results_list.set_factory(Some(&factory));
    }

    fn setup_common_chips(&self) {
        let flow = self.imp().common_types_box.get();
        for &ext in COMMON_EXTENSIONS {
            let btn = gtk::Button::builder()
                .label(&format!(".{ext}"))
                .css_classes(["chip"])
                .build();
            let child = gtk::FlowBoxChild::builder().child(&btn).build();
            flow.append(&child);
        }
    }

    fn update_results(&self, query: &str) {
        let imp = self.imp();

        let matches: Vec<_> = if query.is_empty() {
            KNOWN_TYPES.iter().copied().collect()
        } else {
            KNOWN_TYPES
                .iter()
                .copied()
                .filter(|(ext, label, _)| {
                    ext.contains(query)
                        || label.to_lowercase().contains(query)
                        || translated_file_type_label(label)
                            .to_lowercase()
                            .contains(query)
                })
                .collect()
        };

        *imp.filtered.borrow_mut() = matches.clone();

        if query.is_empty() {
            imp.search_stack.set_visible_child_name("start");
            return;
        }

        if matches.is_empty() {
            imp.search_stack.set_visible_child_name("empty");
            imp.add_button.set_sensitive(false);
            *imp.selected_ext.borrow_mut() = None;
            return;
        }

        // Rebuild StringList
        while imp.model.n_items() > 0 {
            imp.model.remove(0);
        }
        for (ext, label, _) in &matches {
            imp.model.append(&format!("{label} (.{ext})"));
        }

        // Auto-select first result
        *imp.selected_ext.borrow_mut() = Some(matches[0].0.to_string());
        imp.add_button.set_sensitive(true);
        imp.search_stack.set_visible_child_name("results");
    }

    fn emit_selected(&self) {
        if let Some(ext) = self.imp().selected_ext.borrow().clone() {
            self.emit_by_name::<()>("file-type-selected", &[&ext.to_value()]);
            self.close();
        }
    }
}
