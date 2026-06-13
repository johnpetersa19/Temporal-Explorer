/* filter_types_dialog.rs
 *
 * Copyright 2026 John Peter Sá
 *
 * This program is free software: you can redistribute it and/or modify
 * it under the terms of the GNU General Public License as published by
 * the Free Software Foundation, either version 3 of the License, or
 * (at your option) any later version.
 *
 * This program is distributed in the hope that it will be useful,
 * but WITHOUT ANY WARRANTY; without even the implied warranty of
 * MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
 * GNU General Public License for more details.
 *
 * You should have received a copy of the GNU General Public License
 * along with this program.  If not, see <https://www.gnu.org/licenses/>.
 *
 * SPDX-License-Identifier: GPL-3.0-or-later
 */

//! `FilterTypesDialog` — file-extension picker for the search filter popover.
//!
//! Port of `nautilus-search-types-dialog`, adapted for Temporal-Explorer's
//! commit-file context:
//!
//! * A `SearchEntry` filters a curated list of extensions + descriptions.
//! * Common types are pre-shown as chips on the start state.
//! * Pressing **Add** (or activating the list) emits `file-type-selected`
//!   with the chosen extension string (e.g. `"rs"`).
//! * `SearchFilterPopover` calls `connect_file_type_selected` and appends
//!   the result to its own `FileTypeFilter`.
//!
//! # Usage
//! ```rust
//! let dialog = FilterTypesDialog::new();
//! dialog.connect_file_type_selected(|ext| { /* forward to popover */ });
//! dialog.present(Some(&window));
//! ```

use gtk::glib;
use gtk::prelude::*;
use gtk::subclass::prelude::*;
use adw::prelude::*;
use std::cell::RefCell;
use std::sync::OnceLock;

// ── Known file types ───────────────────────────────────────────────────────────────

/// (extension, human label, mime-type icon-name)
const KNOWN_TYPES: &[(&str, &str, &str)] = &[
    ("rs",    "Rust source",              "text-x-rust"),
    ("toml",  "TOML config",              "text-x-toml"),
    ("blp",   "Blueprint UI",             "text-xml"),
    ("py",    "Python script",            "text-x-python"),
    ("js",    "JavaScript",               "text-x-javascript"),
    ("ts",    "TypeScript",               "text-x-typescript"),
    ("c",     "C source",                 "text-x-csrc"),
    ("cpp",   "C++ source",               "text-x-c++src"),
    ("h",     "C/C++ header",             "text-x-chdr"),
    ("go",    "Go source",                "text-x-go"),
    ("java",  "Java source",              "text-x-java"),
    ("kt",    "Kotlin source",            "text-x-kotlin"),
    ("swift", "Swift source",             "text-x-swift"),
    ("sh",    "Shell script",             "text-x-script"),
    ("md",    "Markdown",                 "text-x-markdown"),
    ("json",  "JSON data",                "text-x-json"),
    ("yaml",  "YAML config",              "text-x-yaml"),
    ("xml",   "XML document",             "text-xml"),
    ("sql",   "SQL query",                "text-x-sql"),
    ("html",  "HTML document",            "text-html"),
    ("css",   "CSS stylesheet",           "text-css"),
    ("txt",   "Plain text",               "text-plain"),
    ("lock",  "Lock file (Cargo/npm)",    "text-plain"),
    ("png",   "PNG image",               "image-png"),
    ("svg",   "SVG vector image",        "image-svg+xml"),
];

/// Extensions shown on the \"start\" (pre-search) state as quick chips.
const COMMON_EXTENSIONS: &[&str] = &["rs", "toml", "blp", "py", "js", "md", "json", "sh"];

// ── GObject subclass ───────────────────────────────────────────────────────────────

mod imp {
    use super::*;

    #[derive(Debug, Default, gtk::CompositeTemplate)]
    #[template(resource = "/io/github/johnpetersa19/TemporalExplorer/filter-types-dialog.ui")]
    pub struct FilterTypesDialog {
        #[template_child] pub search_entry:      TemplateChild<gtk::SearchEntry>,
        #[template_child] pub search_stack:      TemplateChild<gtk::Stack>,
        #[template_child] pub common_types_box:  TemplateChild<gtk::FlowBox>,
        #[template_child] pub results_list:      TemplateChild<gtk::ListView>,
        #[template_child] pub add_button:        TemplateChild<gtk::Button>,
        #[template_child] pub cancel_button:     TemplateChild<gtk::Button>,

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
                vec![
                    glib::subclass::Signal::builder("file-type-selected")
                        .param_types([String::static_type()])
                        .build(),
                ]
            })
        }
    }

    impl WidgetImpl     for FilterTypesDialog {}
    impl AdwDialogImpl  for FilterTypesDialog {}

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
                let ext   = query.trim_start_matches('.');
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
                    self.obj().emit_by_name::<()>(
                        "file-type-selected",
                        &[&ext_str.to_value()],
                    );
                    self.obj().close();
                }
            }
        }
    }
}

// ── Public wrapper ───────────────────────────────────────────────────────────────

glib::wrapper! {
    pub struct FilterTypesDialog(ObjectSubclass<imp::FilterTypesDialog>)
        @extends adw::Dialog, gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl Default for FilterTypesDialog {
    fn default() -> Self { Self::new() }
}

impl FilterTypesDialog {
    pub fn new() -> Self {
        glib::Object::new()
    }

    // ── Public API ──────────────────────────────────────────────────────────────────

    pub fn connect_file_type_selected<F>(&self, f: F) -> glib::SignalHandlerId
    where
        F: Fn(&Self, &str) + 'static,
    {
        self.connect_local("file-type-selected", false, move |values| {
            let dialog = values[0].get::<FilterTypesDialog>().unwrap();
            let ext    = values[1].get::<String>().unwrap();
            f(&dialog, &ext);
            None
        })
    }

    // ── Internal setup ──────────────────────────────────────────────────────────────

    fn setup(&self) {
        let imp = self.imp();

        // ── SizeGroup: keep cancel and add buttons equal-width ─────────────────
        // (replaces the SizeGroup removed from filter-types-dialog.blp because
        // Blueprint 0.12 does not allow bare object blocks in a template body)
        let size_group = gtk::SizeGroup::new(gtk::SizeGroupMode::Horizontal);
        size_group.add_widget(&imp.cancel_button.get());
        size_group.add_widget(&imp.add_button.get());

        // ── Wire StringList model to the ListView ───────────────────────────
        let selection_model = gtk::NoSelection::new(Some(imp.model.clone()));
        imp.results_list.set_model(Some(&selection_model));

        self.setup_common_chips();
        self.update_results("");
    }

    fn setup_common_chips(&self) {
        let flow = self.imp().common_types_box.get();
        for &ext in COMMON_EXTENSIONS {
            let btn = gtk::Button::builder()
                .label(&format!(".{ext}"))
                .css_classes(["chip"])
                .build();
            let child = gtk::FlowBoxChild::builder()
                .child(&btn)
                .build();
            flow.append(&child);
        }
    }

    fn update_results(&self, query: &str) {
        let imp = self.imp();

        let matches: Vec<_> = if query.is_empty() {
            KNOWN_TYPES.iter().copied().collect()
        } else {
            KNOWN_TYPES.iter().copied().filter(|(ext, label, _)| {
                ext.contains(query) || label.to_lowercase().contains(query)
            }).collect()
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

        // Rebuild the StringList model
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
