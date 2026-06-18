/* search_filter_popover.rs
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
 * You should have received a copy of the GNU General Public License/home/john/Projects/Temporal-Explorer/src
 * along with this program.  If not, see <https://www.gnu.org/licenses/>.
 *
 * SPDX-License-Identifier: GPL-3.0-or-later
 */

//! `SearchFilterPopover` — advanced search/filter panel.
//!
//! Inspired by `nautilus-search-popover` (Nautilus 47+), adapted for
//! Git-commit exploration:
//!
//! * **Date** — quick chips (Today / Yesterday / Past Week / Month / Year)
//!              plus a "…" button that opens [`DateRangeDialog`].
//! * **Author** — free-text entry *plus* auto-generated chips from
//!                the unique authors in the current commit list.
//! * **Branch** — chips populated from `git branch -a` on repo open.
//! * **Changed files** — chip toggles for Rust / TOML / Blueprint / other.
//!
//! The popover emits a `filters-changed` signal carrying a [`FilterState`]
//! struct.  `window.rs` listens to it and re-runs `run_search()` with the
//! active filter applied on top of the text query.

use adw::prelude::*;
use gtk::glib;
use gtk::prelude::*;
use gtk::subclass::prelude::*;
use std::cell::RefCell;
use std::sync::OnceLock;

use crate::date_range_dialog::DateRangeDialog;
use crate::filter_types_dialog::FilterTypesDialog;
use crate::git_engine::CommitInfo;

// ── FilterDateRange ────────────────────────────────────────────────────────────

/// A half-open date range `[from, to)` expressed as Unix timestamps.
/// Both ends are optional; `None` means "unbounded".
#[derive(Debug, Clone, Default, PartialEq)]
pub struct FilterDateRange {
    pub from: Option<i64>,
    pub to: Option<i64>,
}

impl FilterDateRange {
    /// Returns `true` when at least one bound is set.
    pub fn is_active(&self) -> bool {
        self.from.is_some() || self.to.is_some()
    }

    /// Returns `true` when `ts` falls inside the range.
    pub fn contains(&self, ts: i64) -> bool {
        let after = self.from.map_or(true, |f| ts >= f);
        let before = self.to.map_or(true, |t| ts < t);
        after && before
    }

    // ── Preset constructors ────────────────────────────────────────────────

    pub fn today() -> Self {
        let now = glib::DateTime::now_local().unwrap();
        let start = glib::DateTime::new(
            &glib::TimeZone::local(),
            now.year(),
            now.month(),
            now.day_of_month(),
            0,
            0,
            0.0,
        )
        .unwrap();
        let end = start.add_days(1).unwrap();
        Self {
            from: Some(start.to_unix()),
            to: Some(end.to_unix()),
        }
    }

    pub fn yesterday() -> Self {
        let now = glib::DateTime::now_local().unwrap();
        let start = glib::DateTime::new(
            &glib::TimeZone::local(),
            now.year(),
            now.month(),
            now.day_of_month(),
            0,
            0,
            0.0,
        )
        .unwrap()
        .add_days(-1)
        .unwrap();
        let end = start.add_days(1).unwrap();
        Self {
            from: Some(start.to_unix()),
            to: Some(end.to_unix()),
        }
    }

    pub fn last_n_days(n: i32) -> Self {
        let now = glib::DateTime::now_local().unwrap();
        let start = now.add_days(-n).unwrap();
        Self {
            from: Some(start.to_unix()),
            to: None,
        }
    }

    /// Build from raw Unix bounds as returned by `DateRangeDialog`.
    /// `i64::MIN` / `i64::MAX` sentinels are normalised back to `None`.
    pub fn from_unix_bounds(from: i64, to: i64) -> Self {
        Self {
            from: if from == i64::MIN { None } else { Some(from) },
            to: if to == i64::MAX { None } else { Some(to) },
        }
    }

