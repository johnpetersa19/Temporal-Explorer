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

use gettextrs::gettext;
use adw::prelude::*;
use adw::subclass::prelude::*;
use gtk::{gio, glib};

use crate::config::VERSION;
use crate::TemporalExplorerWindow;

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
            obj.set_accels_for_action("app.quit", &["<control>q"]);
        }
    }

    impl ApplicationImpl for TemporalExplorerApplication {
        fn activate(&self) {
            let application = self.obj();
            let window = application.active_window().unwrap_or_else(|| {
                let window = TemporalExplorerWindow::new(&*application);
                window.upcast()
            });
            window.present();
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
            .property("resource-base-path", "/io/github/johnpetersa19/TemporalExplorer")
            .build()
    }

    fn setup_gactions(&self) {
        let quit_action = gio::ActionEntry::builder("quit")
            .activate(move |app: &Self, _, _| app.quit())
            .build();
        let about_action = gio::ActionEntry::builder("about")
            .activate(move |app: &Self, _, _| app.show_about())
            .build();
        self.add_action_entries([quit_action, about_action]);
    }

    fn show_about(&self) {
        let window = self.active_window().unwrap();
        let about = adw::AboutDialog::builder()
            .application_name("Temporal Explorer")
            .application_icon("io.github.johnpetersa19.TemporalExplorer")
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
