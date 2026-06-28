/* select_commits_by_pattern.rs
 *
 * Copyright 2026 John Peter Sá
 * SPDX-License-Identifier: GPL-3.0-or-later
 *
 * Dialog: select commits whose changed file paths match a glob.
 *
 * Usage
 * -----
 *   let dlg = SelectCommitsByPattern::new();
 *   dlg.set_commits(&all_commits);   // needed for live match-count preview
 *   dlg.connect_pattern_selected(|pattern, mode, icase| { … });
 *   dlg.present(Some(&window));
 *
 * The dialog emits `pattern-selected` when the user clicks "Select Commits".
 * The caller is responsible for iterating the commit list and marking the
 * matching rows as selected inside the GtkListView / GtkSelectionModel.
 *
 * MatchMode
 * ---------
 *   Any  — commit is included if ANY changed file matches the pattern
 *   All  — commit is included only if ALL changed files match
 */

use adw::prelude::*;
use adw::subclass::prelude::*;
use gtk::glib;
use std::cell::RefCell;
use std::sync::OnceLock;

use gettextrs::gettext;

use crate::git_engine::CommitInfo;

// ── MatchMode ─────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MatchMode {
    #[default]
    Any,
    All,
}

impl MatchMode {
    fn from_index(idx: u32) -> Self {
        if idx == 1 {
            Self::All
        } else {
            Self::Any
        }
    }

    pub fn as_index(self) -> u32 {
        match self {
            Self::Any => 0,
            Self::All => 1,
        }
    }
}

// ── Glob matching helper ──────────────────────────────────────────────────────

/// Minimal glob matcher — supports `*` (any chars except `/`) and
/// `**` (any chars including `/`). No external crate required.
/// For production use, swap with the `globset` crate.
fn glob_matches(pattern: &str, path: &str, icase: bool) -> bool {
    let (p, s) = if icase {
        (pattern.to_lowercase(), path.to_lowercase())
    } else {
        (pattern.to_string(), path.to_string())
    };
    glob_match_inner(&p, &s)
}

fn glob_match_inner(pat: &str, s: &str) -> bool {
    let pb = pat.as_bytes();
    let sb = s.as_bytes();
    glob_rec(pb, sb)
}

fn glob_rec(p: &[u8], s: &[u8]) -> bool {
    match (p.first(), s.first()) {
        (None, None) => true,
        (None, Some(_)) => false,
        (Some(b'*'), _) => {
            // Check for `**`
            if p.get(1) == Some(&b'*') {
                // `**` — try matching 0 or more characters including '/'
                let rest = if p.get(2) == Some(&b'/') {
                    &p[3..]
                } else {
                    &p[2..]
                };
                for i in 0..=s.len() {
                    if glob_rec(rest, &s[i..]) {
                        return true;
                    }
                }
                false
            } else {
                // `*` — match any run of non-'/'
                let rest = &p[1..];
                for i in 0..=s.len() {
                    if s[..i].contains(&b'/') {
                        break;
                    }
                    if glob_rec(rest, &s[i..]) {
                        return true;
                    }
                }
                false
            }
        }
        (Some(&pc), Some(&sc)) => {
            if pc == b'?' || pc == sc {
                glob_rec(&p[1..], &s[1..])
            } else {
                false
            }
        }
        _ => false,
    }
}

// ── GObject subclass ──────────────────────────────────────────────────────────

mod imp {
    use super::*;

    #[derive(Debug, Default, gtk::CompositeTemplate)]
    #[template(resource = "/io/github/johnpetersa19/TemporalExplorer/select-commits-by-pattern.ui")]
    pub struct SelectCommitsByPattern {
        #[template_child]
        pub pattern_entry: TemplateChild<adw::EntryRow>,
        #[template_child]
        pub match_mode_row: TemplateChild<adw::ComboRow>,
        #[template_child]
        pub case_insensitive_row: TemplateChild<adw::SwitchRow>,
        #[template_child]
        pub match_count_label: TemplateChild<gtk::Label>,
        #[template_child]
        pub select_button: TemplateChild<gtk::Button>,
        #[template_child]
        pub cancel_button: TemplateChild<gtk::Button>,

        /// Full commit list for live preview; set via `set_commits`.
        pub commits: RefCell<Vec<CommitInfo>>,
        /// Last match count (-1 = not yet evaluated).
        pub match_count: RefCell<i32>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for SelectCommitsByPattern {
        const NAME: &'static str = "SelectCommitsByPattern";
        type Type = super::SelectCommitsByPattern;
        type ParentType = adw::Dialog;

        fn class_init(klass: &mut Self::Class) {
            klass.bind_template();
            klass.bind_template_callbacks();
        }

        fn instance_init(obj: &glib::subclass::InitializingObject<Self>) {
            obj.init_template();
        }
    }

    impl ObjectImpl for SelectCommitsByPattern {
        fn constructed(&self) {
            self.parent_constructed();
            self.obj().setup_callbacks();
        }

        fn signals() -> &'static [glib::subclass::Signal] {
            static SIGNALS: OnceLock<Vec<glib::subclass::Signal>> = OnceLock::new();
            SIGNALS.get_or_init(|| {
                vec![glib::subclass::Signal::builder("pattern-selected")
                    .param_types([
                        String::static_type(), // pattern
                        u32::static_type(),    // MatchMode index
                        bool::static_type(),   // case_insensitive
                    ])
                    .build()]
            })
        }
    }

