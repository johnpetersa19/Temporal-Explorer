/* merge_conflict_dialog.rs
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

//! `MergeConflictDialog` — resolution UI for merge-commit conflicts.
//!
//! Shows the conflicted file path, metadata for both the HEAD (ours) and
//! incoming (theirs) commits, an optional diff preview, and three action
//! buttons: **Use Ours**, **Use Theirs**, **Keep Both**.
//!
//! Emits a `conflict-resolved` signal with a [`ConflictResolution`] payload
//! so `window.rs` / `commit_controller.rs` can act on the user's choice.
//!
//! # Usage
//! ```rust
//! let dialog = MergeConflictDialog::new();
//! dialog.load_conflict(&conflict_info);
//! dialog.connect_conflict_resolved(|res| { /* handle */ });
//! dialog.present(Some(&window));
//! ```

use adw::prelude::*;
use adw::subclass::prelude::*;
use gtk::glib;
use std::cell::RefCell;

/// All information needed to populate the dialog.
#[derive(Debug, Clone, Default)]
pub struct ConflictInfo {
    /// Repo-relative path of the conflicted file.
    pub file_path: String,

    // Ours (HEAD)
    pub ours_sha: String,
    pub ours_author: String,
    pub ours_date: String,

    // Theirs (incoming)
    pub theirs_sha: String,
    pub theirs_author: String,
    pub theirs_date: String,

    /// Raw unified diff string (optional — empty hides the expander).
    pub diff_text: String,
}

// ── GObject subclass ───────────────────────────────────────────────────────────

mod imp {
    use super::*;

    #[derive(Debug, Default, gtk::CompositeTemplate)]
    #[template(resource = "/io/github/johnpetersa19/TemporalExplorer/merge-conflict-dialog.ui")]
    pub struct MergeConflictDialog {
        // Header
        #[template_child]
        pub conflict_banner: TemplateChild<adw::Banner>,

        // File info
        #[template_child]
        pub file_path_row: TemplateChild<adw::ActionRow>,

        // Ours
        #[template_child]
        pub ours_commit_row: TemplateChild<adw::ActionRow>,
        #[template_child]
        pub ours_author_row: TemplateChild<adw::ActionRow>,
        #[template_child]
        pub ours_date_row: TemplateChild<adw::ActionRow>,

        // Theirs
        #[template_child]
        pub theirs_commit_row: TemplateChild<adw::ActionRow>,
        #[template_child]
        pub theirs_author_row: TemplateChild<adw::ActionRow>,
        #[template_child]
        pub theirs_date_row: TemplateChild<adw::ActionRow>,

        // Diff
        #[template_child]
        pub diff_expander: TemplateChild<gtk::Expander>,
        #[template_child]
        pub diff_view: TemplateChild<gtk::TextView>,

        // State
        pub conflict_info: RefCell<ConflictInfo>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for MergeConflictDialog {
        const NAME: &'static str = "MergeConflictDialog";
        type Type = super::MergeConflictDialog;
        type ParentType = adw::Dialog;

        fn class_init(klass: &mut Self::Class) {
            klass.bind_template();
            klass.bind_template_callbacks();
            klass.install_action("dialog.show-diff", None, |d, _, _| {
                let exp = d.imp().diff_expander.get();
                exp.set_expanded(!exp.is_expanded());
            });
            klass.install_action("dialog.close", None, |d, _, _| {
                d.close();
            });
        }

        fn instance_init(obj: &glib::subclass::InitializingObject<Self>) {
            obj.init_template();
        }
    }

    impl ObjectImpl for MergeConflictDialog {
        fn constructed(&self) {
            self.parent_constructed();
        }
    }

    impl WidgetImpl for MergeConflictDialog {}
    impl AdwDialogImpl for MergeConflictDialog {}

    #[gtk::template_callbacks]
    impl MergeConflictDialog {
        #[template_callback]
        fn on_cancel_clicked(&self) {
            self.obj().close();
        }
    }
}

// ── Public wrapper ─────────────────────────────────────────────────────────────

glib::wrapper! {
    pub struct MergeConflictDialog(ObjectSubclass<imp::MergeConflictDialog>)
        @extends adw::Dialog, gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl Default for MergeConflictDialog {
    fn default() -> Self {
        Self::new()
    }
}

impl MergeConflictDialog {
    pub fn new() -> Self {
        glib::Object::new()
    }

    // ── Public API ─────────────────────────────────────────────────────────

    /// Populate the dialog with conflict metadata.
    /// Call this before `present()`.
    pub fn load_conflict(&self, info: &ConflictInfo) {
        let imp = self.imp();

        // File path
        imp.file_path_row.set_subtitle(&info.file_path);
        imp.file_path_row.set_title("Path");

        // Ours
        let short_ours = if info.ours_sha.len() >= 8 {
            &info.ours_sha[..8]
        } else {
            &info.ours_sha
        };
        imp.ours_commit_row.set_subtitle(short_ours);
        imp.ours_author_row.set_subtitle(&info.ours_author);
        imp.ours_date_row.set_subtitle(&info.ours_date);

        // Theirs
        let short_theirs = if info.theirs_sha.len() >= 8 {
            &info.theirs_sha[..8]
        } else {
            &info.theirs_sha
        };
        imp.theirs_commit_row.set_subtitle(short_theirs);
        imp.theirs_author_row.set_subtitle(&info.theirs_author);
        imp.theirs_date_row.set_subtitle(&info.theirs_date);

        // Diff
        if info.diff_text.is_empty() {
            imp.diff_expander.set_visible(false);
            imp.conflict_banner.set_button_label(Some(""));
        } else {
            imp.diff_expander.set_visible(true);
            let buf = imp.diff_view.buffer();
            buf.set_text(&info.diff_text);
            // Basic syntax colouring via tags (red for removals, green for additions)
            if let Some(tag_add) = buf.create_tag(Some("add"), &[]) {
                tag_add.set_foreground(Some("#26a269"));
            }
            if let Some(tag_del) = buf.create_tag(Some("del"), &[]) {
                tag_del.set_foreground(Some("#c01c28"));
            }

            let text = buf.text(&buf.start_iter(), &buf.end_iter(), false);
            let mut offset = 0i32;
            for line in text.split_inclusive('\n') {
                let line_len = line.chars().count() as i32;
                let start = buf.iter_at_offset(offset);
                let end = buf.iter_at_offset(offset + line_len);
                if line.starts_with('+') && !line.starts_with("+++") {
                    buf.apply_tag_by_name("add", &start, &end);
                } else if line.starts_with('-') && !line.starts_with("---") {
                    buf.apply_tag_by_name("del", &start, &end);
                }
                offset += line_len;
            }
        }

        *imp.conflict_info.borrow_mut() = info.clone();
    }
}