    /// Human-readable label for the chip, e.g. "2026-01-01 → 2026-03-15".
    pub fn chip_label(&self) -> String {
        let fmt = |ts: i64| -> String {
            glib::DateTime::from_unix_local(ts)
                .map(|dt| {
                    format!(
                        "{:04}-{:02}-{:02}",
                        dt.year(),
                        dt.month(),
                        dt.day_of_month()
                    )
                })
                .unwrap_or_else(|_| "?".into())
        };

        match (&self.from, &self.to) {
            (Some(f), Some(t)) => format!("{} → {}", fmt(*f), fmt(*t)),
            (Some(f), None) => format!("From {}", fmt(*f)),
            (None, Some(t)) => format!("Until {}", fmt(*t)),
            (None, None) => String::new(),
        }
    }
}

// ── FileTypeFilter ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Default, PartialEq)]
pub struct FileTypeFilter {
    pub audio: bool,
    pub documents: bool,
    pub folders: bool,
    pub images: bool,
    pub pdf: bool,
    pub text: bool,
    pub videos: bool,
    pub other_ext: Option<String>,
}

impl FileTypeFilter {
    pub fn is_active(&self) -> bool {
        self.audio
            || self.documents
            || self.folders
            || self.images
            || self.pdf
            || self.text
            || self.videos
            || self.other_ext.is_some()
    }
}

fn file_matches_type_filter(path: &str, filter: &FileTypeFilter) -> bool {
    let path_obj = std::path::Path::new(path);

    let ext = path_obj
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();

    // Git changed-files are file paths. A path containing a parent directory is
    // treated as matching "Folders" because the commit touched something inside
    // a folder.
    let in_folder = path_obj.parent().is_some_and(|parent| !parent.as_os_str().is_empty());

    let audio_ext = matches!(
        ext.as_str(),
        "mp3" | "flac" | "wav" | "ogg" | "opus" | "m4a" | "aac" | "mid" | "midi"
    );

    let document_ext = matches!(
        ext.as_str(),
        "doc" | "docx" | "odt" | "ott" | "rtf" | "abw" | "pages"
    );

    let image_ext = matches!(
        ext.as_str(),
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "svg" | "bmp" | "tif" | "tiff" | "heic" | "avif"
    );

    let pdf_ext = ext == "pdf";

    let text_ext = matches!(
        ext.as_str(),
        "txt" | "md" | "markdown" | "rst" | "log" | "csv" | "json" | "jsonc" |
        "yaml" | "yml" | "toml" | "xml" | "html" | "css" | "scss" | "js" |
        "ts" | "jsx" | "tsx" | "rs" | "c" | "h" | "cpp" | "hpp" | "cc" |
        "py" | "sh" | "bash" | "zsh" | "fish" | "go" | "java" | "kt" |
        "swift" | "php" | "rb" | "lua" | "blp" | "ui" | "desktop" | "service"
    );

    let video_ext = matches!(
        ext.as_str(),
        "mp4" | "mkv" | "webm" | "mov" | "avi" | "m4v" | "flv" | "wmv" | "mpeg" | "mpg"
    );

    (filter.audio && audio_ext)
        || (filter.documents && document_ext)
        || (filter.folders && in_folder)
        || (filter.images && image_ext)
        || (filter.pdf && pdf_ext)
        || (filter.text && text_ext)
        || (filter.videos && video_ext)
        || filter
            .other_ext
            .as_deref()
            .map_or(false, |wanted| ext == wanted.trim_start_matches('.').to_lowercase())
}

// ── FilterState ────────────────────────────────────────────────────────────────

/// All active filter constraints from the popover.
/// `window.rs` applies this on top of the free-text query in `run_search()`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct FilterState {
    pub date: FilterDateRange,
    pub author: Option<String>,
    pub branch: Option<String>,
    pub files: FileTypeFilter,
}

impl FilterState {
    /// `true` when at least one filter is active.
    pub fn is_active(&self) -> bool {
        self.date.is_active()
            || self.author.is_some()
            || self.branch.is_some()
            || self.files.is_active()
    }

