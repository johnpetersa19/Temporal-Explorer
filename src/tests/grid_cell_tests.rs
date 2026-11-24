// src/tests/grid_cell_tests.rs
//
// Regression tests for the grid cell measurement.
//
// The Nautilus grid cell reports the very same minimum and natural width.
// That is what keeps the `GtkGridView` from laying out one column more
// than fits in the visible area, which used to clip the last column.

use gtk::prelude::*;
use std::path::PathBuf;
use std::sync::OnceLock;

/// Registers the application resources, so that the composite templates
/// can be instantiated.  Returns `false` when they cannot be found, in
/// which case the widget tests are skipped.
fn register_resources() -> bool {
    let resource_path = "/io/github/TemporalExplorer/file-grid-cell.ui";

    if gio::resources_get_info(resource_path, gio::ResourceLookupFlags::NONE).is_ok() {
        return true;
    }

    let mut candidates: Vec<PathBuf> = Vec::new();

    if let Ok(path) = std::env::var("TEMPORAL_EXPLORER_RESOURCE_FILE") {
        candidates.push(PathBuf::from(path));
    }

    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    candidates.push(manifest.join("build/src/temporal-explorer.gresource"));
    candidates.push(manifest.join("../build/src/temporal-explorer.gresource"));
    candidates.push(manifest.join("../_build/src/temporal-explorer.gresource"));
    candidates.push(manifest.join("../builddir/src/temporal-explorer.gresource"));

    for path in candidates {
        if !path.exists() {
            continue;
        }

        if let Ok(resource) = gio::Resource::load(&path) {
            gio::resources_register(&resource);
            if gio::resources_get_info(resource_path, gio::ResourceLookupFlags::NONE).is_ok() {
                return true;
            }
        }
    }

    false
}

/// Returns `true` when the widget tests can run.
///
/// The result is cached in a `OnceLock` because calling `gtk::init()` a second
/// time in the same process after an initial failure (e.g., headless CI without
/// a display) can crash GTK4 with SIGSEGV.
fn can_run_widget_tests() -> bool {
    static READY: OnceLock<bool> = OnceLock::new();
    *READY.get_or_init(|| {
        if gtk::init().is_err() {
            eprintln!("skipping grid cell tests: no display available");
            return false;
        }

        if !register_resources() {
            eprintln!("skipping grid cell tests: resources not found");
            return false;
        }

        true
    })
}

/// A grid cell must report a fixed width for both long and short file names.
/// A growing natural width is what made the last column of the grid get clipped.
///
/// Both cases run in a single test function so GTK4 is initialized and accessed
/// exclusively from one thread even under Cargo's multi-threaded test runner.
#[test]
fn grid_cell_keeps_a_fixed_width_for_long_and_short_names() {
    if !can_run_widget_tests() {
        return;
    }

    let long_cell = crate::file_grid_cell::FileGridCell::new();
    long_cell.set_cell_width(132);
    long_cell.set_name("org.gnome.gnome-system-monitor.policy.in");

    let (minimum, natural, _, _) = long_cell.measure(gtk::Orientation::Horizontal, -1);

    assert_eq!(
        minimum, 132,
        "the cell must not request less than its fixed width"
    );
    assert_eq!(
        natural, 132,
        "the cell must not grow beyond its fixed width"
    );

    let short_cell = crate::file_grid_cell::FileGridCell::new();
    short_cell.set_cell_width(132);
    short_cell.set_name("a.txt");

    let (minimum, natural, _, _) = short_cell.measure(gtk::Orientation::Horizontal, -1);

    assert_eq!(minimum, 132);
    assert_eq!(natural, 132);
}
