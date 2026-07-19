/* batch_operations_dialog.rs
 *
 * Copyright 2026 John Peter Sá
 * SPDX-License-Identifier: GPL-3.0-or-later
 *
 * Batch operations dialog for power users.
 *
 * Usage
 * -----
 *   let dlg = BatchOperationsDialog::new();
 *   dlg.set_commits(&selected_commits);   // Vec<CommitInfo>
 *   dlg.connect_operation_requested(|op, shas| { … });
 *   dlg.present(Some(&window));
 *
 * The dialog emits `operation-requested` with a `BatchOp` discriminant
 * and the list of full SHAs when the user clicks Run.  Actual execution
 * (git2 calls) is the caller's responsibility so the dialog stays thin.
 *
 * BatchOp variants
 * ----------------
 *   CherryPick { signoff: bool }
 *   ExportPatches { dest_dir: PathBuf }
 *   CopyShas { short: bool }
 */

use adw::prelude::*;
use adw::subclass::prelude::*;
use gtk::glib;
use std::cell::RefCell;
use std::path::PathBuf;
use std::sync::OnceLock;

use gettextrs::gettext;

use crate::git_engine::CommitInfo;

// ── BatchOp ────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub enum BatchOp {
    CherryPick { signoff: bool },
    ExportPatches { dest_dir: PathBuf },
    CopyShas { short: bool },
}

impl BatchOp {
    /// Returns the ComboRow index that corresponds to this operation.
    /// Used for round-tripping the op discriminant through the GObject signal.
    #[allow(dead_code)]
    fn index(&self) -> u32 {
        match self {
            Self::CherryPick { .. } => 0,
            Self::ExportPatches { .. } => 1,
            Self::CopyShas { .. } => 2,
        }
    }
}

// ── GObject subclass ───────────────────────────────────────────────────────────────

mod imp {
    use super::*;

    #[derive(Debug, Default, gtk::CompositeTemplate)]
    #[template(resource = "/io/github/TemporalExplorer/batch-operations-dialog.ui")]
    pub struct BatchOperationsDialog {
        #[template_child]
        pub commit_count_label: TemplateChild<gtk::Label>,
        #[template_child]
        pub commit_list: TemplateChild<gtk::ListBox>,
        #[template_child]
        pub operation_row: TemplateChild<adw::ComboRow>,
        #[template_child]
        pub export_path_row: TemplateChild<adw::ActionRow>,
        #[template_child]
        pub export_path_button: TemplateChild<gtk::Button>,
        #[template_child]
        pub export_path_label: TemplateChild<gtk::Label>,
        #[template_child]
        pub signoff_row: TemplateChild<adw::SwitchRow>,
        #[template_child]
        pub short_sha_row: TemplateChild<adw::SwitchRow>,
        #[template_child]
        pub preview_group: TemplateChild<adw::PreferencesGroup>,
        #[template_child]
        pub preview_text: TemplateChild<gtk::TextView>,
        #[template_child]
        pub operation_progress: TemplateChild<gtk::ProgressBar>,
        #[template_child]
        pub run_button: TemplateChild<gtk::Button>,
        #[template_child]
        pub cancel_button: TemplateChild<gtk::Button>,

        /// Full list of commits to operate on.
        pub commits: RefCell<Vec<CommitInfo>>,
        /// Chosen export directory for ExportPatches.
        pub dest_dir: RefCell<Option<PathBuf>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for BatchOperationsDialog {
        const NAME: &'static str = "BatchOperationsDialog";
        type Type = super::BatchOperationsDialog;
        type ParentType = adw::Dialog;

        fn class_init(klass: &mut Self::Class) {
            klass.bind_template();
            klass.bind_template_callbacks();
        }

        fn instance_init(obj: &glib::subclass::InitializingObject<Self>) {
            obj.init_template();
        }
    }

    impl ObjectImpl for BatchOperationsDialog {
        fn constructed(&self) {
            self.parent_constructed();
            self.obj().setup_callbacks();
        }