    /// Alias used by `window.rs`.
    pub fn is_empty(&self) -> bool {
        !self.is_active()
    }

    /// Apply this filter to a slice of commits, returning only matching ones.
    pub fn apply<'a>(&self, commits: &'a [CommitInfo]) -> Vec<&'a CommitInfo> {
        commits.iter().filter(|c| self.matches(c)).collect()
    }

    pub fn matches(&self, c: &CommitInfo) -> bool {
        // Date filter
        if self.date.is_active() && !self.date.contains(c.timestamp) {
            return false;
        }

        // Author filter (case-insensitive substring)
        if let Some(ref author) = self.author {
            let a = author.to_lowercase();
            if !c.author.to_lowercase().contains(&a) {
                return false;
            }
        }

        // File-type filter: commit must have touched at least one matching file.
        if self.files.is_active() {
            let has_match = c
                .changed_files
                .iter()
                .any(|path| file_matches_type_filter(path, &self.files));

            if !has_match {
                return false;
            }
        }

        // Branch filter is applied at the repo/query level, not per-commit,
        // so we skip it here (handled in window.rs during repo load).

        true
    }
}

// ── GObject subclass ───────────────────────────────────────────────────────────

mod imp {
    use super::*;

    #[derive(Debug, Default, gtk::CompositeTemplate)]
    #[template(resource = "/io/github/johnpetersa19/TemporalExplorer/search-filter-popover.ui")]
    pub struct SearchFilterPopover {
        // ── Date section ──
        #[template_child]
        pub clear_date_button: TemplateChild<gtk::Button>,
        #[template_child]
        pub date_chips_box: TemplateChild<adw::WrapBox>,
        #[template_child]
        pub today_button: TemplateChild<gtk::Button>,
        #[template_child]
        pub yesterday_button: TemplateChild<gtk::Button>,
        #[template_child]
        pub last_week_button: TemplateChild<gtk::Button>,
        #[template_child]
        pub last_month_button: TemplateChild<gtk::Button>,
        #[template_child]
        pub last_year_button: TemplateChild<gtk::Button>,
        #[template_child]
        pub custom_range_chip: TemplateChild<gtk::Button>,
        #[template_child]
        pub custom_range_button: TemplateChild<gtk::Button>,

        // ── Author section ──
        #[template_child]
        pub clear_author_button: TemplateChild<gtk::Button>,
        #[template_child]
        pub author_entry: TemplateChild<gtk::SearchEntry>,
        #[template_child]
        pub author_chips_box: TemplateChild<gtk::Box>,

        // ── Branch section ──
        #[template_child]
        pub clear_branch_button: TemplateChild<gtk::Button>,
        #[template_child]
        pub branch_chips_box: TemplateChild<adw::WrapBox>,

        // ── File types section ──
        #[template_child]
        pub file_type_audio_button: TemplateChild<gtk::Button>,
        #[template_child]
        pub file_type_documents_button: TemplateChild<gtk::Button>,
        #[template_child]
        pub file_type_folders_button: TemplateChild<gtk::Button>,
        #[template_child]
        pub file_type_images_button: TemplateChild<gtk::Button>,
        #[template_child]
        pub file_type_pdf_button: TemplateChild<gtk::Button>,
        #[template_child]
        pub file_type_text_button: TemplateChild<gtk::Button>,
        #[template_child]
        pub file_type_videos_button: TemplateChild<gtk::Button>,
        #[template_child]
        pub file_type_other_button: TemplateChild<gtk::Button>,
        #[template_child]
        pub file_type_chips_box: TemplateChild<adw::WrapBox>,

        // ── Footer ──
        #[template_child]
        pub reset_all_button: TemplateChild<gtk::Button>,

