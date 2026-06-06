pkgdatadir = get_option('prefix') / get_option('datadir') / meson.project_name()
gnome = import('gnome')

# ── Blueprint ───────────────────────────────────────────────────────────────
# Compile .blp files to .ui before bundling resources.
blueprint_compiler = find_program('blueprint-compiler', version: '>= 0.12')

blueprints = custom_target(
  'blueprints',
  input: files('window.blp'),
  output: '.',
  command: [
    blueprint_compiler, 'batch-compile', '@OUTPUT@', '@CURRENT_SOURCE_DIR@', '@INPUT@'
  ],
)

# ── GResources ──────────────────────────────────────────────────────────────
# source_dir includes the blueprint build output directory so that
# glib-compile-resources can locate the generated window.ui file.
gnome.compile_resources(
  'temporal-explorer',
  'temporal-explorer.gresource.xml',
  gresource_bundle: true,
  install: true,
  install_dir: pkgdatadir,
  dependencies: blueprints,
  source_dir: [
    meson.current_source_dir(),
    meson.current_build_dir(),
  ],
)

# ── Config ─────────────────────────────────────────────────────────────────
conf = configuration_data()
conf.set_quoted('VERSION', meson.project_version())
conf.set_quoted('GETTEXT_PACKAGE', 'temporal-explorer')
conf.set_quoted('LOCALEDIR', get_option('prefix') / get_option('localedir'))
conf.set_quoted('PKGDATADIR', pkgdatadir)

configure_file(
  input: 'config.rs.in',
  output: 'config.rs',
  configuration: conf,
)

run_command(
  'cp',
  meson.project_build_root() / 'src' / 'config.rs',
  meson.project_source_root() / 'src' / 'config.rs',
  check: true,
)

# ── Cargo / Rust ────────────────────────────────────────────────────────────
cargo_bin  = find_program('cargo')
cargo_opt  = ['--manifest-path', meson.project_source_root() / 'Cargo.toml']
cargo_opt += ['--target-dir', meson.project_build_root() / 'src']
cargo_env  = ['CARGO_HOME=' + meson.project_build_root() / 'cargo-home']

if get_option('buildtype') == 'release'
  cargo_opt += ['--release']
  rust_target = 'release'
else
  rust_target = 'debug'
endif

cargo_build = custom_target(
  'cargo-build',
  build_by_default: true,
  build_always_stale: true,
  output: meson.project_name(),
  console: true,
  install: true,
  install_dir: get_option('bindir'),
  depends: blueprints,
  command: [
    'env', cargo_env,
    cargo_bin, 'build',
    cargo_opt, '&&', 'cp',
    'src' / rust_target / meson.project_name(), '@OUTPUT@',
  ],
)