        fn signals() -> &'static [glib::subclass::Signal] {
            static SIGNALS: OnceLock<Vec<glib::subclass::Signal>> = OnceLock::new();
            SIGNALS.get_or_init(|| {
                vec![
                    // Carries (op_index: u32, shas: Vec<String>) — caller decodes
                    // op_index back into BatchOp using current dialog state.
                    glib::subclass::Signal::builder("operation-requested")
                        .param_types([
                            u32::static_type(),
                            // Vec<String> is not a GType; we serialise SHAs as
                            // newline-joined string and split on the other side.
                            String::static_type(),
                        ])
                        .build(),
                ]
            })
        }
    }

    impl WidgetImpl for BatchOperationsDialog {}
    impl AdwDialogImpl for BatchOperationsDialog {}

    #[gtk::template_callbacks]
    impl BatchOperationsDialog {
        #[template_callback]
        fn on_run_clicked(&self) {
            self.obj().emit_operation_requested();
        }

        #[template_callback]
        fn on_cancel_clicked(&self) {
            self.obj().close();
        }
    }
}

// ── Public wrapper ─────────────────────────────────────────────────────────────────────────

glib::wrapper! {
    pub struct BatchOperationsDialog(ObjectSubclass<imp::BatchOperationsDialog>)
        @extends adw::Dialog, gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl Default for BatchOperationsDialog {
    fn default() -> Self {
        Self::new()
    }
}

impl BatchOperationsDialog {
    pub fn new() -> Self {
        glib::Object::new()
    }

    // ── Public API ────────────────────────────────────────────────────────────────────

    /// Populate the commit list; must be called before `present`.
    pub fn set_commits(&self, commits: &[CommitInfo]) {
        let imp = self.imp();
        *imp.commits.borrow_mut() = commits.to_vec();

        // Update count chip
        imp.commit_count_label
            .set_label(&format!("{}", commits.len()));

        // Rebuild list rows
        while let Some(child) = imp.commit_list.first_child() {
            imp.commit_list.remove(&child);
        }
        for c in commits {
            let row = adw::ActionRow::builder()
                .title(&c.summary)
                .subtitle(&c.hash[..7.min(c.hash.len())])
                .build();
            imp.commit_list.append(&row);
        }

        self.on_operation_changed();
        self.refresh_preview();
    }

    /// Connect to `operation-requested` signal.
    /// The callback receives the `BatchOp` and the list of full SHAs.
    pub fn connect_operation_requested<F>(&self, f: F) -> glib::SignalHandlerId
    where
        F: Fn(&Self, BatchOp, Vec<String>) + 'static,
    {
        self.connect_local("operation-requested", false, move |v| {
            let dlg = v[0].get::<BatchOperationsDialog>().unwrap();
            let idx = v[1].get::<u32>().unwrap();
            let shas_str = v[2].get::<String>().unwrap();
            let shas: Vec<String> = shas_str
                .lines()
                .filter(|s| !s.is_empty())
                .map(String::from)
                .collect();

            // Reconstruct BatchOp from index + current dialog state
            let op = dlg.current_batch_op(idx);
            f(&dlg, op, shas);
            None
        })
    }

    /// Show or hide the progress bar and pulse it.
    pub fn set_progress_visible(&self, visible: bool) {
        self.imp().operation_progress.set_visible(visible);
        if visible {
            self.imp().operation_progress.pulse();
        }
    }

    /// Called by the caller after a batch operation finishes.
    pub fn mark_done(&self) {
        let imp = self.imp();
        imp.operation_progress.set_fraction(1.0);
        imp.run_button.set_sensitive(false);
    }

    /// Restore the dialog after an operation failed so the user can adjust
    /// the options and try again.
    pub fn mark_failed(&self) {
        self.imp().operation_progress.set_visible(false);
        self.update_run_button();
    }

    // ── Internal ──────────────────────────────────────────────────────────────────────

    fn setup_callbacks(&self) {
        let imp = self.imp();

        // Show/hide per-operation rows when operation_row changes
        let dlg = self.clone();
        imp.operation_row.connect_selected_notify(move |_| {
            dlg.on_operation_changed();
        });

        // Export path chooser
        let dlg = self.clone();
        imp.export_path_button.connect_clicked(move |_| {
            dlg.open_export_dir_chooser();
        });

        // Preview refresh on signoff / short_sha toggle
        let dlg = self.clone();
        imp.signoff_row.connect_active_notify(move |_| {
            dlg.refresh_preview();
        });
        let dlg = self.clone();
        imp.short_sha_row.connect_active_notify(move |_| {
            dlg.refresh_preview();
        });
    }

    fn on_operation_changed(&self) {
        let idx = self.imp().operation_row.selected();
        let imp = self.imp();
        imp.signoff_row.set_visible(idx == 0);
        imp.export_path_row.set_visible(idx == 1);
        imp.short_sha_row.set_visible(idx == 2);
        self.update_run_button();
        self.refresh_preview();
    }

    fn update_run_button(&self) {
        let imp = self.imp();
        let has_commits = !imp.commits.borrow().is_empty();
        let has_destination = imp.operation_row.selected() != 1 || imp.dest_dir.borrow().is_some();
        imp.run_button.set_sensitive(has_commits && has_destination);
    }

    fn refresh_preview(&self) {
        let imp = self.imp();
        let idx = imp.operation_row.selected();
        let commits = imp.commits.borrow();
        let buf = imp.preview_text.buffer();

        let text = match idx {
            0 => {
                // Cherry-pick: list git cherry-pick commands
                commits
                    .iter()
                    .map(|c| format!("git cherry-pick {}", &c.hash[..7.min(c.hash.len())]))
                    .collect::<Vec<_>>()
                    .join("\n")
            }
            1 => {
                // Export patches: list output file names
                let dir = imp.dest_dir.borrow();
                let prefix = dir
                    .as_ref()
                    .map(|d| d.display().to_string())
                    .unwrap_or_else(|| gettext("Choose a directory"));
                commits
                    .iter()
                    .enumerate()
                    .map(|(i, c)| {
                        format!(
                            "{}/{:04}-{}.patch",
                            prefix,
                            i + 1,
                            c.summary
                                .chars()
                                .take(40)
                                .collect::<String>()
                                .replace(|ch: char| !ch.is_alphanumeric(), "-")
                        )
                    })
                    .collect::<Vec<_>>()
                    .join("\n")
            }
            _ => {
                // Copy SHAs
                let short = imp.short_sha_row.is_active();
                commits
                    .iter()
                    .map(|c| {
                        if short {
                            c.hash[..7.min(c.hash.len())].to_string()
                        } else {
                            c.hash.clone()
                        }
                    })
                    .collect::<Vec<_>>()
                    .join("\n")
            }
        };

        buf.set_text(&text);
    }

    fn open_export_dir_chooser(&self) {
        // Use GtkFileDialog (GTK 4.10+)
        let chooser = gtk::FileDialog::builder()
            .title(gettext("Choose export directory"))
            .accept_label(gettext("Select"))
            .build();

        let dlg = self.clone();
        if let Some(root) = self.root().and_downcast::<gtk::Window>() {
            chooser.select_folder(Some(&root), gtk::gio::Cancellable::NONE, move |res| {
                if let Ok(file) = res {
                    if let Some(path) = file.path() {
                        let imp = dlg.imp();
                        imp.export_path_label.set_label(&path.display().to_string());
                        *imp.dest_dir.borrow_mut() = Some(path);
                        dlg.update_run_button();
                        dlg.refresh_preview();
                    }
                }
            });
        }
    }

    fn current_batch_op(&self, idx: u32) -> BatchOp {
        let imp = self.imp();
        match idx {
            0 => BatchOp::CherryPick {
                signoff: imp.signoff_row.is_active(),
            },
            1 => BatchOp::ExportPatches {
                dest_dir: imp
                    .dest_dir
                    .borrow()
                    .clone()
                    .unwrap_or_else(|| PathBuf::from(".")),
            },
            _ => BatchOp::CopyShas {
                short: imp.short_sha_row.is_active(),
            },
        }
    }

    fn emit_operation_requested(&self) {
        let imp = self.imp();
        let idx = imp.operation_row.selected();
        let commits = imp.commits.borrow();
        let shas = commits
            .iter()
            .map(|c| c.hash.clone())
            .collect::<Vec<_>>()
            .join("\n");
        drop(commits);
        self.emit_by_name::<()>("operation-requested", &[&idx.to_value(), &shas.to_value()]);
    }
}