        // ── Internal state ──
        pub filter_state: RefCell<FilterState>,
        /// Full author list used by the author SearchEntry to filter visible rows.
        pub all_authors: RefCell<Vec<String>>,
        /// The `DateRangeDialog` instance; kept alive so signals don't disconnect.
        pub date_range_dialog: RefCell<Option<DateRangeDialog>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for SearchFilterPopover {
        const NAME: &'static str = "SearchFilterPopover";
        type Type = super::SearchFilterPopover;
        type ParentType = gtk::Popover;

        fn class_init(klass: &mut Self::Class) {
            klass.bind_template();
            klass.bind_template_callbacks();
        }

        fn instance_init(obj: &glib::subclass::InitializingObject<Self>) {
            obj.init_template();
        }
    }

    impl ObjectImpl for SearchFilterPopover {
        fn constructed(&self) {
            self.parent_constructed();
            self.obj().setup_callbacks();
        }

        fn signals() -> &'static [glib::subclass::Signal] {
            static SIGNALS: OnceLock<Vec<glib::subclass::Signal>> = OnceLock::new();
            SIGNALS.get_or_init(|| vec![glib::subclass::Signal::builder("filters-changed").build()])
        }
    }

    impl WidgetImpl for SearchFilterPopover {}
    impl PopoverImpl for SearchFilterPopover {}

    #[gtk::template_callbacks]
    impl SearchFilterPopover {}
}

// ── Public wrapper ─────────────────────────────────────────────────────────────

glib::wrapper! {
    pub struct SearchFilterPopover(ObjectSubclass<imp::SearchFilterPopover>)
        @extends gtk::Popover, gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget,
                    gtk::Native, gtk::ShortcutManager;
}

impl Default for SearchFilterPopover {
    fn default() -> Self {
        Self::new()
    }
}

impl SearchFilterPopover {
    pub fn new() -> Self {
        glib::Object::new()
    }

    // ── Public API ─────────────────────────────────────────────────────────

    /// Returns a clone of the current [`FilterState`].
    pub fn filter_state(&self) -> FilterState {
        self.imp().filter_state.borrow().clone()
    }

    /// Alias used by `window.rs` via `connect_local("filters-changed", ...)`.
    pub fn current_filter(&self) -> FilterState {
        self.filter_state()
    }

    /// Populate the complete author list.
    /// The SearchEntry filters this stored list visually; clicking a row applies
    /// the actual author filter to the repository/timeline.
    pub fn populate_author_chips(&self, authors: &[String]) {
        let imp = self.imp();

        let mut normalized: Vec<String> = authors
            .iter()
            .map(|author| author.trim().to_string())
            .filter(|author| !author.is_empty())
            .collect();

        normalized.sort_by_key(|author| author.to_lowercase());
        normalized.dedup_by(|a, b| a.eq_ignore_ascii_case(b));

        *imp.all_authors.borrow_mut() = normalized;

        let query = imp.author_entry.text().to_string();
        self.rebuild_author_rows(&query);
    }

    fn apply_author_row_search(&self, query: &str) {
        let imp = self.imp();
        let query = query.trim().to_lowercase();
        let chips_box = imp.author_chips_box.get();

        let mut child = chips_box.first_child();
        while let Some(widget) = child {
            let next = widget.next_sibling();

            if let Some(button) = widget.downcast_ref::<gtk::Button>() {
                let author = button.widget_name().to_string();
                let visible = query.is_empty() || author.to_lowercase().starts_with(&query);
                button.set_visible(visible);
            }

            child = next;
        }
    }

