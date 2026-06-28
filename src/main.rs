mod address_bar;
mod application;
mod batch_operations_dialog;
mod branch_sidebar_row;
mod clone_repository_dialog;
mod column_chooser;
mod commit_controller;
mod commit_name_cell;
mod config;
mod date_range_dialog;
mod file_grid_captions_dialog;
mod file_grid_cell;
mod file_list_row;
mod file_preview;
mod filter_types_dialog;
mod git_engine;
mod history_controls;
mod icon_helpers;
mod merge_conflict_dialog;
mod new_branch_dialog;
mod node_properties_dialog;
mod operation_progress_dialog;
mod preferences_dialog;
mod search_filter_popover;
mod select_commits_by_pattern;
mod ssh_passphrase_dialog;
mod timeline_filter;
mod toolbar;
mod view_controls;
mod views;
mod window;

#[cfg(test)]
mod tests;

use application::Application;
use config::{GETTEXT_PACKAGE, LOCALEDIR, RESOURCES_FILE};

use gio::prelude::*;
use std::path::PathBuf;

fn load_app_resources() {
    let mut candidates: Vec<PathBuf> = Vec::new();

    if let Ok(path) = std::env::var("TEMPORAL_EXPLORER_RESOURCE_FILE") {
        candidates.push(PathBuf::from(path));
    }

    // Flatpak/default Meson installation path.
    candidates.push(PathBuf::from(RESOURCES_FILE));

    // Native Linux installation paths.
    candidates.push(PathBuf::from(
        "/usr/local/share/temporal-explorer/temporal-explorer.gresource",
    ));
    candidates.push(PathBuf::from(
        "/usr/share/temporal-explorer/temporal-explorer.gresource",
    ));

    // Developer build directories when running from the project root.
    if let Ok(cwd) = std::env::current_dir() {
        candidates.push(cwd.join("build/src/temporal-explorer.gresource"));
        candidates.push(cwd.join("builddir/src/temporal-explorer.gresource"));
        candidates.push(cwd.join("_build/src/temporal-explorer.gresource"));
    }

    for path in &candidates {
        if !path.exists() {
            continue;
        }

        match gio::Resource::load(path) {
            Ok(resource) => {
                gio::resources_register(&resource);
                eprintln!("Loaded resources from {}", path.display());
                return;
            }
            Err(err) => {
                eprintln!("Failed to load resources from {}: {err}", path.display());
            }
        }
    }

    let checked = candidates
        .iter()
        .map(|path| format!("  - {}", path.display()))
        .collect::<Vec<_>>()
        .join("\n");

    panic!("Could not load temporal-explorer.gresource. Checked:\n{checked}");
}

fn main() -> glib::ExitCode {
    // Initialise translations
    gettextrs::setlocale(gettextrs::LocaleCategory::LcAll, "");
    gettextrs::bindtextdomain(GETTEXT_PACKAGE, LOCALEDIR).expect("Unable to bind the text domain");
    gettextrs::textdomain(GETTEXT_PACKAGE).expect("Unable to switch to the text domain");

    // Load resources
    load_app_resources();

    let app = Application::new(
        "io.github.johnpetersa19.TemporalExplorer",
        &gio::ApplicationFlags::default(),
    );
    app.run()
}