    impl WidgetImpl for SelectCommitsByPattern {}
    impl AdwDialogImpl for SelectCommitsByPattern {}

    #[gtk::template_callbacks]
    impl SelectCommitsByPattern {
        #[template_callback]
        fn on_pattern_apply(&self) {
            self.obj().run_preview();
        }

        #[template_callback]
        fn on_select_clicked(&self) {
            self.obj().emit_pattern_selected();
        }

        #[template_callback]
        fn on_cancel_clicked(&self) {
            self.obj().close();
        }
    }
}

// ── Public wrapper ────────────────────────────────────────────────────────────

glib::wrapper! {
    pub struct SelectCommitsByPattern(ObjectSubclass<imp::SelectCommitsByPattern>)
        @extends adw::Dialog, gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl Default for SelectCommitsByPattern {
    fn default() -> Self {
        Self::new()
    }
}

impl SelectCommitsByPattern {
    pub fn new() -> Self {
        glib::Object::new()
    }

    // ── Public API ─────────────────────────────────────────────────────────

    /// Provide the full commit list for live match-count preview.
    pub fn set_commits(&self, commits: &[CommitInfo]) {
        *self.imp().commits.borrow_mut() = commits.to_vec();
        self.run_preview();
    }

    /// Connect to `pattern-selected` signal.
    pub fn connect_pattern_selected<F>(&self, f: F) -> glib::SignalHandlerId
    where
        F: Fn(&Self, &str, MatchMode, bool) + 'static,
    {
        self.connect_local("pattern-selected", false, move |v| {
            let dlg = v[0].get::<SelectCommitsByPattern>().unwrap();
            let pattern = v[1].get::<String>().unwrap();
            let mode = MatchMode::from_index(v[2].get::<u32>().unwrap());
            let icase = v[3].get::<bool>().unwrap();
            f(&dlg, &pattern, mode, icase);
            None
        })
    }

    // ── Internal ───────────────────────────────────────────────────────────

    fn setup_callbacks(&self) {
        let imp = self.imp();

        // Re-preview when match mode or case toggle changes
        let dlg = self.clone();
        imp.match_mode_row.connect_selected_notify(move |_| {
            dlg.run_preview();
        });

        let dlg = self.clone();
        imp.case_insensitive_row.connect_active_notify(move |_| {
            dlg.run_preview();
        });

        // Also preview on each keystroke in the entry
        let dlg = self.clone();
        imp.pattern_entry.connect_changed(move |_| {
            dlg.run_preview();
        });
    }

    /// Run the glob against `self.commits` and update the UI feedback.
    fn run_preview(&self) {
        let imp = self.imp();
        let pattern = imp.pattern_entry.text().to_string();
        let pattern = pattern.trim().to_string();

        if pattern.is_empty() {
            imp.match_count_label
                .set_label(&gettext("Enter a pattern to preview matches"));
            imp.select_button.set_sensitive(false);
            *imp.match_count.borrow_mut() = -1;
            return;
        }

        let icase = imp.case_insensitive_row.is_active();
        let mode = MatchMode::from_index(imp.match_mode_row.selected());
        let commits = imp.commits.borrow();

        let count = count_matching_commits(&commits, &pattern, mode, icase);

        *imp.match_count.borrow_mut() = count as i32;

        let total = commits.len();
        let label = if count == 0 {
            gettext("No commits match")
        } else if count == total {
            gettext("All {count} commits match").replace("{count}", &total.to_string())
        } else {
            gettext("{count} of {total} commits match")
                .replace("{count}", &count.to_string())
                .replace("{total}", &total.to_string())
        };
        imp.match_count_label.set_label(&label);
        imp.select_button.set_sensitive(count > 0);
    }

    fn emit_pattern_selected(&self) {
        let imp = self.imp();
        let pattern = imp.pattern_entry.text().to_string();
        let mode = imp.match_mode_row.selected();
        let icase = imp.case_insensitive_row.is_active();
        self.emit_by_name::<()>(
            "pattern-selected",
            &[&pattern.to_value(), &mode.to_value(), &icase.to_value()],
        );
        self.close();
    }
}

// ── Free function used by window.rs for post-selection filtering ──────────────

/// Returns `true` when `commit` satisfies the glob pattern under `mode`.
pub fn commit_matches_pattern(
    commit: &CommitInfo,
    pattern: &str,
    mode: MatchMode,
    icase: bool,
) -> bool {
    if commit.changed_files.is_empty() {
        return false;
    }
    match mode {
        MatchMode::Any => commit
            .changed_files
            .iter()
            .any(|f| glob_matches(pattern, f, icase)),
        MatchMode::All => commit
            .changed_files
            .iter()
            .all(|f| glob_matches(pattern, f, icase)),
    }
}

fn count_matching_commits(
    commits: &[CommitInfo],
    pattern: &str,
    mode: MatchMode,
    icase: bool,
) -> usize {
    commits
        .iter()
        .filter(|commit| commit_matches_pattern(commit, pattern, mode, icase))
        .count()
}