    fn rebuild_author_rows(&self, query: &str) {
        let imp = self.imp();
        let chips_box = imp.author_chips_box.get();

        while let Some(child) = chips_box.first_child() {
            chips_box.remove(&child);
        }

        let query = query.trim().to_lowercase();
        let selected_author = imp.filter_state.borrow().author.clone();
        let authors = imp.all_authors.borrow().clone();

        for author in authors {
            if !query.is_empty() && !author.to_lowercase().starts_with(&query) {
                continue;
            }

            let chip = gtk::Button::builder()
                .label(&author)
                .hexpand(true)
                .halign(gtk::Align::Fill)
                .css_classes(["chip"])
                .build();

            chip.set_widget_name(&author);
            chip.set_tooltip_text(Some(&author));

            if selected_author
                .as_deref()
                .map(|selected| selected.eq_ignore_ascii_case(&author))
                .unwrap_or(false)
            {
                chip.add_css_class("suggested-action");
            }

            if let Some(label) = chip.child().and_then(|w| w.downcast::<gtk::Label>().ok()) {
                label.set_xalign(0.0);
                label.set_ellipsize(gtk::pango::EllipsizeMode::End);
            }

            let popover = self.clone();
            let author_clone = author.clone();
            chip.connect_clicked(move |btn| {
                let imp = popover.imp();
                let mut state = imp.filter_state.borrow_mut();

                if state
                    .author
                    .as_deref()
                    .map(|selected| selected.eq_ignore_ascii_case(&author_clone))
                    .unwrap_or(false)
                {
                    state.author = None;
                    btn.remove_css_class("suggested-action");
                    imp.clear_author_button.set_visible(false);
                } else {
                    state.author = Some(author_clone.clone());
                    imp.clear_author_button.set_visible(true);
                }

                drop(state);

                let selected = popover.imp().filter_state.borrow().author.clone();
                let chips_box = popover.imp().author_chips_box.get();

                let mut child = chips_box.first_child();
                while let Some(widget) = child {
                    let next = widget.next_sibling();

                    if let Some(button) = widget.downcast_ref::<gtk::Button>() {
                        let author = button.widget_name().to_string();
                        let is_selected = selected
                            .as_deref()
                            .map(|selected| selected.eq_ignore_ascii_case(&author))
                            .unwrap_or(false);

                        if is_selected {
                            button.add_css_class("suggested-action");
                        } else {
                            button.remove_css_class("suggested-action");
                        }
                    }

                    child = next;
                }

                popover.emit_filters_changed();
            });

            chips_box.append(&chip);
        }
    }

    /// Populate branch chip buttons.
    /// `branches` should be the list of local + remote branch names.
    pub fn populate_branch_chips(&self, branches: &[String]) {
        let imp = self.imp();
        let chips_box = imp.branch_chips_box.get();

        while let Some(child) = chips_box.first_child() {
            chips_box.remove(&child);
        }

        for branch in branches {
            let chip = gtk::Button::builder()
                .label(branch)
                .css_classes(["chip"])
                .build();

            let popover = self.clone();
            let branch_clone = branch.clone();
            chip.connect_clicked(move |btn| {
                let imp = popover.imp();
                let mut state = imp.filter_state.borrow_mut();

                if state.branch.as_deref() == Some(&branch_clone) {
                    state.branch = None;
                    btn.remove_css_class("suggested-action");
                    imp.clear_branch_button.set_visible(false);
                } else {
                    let mut sibling = imp.branch_chips_box.first_child();
                    while let Some(w) = sibling {
                        if let Some(b) = w.downcast_ref::<gtk::Button>() {
                            b.remove_css_class("suggested-action");
                        }
                        sibling = w.next_sibling();
                    }

                    state.branch = Some(branch_clone.clone());
                    btn.add_css_class("suggested-action");
                    imp.clear_branch_button.set_visible(true);
                }
                drop(state);
                popover.emit_filters_changed();
            });

            chips_box.append(&chip);
        }
    }

