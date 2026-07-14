pkgdatadir = get_option('prefix') / get_option('datadir') / meson.project_name()
gnome = import('gnome')

# ── 1. Compile Blueprint files ─────────────────────────────────────────────
blueprints = custom_target('blueprints',
  input: files(
    'batch-operations-dialog.blp',
    'branch-sidebar-row.blp',
    'clone-repository-dialog.blp',
    'column-chooser.blp',
    'commit-details-dialog.blp',
    'commit-name-cell.blp',
    'commit-row.blp',
    'date-range-dialog.blp',
    'file-grid-captions-dialog.blp',
    'file-grid-cell.blp',
    'file-list-row.blp',
    'file-list-view.blp',
    'file-preview-dialog.blp',
    'filter-types-dialog.blp',
    'history-controls.blp',
    'merge-conflict-dialog.blp',
    'month-row.blp',
    'new-branch-dialog.blp',
    'node-properties-dialog.blp',
    'operation-progress-dialog.blp',
    'preferences-dialog.blp',
    'search-filter-popover.blp',
    'select-commits-by-pattern.blp',
    'shortcuts-dialog.blp',
    'ssh-passphrase-dialog.blp',
    'toolbar.blp',
    'view-controls.blp',
    'window.blp',
    'year-row.blp',
  ),
  output: '.',
  command: [find_program('blueprint-compiler'), 'batch-compile', '@OUTPUT@', '@CURRENT_SOURCE_DIR@', '@INPUT@'],
)

# ── 2. Compile GResources (depends on blueprints) ─────────────────────────
gnome.compile_resources('temporal-explorer',
  'temporal-explorer.gresource.xml',
  gresource_bundle: true,
  install: true,
  install_dir: pkgdatadir,
  dependencies: blueprints,
)

# ── 3. Generate config.rs ──────────────────────────────────────────────────
conf = configuration_data()
conf.set_quoted('VERSION', meson.project_version())
conf.set_quoted('GETTEXT_PACKAGE', meson.project_name())
conf.set_quoted('LOCALEDIR', get_option('prefix') / get_option('localedir'))
conf.set_quoted('PKGDATADIR', pkgdatadir)

configure_file(
  input: 'config.rs.in',
  output: 'config.rs',
  configuration: conf,
)

# ── 4. Build with Cargo ────────────────────────────────────────────────────
cargo = find_program('cargo', required: true)

if get_option('buildtype') == 'release'
  rust_target = 'release'
  cargo_flags = ['--release']
else
  rust_target = 'debug'
  cargo_flags = []
endif

cargo_home = meson.project_build_root() / 'cargo-home'
cargo_manifest = meson.project_source_root() / 'Cargo.toml'
cargo_target_dir = meson.project_build_root() / 'target'

# Cargo performs its own incremental compilation, but Meson still needs to
# know when invoking Cargo is necessary.  Keeping these inputs explicit avoids
# running `cargo build` on every `meson compile` while still rebuilding after a
# manifest, lockfile, or application source change.
cargo_inputs = files(
  '../Cargo.toml',
  '../Cargo.lock',
  'address_bar.rs',
  'application.rs',
  'batch_operations_dialog.rs',
  'branch_sidebar_row.rs',
  'clone_repository_dialog.rs',
  'column_chooser.rs',
  'commit_controller.rs',
  'commit_details_dialog.rs',
  'commit_name_cell.rs',
  'config.rs',
  'date_range_dialog.rs',
  'file_grid_captions_dialog.rs',
  'file_grid_cell.rs',
  'file_list_row.rs',
  'file_preview.rs',
  'filter_types_dialog.rs',
  'git_engine.rs',
  'history_controls.rs',
  'icon_helpers.rs',
  'main.rs',
  'merge_conflict_dialog.rs',
  'new_branch_dialog.rs',
  'node_properties_dialog.rs',
  'operation_progress_dialog.rs',
  'preferences_dialog.rs',
  'search_filter_popover.rs',
  'select_commits_by_pattern.rs',
  'ssh_passphrase_dialog.rs',
  'timeline_filter.rs',
  'toolbar.rs',
  'view_controls.rs',
  'views/file_list_view.rs',
  'views/grid_view.rs',
  'views/list_view.rs',
  'views/mod.rs',
  'window.rs',
)

if get_option('buildtype') == 'release'
  cargo_shell_command = 'CARGO_HOME="$1" "$2" build --release --manifest-path "$3" --target-dir "$4" && cp "$4/$5/temporal-explorer" "$6"'
else
  cargo_shell_command = 'CARGO_HOME="$1" "$2" build --manifest-path "$3" --target-dir "$4" && cp "$4/$5/temporal-explorer" "$6"'
endif

cargo_build = custom_target('cargo-build',
  build_by_default: true,
  depend_files: cargo_inputs,
  output: 'temporal-explorer',
  console: true,
  install: true,
  install_dir: get_option('bindir'),
  install_mode: 'rwxr-xr-x',
  command: [
    'sh', '-c',
    cargo_shell_command,
    'sh',
    cargo_home,
    cargo.full_path(),
    cargo_manifest,
    cargo_target_dir,
    rust_target,
    '@OUTPUT@',
  ],
)
