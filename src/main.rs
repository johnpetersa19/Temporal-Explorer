mod address_bar;
mod application;
mod batch_operations_dialog;
mod branch_sidebar_row;
mod column_chooser;
mod commit_controller;
mod commit_name_cell;
mod config;
mod date_range_dialog;
mod file_grid_captions_dialog;
mod file_preview;
mod filter_types_dialog;
mod git_engine;
mod history_controls;
mod icon_helpers;
mod merge_conflict_dialog;
mod new_branch_dialog;
mod preferences_dialog;
mod search_filter_popover;
mod select_commits_by_pattern;
mod ssh_passphrase_dialog;
mod timeline_filter;
mod toolbar;
mod view_controls;
mod views;
mod window;

use application::Application;
use config::{GETTEXT_PACKAGE, LOCALEDIR, RESOURCES_FILE};

fn main() -> glib::ExitCode {
    // Initialise translations
    gettextrs::setlocale(gettextrs::LocaleCategory::LcAll, "");
    gettextrs::bindtextdomain(GETTEXT_PACKAGE, LOCALEDIR)
        .expect("Unable to bind the text domain");
    gettextrs::textdomain(GETTEXT_PACKAGE).expect("Unable to switch to the text domain");

    // Load resources
    let res = gio::Resource::load(RESOURCES_FILE).expect("Could not load gresource file");
    gio::resources_register(&res);

    let app = Application::new();
    app.run()
}