    /// Synchronize file-extension selection coming from FilterTypesDialog.
    pub fn set_file_ext_filter(&self, ext: &str) {
        let imp = self.imp();
        let normalized = ext.trim().trim_start_matches('.').to_lowercase();

        for btn in [
            imp.file_type_audio_button.get(),
            imp.file_type_documents_button.get(),
            imp.file_type_folders_button.get(),
            imp.file_type_images_button.get(),
            imp.file_type_pdf_button.get(),
            imp.file_type_text_button.get(),
            imp.file_type_videos_button.get(),
            imp.file_type_other_button.get(),
        ] {
            btn.remove_css_class("suggested-action");
        }

        let mut state = imp.filter_state.borrow_mut();
        state.files = FileTypeFilter::default();

        match normalized.as_str() {
            "" => {}
            "mp3" | "flac" | "wav" | "ogg" | "opus" | "m4a" | "aac" | "mid" | "midi" => {
                state.files.audio = true;
                imp.file_type_audio_button.add_css_class("suggested-action");
            }
            "doc" | "docx" | "odt" | "ott" | "rtf" | "abw" | "pages" => {
                state.files.documents = true;
                imp.file_type_documents_button.add_css_class("suggested-action");
            }
            "png" | "jpg" | "jpeg" | "gif" | "webp" | "svg" | "bmp" | "tif" | "tiff" | "heic" | "avif" => {
                state.files.images = true;
                imp.file_type_images_button.add_css_class("suggested-action");
            }
            "pdf" => {
                state.files.pdf = true;
                imp.file_type_pdf_button.add_css_class("suggested-action");
            }
            "txt" | "md" | "markdown" | "rst" | "log" | "csv" | "json" | "jsonc" |
            "yaml" | "yml" | "toml" | "xml" | "html" | "css" | "scss" | "js" |
            "ts" | "jsx" | "tsx" | "rs" | "c" | "h" | "cpp" | "hpp" | "cc" |
            "py" | "sh" | "bash" | "zsh" | "fish" | "go" | "java" | "kt" |
            "swift" | "php" | "rb" | "lua" | "blp" | "ui" | "desktop" | "service" => {
                state.files.text = true;
                imp.file_type_text_button.add_css_class("suggested-action");
            }
            "mp4" | "mkv" | "webm" | "mov" | "avi" | "m4v" | "flv" | "wmv" | "mpeg" | "mpg" => {
                state.files.videos = true;
                imp.file_type_videos_button.add_css_class("suggested-action");
            }
            other => {
                state.files.other_ext = Some(other.to_string());
                imp.file_type_other_button.add_css_class("suggested-action");
            }
        }
    }

    // ── Internal setup ─────────────────────────────────────────────────────

    fn setup_callbacks(&self) {
        let imp = self.imp();

        // ── Date preset chips ──────────────────────────────────────────────
        self.connect_date_chip(&imp.today_button, || FilterDateRange::today());
        self.connect_date_chip(&imp.yesterday_button, || FilterDateRange::yesterday());
        self.connect_date_chip(&imp.last_week_button, || FilterDateRange::last_n_days(7));
        self.connect_date_chip(&imp.last_month_button, || FilterDateRange::last_n_days(30));
        self.connect_date_chip(&imp.last_year_button, || FilterDateRange::last_n_days(365));

        // ── Custom range button ────────────────────────────────────────────
        {
            let popover = self.clone();
            imp.custom_range_button.connect_clicked(move |_| {
                popover.open_date_range_dialog();
            });
        }

        // ── Clear date ────────────────────────────────────────────────────
        {
            let popover = self.clone();
            imp.clear_date_button.connect_clicked(move |_| {
                popover.clear_date_filter();
            });
        }

        // ── Author entry ──────────────────────────────────────────────────
        {
            let popover = self.clone();
            imp.author_entry.connect_search_changed(move |entry| {
                // Filter only the already-created author rows.
                // This avoids rebuilding/reloading the author list every time
                // the user types or clears the search entry.
                popover.apply_author_row_search(&entry.text());
            });
        }

        // ── Clear author ──────────────────────────────────────────────────
        {
            let popover = self.clone();
            imp.clear_author_button.connect_clicked(move |_| {
                let imp = popover.imp();
                imp.author_entry.set_text("");
                imp.filter_state.borrow_mut().author = None;
                imp.clear_author_button.set_visible(false);

                let chips_box = imp.author_chips_box.get();
                let mut child = chips_box.first_child();
                while let Some(widget) = child {
                    let next = widget.next_sibling();

                    if let Some(button) = widget.downcast_ref::<gtk::Button>() {
                        button.set_visible(true);
                        button.remove_css_class("suggested-action");
                    }

                    child = next;
                }

                popover.emit_filters_changed();
            });
        }

        // ── Clear branch ──────────────────────────────────────────────────
        {
            let popover = self.clone();
            imp.clear_branch_button.connect_clicked(move |_| {
                let imp = popover.imp();
                imp.filter_state.borrow_mut().branch = None;
                imp.clear_branch_button.set_visible(false);
                // Deselect all branch chips
                let mut child = imp.branch_chips_box.first_child();
                while let Some(c) = child {
                    let next = c.next_sibling();
                    c.remove_css_class("suggested-action");
                    child = next;
                }
                popover.emit_filters_changed();
            });
        }

        // ── File type chips ───────────────────────────────────────────────
        self.connect_file_type_chip(&imp.file_type_audio_button, |f| &mut f.audio);
        self.connect_file_type_chip(&imp.file_type_documents_button, |f| &mut f.documents);
        self.connect_file_type_chip(&imp.file_type_folders_button, |f| &mut f.folders);
        self.connect_file_type_chip(&imp.file_type_images_button, |f| &mut f.images);
        self.connect_file_type_chip(&imp.file_type_pdf_button, |f| &mut f.pdf);
        self.connect_file_type_chip(&imp.file_type_text_button, |f| &mut f.text);
        self.connect_file_type_chip(&imp.file_type_videos_button, |f| &mut f.videos);

        {
            let popover = self.clone();
            imp.file_type_other_button.connect_clicked(move |_| {
                popover.open_file_type_dialog();
            });
        }

        // ── Reset all ─────────────────────────────────────────────────────
        {
            let popover = self.clone();
            imp.reset_all_button.connect_clicked(move |_| {
                popover.reset_all();
            });
        }
    }

