pkgdatadir = get_option('prefix') / get_option('datadir') / meson.project_name()
gnome = import('gnome')

blueprint_files = files(
  'address_bar.rs',
)

blueprints = custom_target('blueprints',
  input: files(
    'batch-operations-dialog.blp',
    'branch-sidebar-row.blp',
    'column-chooser.blp',
    'commit-name-cell.blp',
    'commit-row.blp',
    'date-range-dialog.blp',
    'file-grid-captions-dialog.blp',
    'file-grid-cell.blp',
    'file-list-row.blp',
    'file-preview-dialog.blp',
    'filter-types-dialog.blp',
    'history-controls.blp',
    'merge-conflict-dialog.blp',
    'month-row.blp',
    'new-branch-dialog.blp',
    'preferences-dialog.blp',
    'search-filter-popover.blp',
    'select-commits-by-pattern.blp',
    'ssh-passphrase-dialog.blp',
    'toolbar.blp',
    'view-controls.blp',
    'window.blp',
    'year-row.blp',
  ),
  output: '.',
  command: [find_program('blueprint-compiler'), 'batch-compile', '@OUTPUT@', '@CURRENT_SOURCE_DIR@', '@INPUT@'],
)

gnome.compile_resources('temporal-explorer',
  'temporal-explorer.gresource.xml',
  gresource_bundle: true,
  install: true,
  install_dir: pkgdatadir,
  dependencies: blueprints,
)

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

config_rs = declare_dependency(
  sources: [meson.project_build_root() / 'src' / 'config.rs']
)

exe = executable('temporal-explorer', 'main.rs',
  install: true,
  rust_args: ['--edition=2021'],
  dependencies: [
    dependency('gtk4'),
    dependency('libadwaita-1'),
    dependency('gio-2.0'),
    config_rs,
  ],
)
