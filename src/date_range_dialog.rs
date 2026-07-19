/* date_range_dialog.rs
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

//! `DateRangeDialog` — custom date-range picker for the filter popover.
//!
//! Port / inspiration: `nautilus-date-range-dialog` (GNOME Nautilus 47+),
//! adapted from a file-date filter into a Git commit-date range filter.
//!
//! # Behaviour
//!
//! * Both `from_entry` and `to_entry` accept free-text `YYYY-MM-DD` input.
//! * The **Apply Filter** button stays insensitive until at least one valid
//!   date is entered *and* the range is coherent (from ≤ to when both set).
//! * On confirmation the dialog emits the `date-range-selected` signal
//!   carrying `(from_unix: i64, to_unix: i64)` — both values are the
//!   midnight Unix timestamp of the respective day (local timezone).
//!   When only one bound is supplied the other is `i64::MIN` / `i64::MAX`.
//! * Parsing errors show an inline `error_group` banner; the apply button
//!   stays disabled.
//!
//! # Usage
//!
//! ```rust
//! let dialog = DateRangeDialog::new();
//! dialog.connect_date_range_selected(|from, to| {
//!     // from / to are i64 Unix timestamps
//! });
//! dialog.present(Some(&parent_widget));
//! ```

use adw::prelude::*;
use adw::subclass::prelude::*;
use gtk::glib;
use gtk::glib::subclass::Signal;
use std::cell::Cell;
use std::sync::OnceLock;

// ── parse_date_entry ───────────────────────────────────────────────────────────

/// Parse `YYYY-MM-DD` text into a midnight Unix timestamp (local tz).
/// Returns `None` on any error.
fn parse_date_entry(text: &str) -> Option<i64> {
    let text = text.trim();
    if text.is_empty() {
        return None;
    }

    // Accept both "YYYY-MM-DD" and "YYYY/MM/DD"
    let normalized = text.replace('/', "-");
    let parts: Vec<&str> = normalized.split('-').collect();
    if parts.len() != 3 {
        return None;
    }

    let year: i32 = parts[0].parse().ok()?;
    let month: i32 = parts[1].parse().ok()?;
    let day: i32 = parts[2].parse().ok()?;

    // Basic range validation
    if !(1970..=9999).contains(&year) || !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }

    let dt = glib::DateTime::new(&glib::TimeZone::local(), year, month, day, 0, 0, 0.0).ok()?;

    Some(dt.to_unix())
}

// ── GObject subclass ───────────────────────────────────────────────────────────

mod imp {
    use super::*;

    #[derive(Debug, Default, gtk::CompositeTemplate)]
    #[template(resource = "/io/github/TemporalExplorer/date-range-dialog.ui")]
    pub struct DateRangeDialog {
        #[template_child]
        pub from_entry: TemplateChild<adw::EntryRow>,
        #[template_child]
        pub to_entry: TemplateChild<adw::EntryRow>,
        #[template_child]
        pub error_group: TemplateChild<adw::PreferencesGroup>,
        #[template_child]
        pub apply_button: TemplateChild<gtk::Button>,
        #[template_child]
        pub cancel_button: TemplateChild<gtk::Button>,

        /// Cached parsed timestamps; `None` when the field is empty or invalid.
        pub from_ts: Cell<Option<i64>>,
        pub to_ts: Cell<Option<i64>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for DateRangeDialog {
        const NAME: &'static str = "DateRangeDialog";
        type Type = super::DateRangeDialog;
        type ParentType = adw::Dialog;

        fn class_init(klass: &mut Self::Class) {
            klass.bind_template();
            klass.bind_template_callbacks();
        }

        fn instance_init(obj: &glib::subclass::InitializingObject<Self>) {
            obj.init_template();
        }
    }

    impl ObjectImpl for DateRangeDialog {
        fn constructed(&self) {
            self.parent_constructed();
            self.obj().setup_callbacks();
        }

        fn signals() -> &'static [Signal] {
            static SIGNALS: OnceLock<Vec<Signal>> = OnceLock::new();
            SIGNALS.get_or_init(|| {
                vec![
                    // Emitted when the user confirms; carries (from_unix, to_unix).
                    // i64::MIN means "no lower bound"; i64::MAX means "no upper bound".
                    Signal::builder("date-range-selected")
                        .param_types([i64::static_type(), i64::static_type()])
                        .build(),
                ]
            })
        }
    }

    impl WidgetImpl for DateRangeDialog {}
    impl AdwDialogImpl for DateRangeDialog {}

    // ── Template callbacks ─────────────────────────────────────────────────

    #[gtk::template_callbacks]
    impl DateRangeDialog {}
}

// ── Public wrapper ─────────────────────────────────────────────────────────────

glib::wrapper! {
    pub struct DateRangeDialog(ObjectSubclass<imp::DateRangeDialog>)
        @extends adw::Dialog, gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl Default for DateRangeDialog {
    fn default() -> Self {
        Self::new()
    }
}

impl DateRangeDialog {
    pub fn new() -> Self {
        glib::Object::new()
    }

    // ── Internal wiring ────────────────────────────────────────────────────

    fn setup_callbacks(&self) {
        let imp = self.imp();

        // from_entry changed
        {
            let dialog = self.clone();
            imp.from_entry
                .connect_changed(move |_| dialog.on_entry_changed());
        }

        // to_entry changed
        {
            let dialog = self.clone();
            imp.to_entry
                .connect_changed(move |_| dialog.on_entry_changed());
        }

        // Apply button
        {
            let dialog = self.clone();
            imp.apply_button.connect_clicked(move |_| dialog.on_apply());
        }

        {
            let dialog = self.clone();
            imp.cancel_button.connect_clicked(move |_| {
                dialog.close();
            });
        }
    }

    fn on_entry_changed(&self) {
        let imp = self.imp();

        let from_text = imp.from_entry.text();
        let to_text = imp.to_entry.text();

        let from_ts = parse_date_entry(&from_text);
        let to_ts = parse_date_entry(&to_text);

        // Store parsed values
        imp.from_ts.set(from_ts);
        imp.to_ts.set(to_ts);

        let from_empty = from_text.trim().is_empty();
        let to_empty = to_text.trim().is_empty();

        // Determine error state
        let from_invalid = !from_empty && from_ts.is_none();
        let to_invalid = !to_empty && to_ts.is_none();

        // Range coherence: from must be ≤ to when both are set
        let range_incoherent = matches!((from_ts, to_ts), (Some(f), Some(t)) if f > t);

        let has_error = from_invalid || to_invalid || range_incoherent;

        // Show/hide inline error banner
        imp.error_group.set_visible(has_error);

        // Apply button: needs at least one valid date and no errors
        let has_any_date = from_ts.is_some() || to_ts.is_some();
        imp.apply_button.set_sensitive(has_any_date && !has_error);

        // Visual feedback on the entry rows themselves
        if from_invalid {
            imp.from_entry.add_css_class("error");
        } else {
            imp.from_entry.remove_css_class("error");
        }

        if to_invalid {
            imp.to_entry.add_css_class("error");
        } else {
            imp.to_entry.remove_css_class("error");
        }
    }

    fn on_apply(&self) {
        let imp = self.imp();

        let from = imp.from_ts.get().unwrap_or(i64::MIN);
        let to = imp.to_ts.get().unwrap_or(i64::MAX);

        // Emit signal before closing so listeners can read values
        self.emit_by_name::<()>("date-range-selected", &[&from, &to]);
        self.close();
    }

    // ── Public API ─────────────────────────────────────────────────────────

    /// Pre-populate the entries from an existing `FilterDateRange`.
    /// Useful when re-opening the dialog to edit a previous filter.
    pub fn prefill(&self, from: Option<i64>, to: Option<i64>) {
        let imp = self.imp();

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
                .unwrap_or_default()
        };

        if let Some(f) = from {
            imp.from_entry.set_text(&fmt(f));
        }
        if let Some(t) = to {
            // Subtract 1 second so "to midnight of day D" shows as day D, not D+1
            imp.to_entry.set_text(&fmt(t.saturating_sub(1)));
        }
    }

    /// Connect to the `date-range-selected` signal.
    /// `f` receives `(from_unix: i64, to_unix: i64)`.
    pub fn connect_date_range_selected<F>(&self, f: F) -> glib::SignalHandlerId
    where
        F: Fn(i64, i64) + 'static,
    {
        self.connect_local("date-range-selected", false, move |values| {
            let from = values[1].get::<i64>().unwrap_or(i64::MIN);
            let to = values[2].get::<i64>().unwrap_or(i64::MAX);
            f(from, to);
            None
        })
    }
}