    fn connect_date_chip<F>(&self, btn: &gtk::Button, make_range: F)
    where
        F: Fn() -> FilterDateRange + 'static,
    {
        let popover = self.clone();
        let btn_clone = btn.clone();
        btn.connect_clicked(move |_| {
            let imp = popover.imp();
            let new_range = make_range();
            let already_active = imp.filter_state.borrow().date == new_range;

            if already_active {
                popover.clear_date_filter();
            } else {
                // Deselect all date preset chips
                for b in [
                    imp.today_button.get(),
                    imp.yesterday_button.get(),
                    imp.last_week_button.get(),
                    imp.last_month_button.get(),
                    imp.last_year_button.get(),
                ] {
                    b.remove_css_class("suggested-action");
                }
                imp.custom_range_chip.set_visible(false);

                imp.filter_state.borrow_mut().date = new_range;
                btn_clone.add_css_class("suggested-action");
                imp.clear_date_button.set_visible(true);
                popover.emit_filters_changed();
            }
        });
    }

    fn connect_file_type_chip<F>(&self, btn: &gtk::Button, field: F)
    where
        F: Fn(&mut FileTypeFilter) -> &mut bool + 'static,
    {
        let popover = self.clone();
        btn.connect_clicked(move |b| {
            let imp = popover.imp();
            let mut state = imp.filter_state.borrow_mut();
            let flag = field(&mut state.files);
            *flag = !*flag;
            if *flag {
                b.add_css_class("suggested-action");
            } else {
                b.remove_css_class("suggested-action");
            }
            drop(state);
            popover.emit_filters_changed();
        });
    }

    fn open_file_type_dialog(&self) {
        let dialog = FilterTypesDialog::new();

        let popover = self.clone();
        dialog.connect_file_type_selected(move |_dialog, ext| {
            popover.set_file_ext_filter(ext);
            popover.emit_filters_changed();
        });

        if let Some(root) = self.root() {
            dialog.present(Some(&root));
        } else {
            dialog.present(gtk::Window::NONE);
        }
    }

