/* application.rs
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

use adw::prelude::*;
use adw::subclass::prelude::*;
use gettextrs::gettext;
use gtk::{gio, glib};

use crate::config::VERSION;
use crate::preferences_dialog::PreferencesDialog;
use crate::window::TemporalExplorerWindow;

mod imp {
    use super::*;

    #[derive(Debug, Default)]
    pub struct TemporalExplorerApplication {}

    #[glib::object_subclass]
    impl ObjectSubclass for TemporalExplorerApplication {
        const NAME: &'static str = "TemporalExplorerApplication";
        type Type = super::TemporalExplorerApplication;
        type ParentType = adw::Application;
    }

    impl ObjectImpl for TemporalExplorerApplication {
        fn constructed(&self) {
            self.parent_constructed();
            let obj = self.obj();
            obj.setup_gactions();
            obj.setup_accels();
        }
    }

    impl ApplicationImpl for TemporalExplorerApplication {
        fn activate(&self) {
            let application = self.obj();
            application.ensure_icon_resource_path();
            let window = application.active_window().unwrap_or_else(|| {
                let window = TemporalExplorerWindow::new(&*application);
                window.upcast()
            });
            window.present();
        }

        /// Opens the repositories passed on the command line.
        ///
        /// Mirrors the Nautilus, which opens the locations given as
        /// arguments: `temporal-explorer ~/projects/my-repo` opens the
        /// repository in the current window.
        ///
        /// Handling the command line replaces the default activation, so
        /// the window is created explicitly here.
        fn command_line(
            &self,
            command_line: &gio::ApplicationCommandLine,
        ) -> glib::ExitCode {
            self.obj().activate();

            let arguments = command_line.arguments();
            for argument in arguments.iter().skip(1) {
                let path = std::path::PathBuf::from(argument);
                if path.is_dir() {
                    self.obj().open_repository(path);
                    break;
                }
            }

            glib::ExitCode::SUCCESS
        }
    }

    impl GtkApplicationImpl for TemporalExplorerApplication {}
    impl AdwApplicationImpl for TemporalExplorerApplication {}
}

glib::wrapper! {
    pub struct TemporalExplorerApplication(ObjectSubclass<imp::TemporalExplorerApplication>)
        @extends gio::Application, gtk::Application, adw::Application,
        @implements gio::ActionGroup, gio::ActionMap;
}

impl TemporalExplorerApplication {
    pub fn new(application_id: &str, flags: &gio::ApplicationFlags) -> Self {
        glib::Object::builder()
            .property("application-id", application_id)
            .property("flags", flags)
            .property("resource-base-path", "/io/github/TemporalExplorer")
            .build()
    }

    /// Opens a repository in the current window.
    fn open_repository(&self, path: std::path::PathBuf) {
        let Some(window) = self
            .active_window()
            .and_downcast::<TemporalExplorerWindow>()
        else {
            return;
        };

        window.load_repository(path);
    }

    fn ensure_icon_resource_path(&self) {
        let Some(display) = gtk::gdk::Display::default() else {
            return;
        };

        let icon_theme = gtk::IconTheme::for_display(&display);
        icon_theme.add_resource_path("/io/github/TemporalExplorer/icons");

        // Flatpak's GTK icon theme only searches the runtime by default.  The
        // repository browser should render the same MIME icons as the host file
        // manager, so include the read-only host and per-user icon locations.
        if std::path::Path::new("/.flatpak-info").exists() {
            let home = glib::home_dir();
            for path in [
                home.join(".local/share/icons"),
                home.join(".icons"),
                std::path::PathBuf::from("/run/host/usr/local/share/icons"),
                std::path::PathBuf::from("/run/host/usr/share/icons"),
            ] {
                if path.is_dir() {
                    icon_theme.add_search_path(path);
                }
            }
        }
    }

    fn setup_gactions(&self) {
        let quit_action = gio::ActionEntry::builder("quit")
            .activate(move |app: &Self, _, _| app.quit())
            .build();
        let about_action = gio::ActionEntry::builder("about")
            .activate(move |app: &Self, _, _| app.show_about())
            .build();
        let preferences_action = gio::ActionEntry::builder("preferences")
            .activate(move |app: &Self, _, _| app.show_preferences())
            .build();
        let shortcuts_action = gio::ActionEntry::builder("shortcuts")
            .activate(move |app: &Self, _, _| app.show_shortcuts())
            .build();
        let help_action = gio::ActionEntry::builder("help")
            .activate(move |app: &Self, _, _| app.show_help())
            .build();

        self.add_action_entries([
            quit_action,
            about_action,
            preferences_action,
            shortcuts_action,
            help_action,
        ]);
    }

    fn setup_accels(&self) {
        self.set_accels_for_action("app.quit", &["<Control>q"]);
        self.set_accels_for_action("app.preferences", &["<Control>comma"]);
        self.set_accels_for_action("app.shortcuts", &["<Control>question"]);
        self.set_accels_for_action("app.help", &["F1"]);

        self.set_accels_for_action("win.reload-repository", &["F5"]);
        self.set_accels_for_action("win.new-window", &["<Control>n"]);
        self.set_accels_for_action("win.new-tab", &["<Control>t"]);
        self.set_accels_for_action("win.close-tab", &["<Control>w"]);
        self.set_accels_for_action("win.restore-closed-tab", &["<Control><Shift>t"]);
        self.set_accels_for_action("win.open-tab-overview", &["<Control><Shift>o"]);
        self.set_accels_for_action("win.edit-location", &["<Control>l"]);
        self.set_accels_for_action("win.prompt-root-location", &["slash", "KP_Divide"]);
        self.set_accels_for_action("win.select-all-files", &["<Control>a"]);
        self.set_accels_for_action("win.invert-file-selection", &["<Control><Shift>a"]);
        self.set_accels_for_action("win.unselect-all-files", &["<Control><Shift>l"]);
        self.set_accels_for_action("win.open-repository", &["<Control>o"]);
        self.set_accels_for_action("win.first-commit", &["<Control>Home"]);
        self.set_accels_for_action("win.latest-commit", &["<Control>End"]);
        self.set_accels_for_action("win.navigate-back", &["<Alt>Left"]);
        self.set_accels_for_action("win.navigate-forward", &["<Alt>Right"]);
        self.set_accels_for_action("win.navigate-up", &["<Alt>Up"]);
        self.set_accels_for_action("win.toggle-sidebar", &["F9"]);
        self.set_accels_for_action("win.toggle-hidden-files", &["<Control>h"]);
        self.set_accels_for_action("win.search-commits", &["<Control>f"]);
        self.set_accels_for_action("win.toggle-filter-popover", &["<Control><Shift>f"]);
        self.set_accels_for_action("win.open-date-range-dialog", &["<Control><Shift>d"]);
        self.set_accels_for_action("win.filter-file-type", &["<Control><Alt>t"]);
        self.set_accels_for_action("win.clear-all-filters", &["<Control>Escape"]);
        self.set_accels_for_action("win.list-view", &["<Control>1"]);
        self.set_accels_for_action("win.grid-view", &["<Control>2"]);
        self.set_accels_for_action("win.zoom-in", &["<Control>plus", "<Control>KP_Add"]);
        self.set_accels_for_action("win.zoom-out", &["<Control>minus", "<Control>KP_Subtract"]);
        self.set_accels_for_action("win.reset-zoom", &["<Control>0"]);
        self.set_accels_for_action("win.show-column-chooser", &["<Control><Shift>c"]);
        self.set_accels_for_action("win.show-captions", &["<Control><Shift>g"]);
        self.set_accels_for_action("win.copy-current-commit-sha", &["<Control>c"]);
        self.set_accels_for_action("win.show-current-commit-details", &["<Control>i"]);
        self.set_accels_for_action("win.preview-selected-file", &["space"]);
        self.set_accels_for_action("win.open-context-menu", &["<Shift>F10"]);
        self.set_accels_for_action("win.batch-operations", &["<Control><Shift>b"]);
        self.set_accels_for_action("win.show-merge-conflicts", &["<Control><Shift>m"]);
        self.set_accels_for_action("win.select-by-pattern", &["<Control>s"]);
        self.set_accels_for_action("win.invert-commit-selection", &["<Control><Shift>i"]);
        self.set_accels_for_action("win.star-snapshot", &["<Control>d"]);
    }

    fn show_preferences(&self) {
        let Some(window) = self.active_window() else {
            return;
        };
        let settings = gio::Settings::new("io.github.TemporalExplorer");
        let dialog = PreferencesDialog::new(&settings);
        dialog.present(Some(&window));
    }

    fn show_shortcuts(&self) {
        let Some(window) = self.active_window() else {
            return;
        };

        let builder =
            gtk::Builder::from_resource("/io/github/TemporalExplorer/shortcuts-dialog.ui");

        let Some(dialog) = builder.object::<adw::Dialog>("shortcuts_dialog") else {
            return;
        };

        dialog.present(Some(&window));
    }

    fn show_help(&self) {
        let launcher =
            gtk::UriLauncher::new("https://github.com/johnpetersa19/Temporal-Explorer#readme");
        let window = self.active_window();
        launcher.launch(window.as_ref(), gio::Cancellable::NONE, |_| {});
    }

    fn show_about(&self) {
        let Some(window) = self.active_window() else {
            return;
        };
        let about = adw::AboutDialog::builder()
            .application_name("Temporal Explorer")
            .application_icon("io.github.TemporalExplorer")
            .developer_name("John Peter Sá")
            .version(VERSION)
            .developers(vec!["John Peter Sá"])
            .translator_credits(&gettext("translator-credits"))
            .copyright("© 2026 John Peter Sá")
            .website("https://github.com/johnpetersa19/Temporal-Explorer")
            .issue_url("https://github.com/johnpetersa19/Temporal-Explorer/issues")
            .build();

        about.present(Some(&window));
    }
}

pub type Application = TemporalExplorerApplication;