    fn open_date_range_dialog(&self) {
        let imp = self.imp();

        // Reuse existing dialog or create a new one
        let dialog = {
            let existing = imp.date_range_dialog.borrow();
            if let Some(ref d) = *existing {
                d.clone()
            } else {
                drop(existing);
                let d = DateRangeDialog::new();

                // Connect signal once
                let popover = self.clone();
                d.connect_date_range_selected(move |from, to| {
                    let imp = popover.imp();
                    let range = FilterDateRange::from_unix_bounds(from, to);

                    // Update the custom chip label
                    let label = range.chip_label();
                    imp.custom_range_chip.set_label(&label);
                    imp.custom_range_chip.set_visible(true);

                    // Deselect all preset date chips
                    for b in [
                        imp.today_button.get(),
                        imp.yesterday_button.get(),
                        imp.last_week_button.get(),
                        imp.last_month_button.get(),
                        imp.last_year_button.get(),
                    ] {
                        b.remove_css_class("suggested-action");
                    }

                    imp.filter_state.borrow_mut().date = range;
                    imp.clear_date_button.set_visible(true);
                    popover.emit_filters_changed();
                });

                *imp.date_range_dialog.borrow_mut() = Some(d.clone());
                d
            }
        };

        // Pre-fill with existing range if set
        let state = imp.filter_state.borrow();
        if state.date.is_active() {
            dialog.prefill(state.date.from, state.date.to);
        }
        drop(state);

        // Present anchored to the popover's root window
        if let Some(root) = self.root() {
            dialog.present(Some(&root));
        } else {
            dialog.present(gtk::Window::NONE);
        }
    }

    fn clear_date_filter(&self) {
        let imp = self.imp();
        imp.filter_state.borrow_mut().date = FilterDateRange::default();
        imp.clear_date_button.set_visible(false);
        imp.custom_range_chip.set_visible(false);
        for btn in [
            imp.today_button.get(),
            imp.yesterday_button.get(),
            imp.last_week_button.get(),
            imp.last_month_button.get(),
            imp.last_year_button.get(),
        ] {
            btn.remove_css_class("suggested-action");
        }
        self.emit_filters_changed();
    }

    fn reset_all(&self) {
        *self.imp().filter_state.borrow_mut() = FilterState::default();
        let imp = self.imp();

        // Clear date UI
        imp.clear_date_button.set_visible(false);
        imp.custom_range_chip.set_visible(false);
        for btn in [
            imp.today_button.get(),
            imp.yesterday_button.get(),
            imp.last_week_button.get(),
            imp.last_month_button.get(),
            imp.last_year_button.get(),
        ] {
            btn.remove_css_class("suggested-action");
        }

        // Clear author UI
        imp.author_entry.set_text("");
        imp.clear_author_button.set_visible(false);
        let mut child = imp.author_chips_box.first_child();
        while let Some(c) = child {
            let next = c.next_sibling();
            c.remove_css_class("suggested-action");
            child = next;
        }

        // Clear branch UI
        imp.clear_branch_button.set_visible(false);
        let mut child = imp.branch_chips_box.first_child();
        while let Some(c) = child {
            let next = c.next_sibling();
            c.remove_css_class("suggested-action");
            child = next;
        }

        // Clear file type UI
        for btn in [
            imp.file_type_audio_button.get(),
            imp.file_type_documents_button.get(),
            imp.file_type_folders_button.get(),
            imp.file_type_images_button.get(),
            imp.file_type_pdf_button.get(),
            imp.file_type_text_button.get(),
            imp.file_type_videos_button.get(),
            imp.file_type_other_button.get(),
        ] {
            btn.remove_css_class("suggested-action");
        }

        self.emit_filters_changed();
    }

    /// Connect to the `filters-changed` signal.
    pub fn connect_filters_changed<F>(&self, f: F) -> glib::SignalHandlerId
    where
        F: Fn(&Self) + 'static,
    {
        self.connect_local("filters-changed", false, move |values| {
            let popover = values[0].get::<SearchFilterPopover>().unwrap();
            f(&popover);
            None
        })
    }

    fn emit_filters_changed(&self) {
        self.emit_by_name::<()>("filters-changed", &[]);
    }
}
