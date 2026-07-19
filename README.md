# Temporal Explorer

> Browse a Git repository exactly as it existed at any point in its history.

Temporal Explorer is a native Linux desktop application that turns Git history
into a time-navigable file browser. Select a commit from the timeline and explore
its complete directory tree without checking out the commit or changing the
current branch.

The application ID is `io.github.TemporalExplorer`.

## Current Features

### Timeline and snapshots

- Navigate commits through year and month views.
- Browse the complete file tree of any selected commit.
- Inspect files and directories that no longer exist in the working tree.
- Move backward and forward through visited commits and snapshot directories.
- View commit metadata, changed files, parent information and signature status.
- Inspect changes introduced by merge commits.

### Search and filtering

- Search commit messages, hashes, authors, dates and changed file names.
- Filter commits by branch, author, date range and file type.
- Select commits using text, glob or regular-expression patterns.
- Process large histories incrementally with paginated loading and cached indexes.

### File browser

- List and grid layouts with configurable zoom and captions.
- Sort by name, modification date, size, extension or status.
- Show or hide dotfiles.
- Preview text and binary-aware file content.
- Open snapshot files with installed applications.
- Export historical files and directories without changing the repository.
- Copy snapshot paths, file contents and repository paths.
- View item properties and mark snapshot items as favorites.

### Tabs

- Open multiple snapshot locations in tabs using the native Libadwaita tab bar.
- Preserve the commit, directory and navigation history independently in each tab.
- Open snapshot directories in a new tab from the context menu.
- Create and close tabs with `Ctrl+T` and `Ctrl+W`.

### Repository operations

- Open an existing local repository or clone one over HTTPS or SSH.
- Create, check out, push, rename and delete branches.
- Cherry-pick selected commits onto the current branch.
- Export selected commits as patch files or copy their SHAs.
- Open the repository in the system file manager or a terminal.
- Restore all tracked project files from a selected snapshot while keeping `HEAD`
  and the current branch unchanged. Restoration is blocked when local changes are
  present and the result is left as unstaged working-tree changes.

### Desktop integration

- Native GTK4 and Libadwaita interface.
- Remembers the window size, maximized state, view mode, zoom, sorting and other
  display preferences.
- Desktop, AppStream, D-Bus, GSettings and symbolic icon integration.
- Gettext localization, including a complete Brazilian Portuguese translation.

## How It Works

Temporal Explorer reads the repository directly through `libgit2`:

- **HistoryReader** streams commit metadata without checking out revisions.
- **SnapshotResolver** resolves a commit into its historical Git tree.
- **SnapshotMaterializer** reconstructs files or directories only when they are
  opened, previewed or exported.
- **DirCache** and the commit file index reduce repeated repository traversal.

Normal browsing is read-only. The working tree is modified only when the user
explicitly confirms an operation such as restoring a snapshot or cherry-picking
commits.

## Requirements

- Linux with GTK 4.14 or newer.
- Libadwaita 1.7 or newer.
- Rust toolchain and Cargo.
- Meson 1.0 or newer.
- Blueprint Compiler.
- Git, Gettext, OpenSSL and zlib.

## Building from Source

On Arch Linux, install the build dependencies:

```bash
sudo pacman -S --needed base-devel git meson rust blueprint-compiler \
  gtk4 libadwaita gettext openssl zlib
```

Configure, compile and test:

```bash
meson setup build --buildtype=release
meson compile -C build
meson test -C build --print-errorlogs
cargo test
```

Install locally:

```bash
sudo meson install -C build
```

## Arch Linux Package

The repository includes a VCS [PKGBUILD](PKGBUILD). After the current changes
have been pushed to the Git repository, build and install it with:

```bash
makepkg -si
```

The package is installed as `temporal-explorer-git` and provides
`temporal-explorer`.

## Flatpak

The Flatpak manifest is
[`io.github.TemporalExplorer.json`](io.github.TemporalExplorer.json). Build and
install it locally with:

```bash
flatpak-builder --user --install --force-clean build-dir \
  io.github.TemporalExplorer.json
```

The manifest currently targets the GNOME 50 runtime.

## Keyboard Shortcuts

The application includes a searchable shortcuts dialog. Common shortcuts include:

| Shortcut | Action |
| --- | --- |
| `Ctrl+O` | Open repository |
| `Ctrl+T` | New tab |
| `Ctrl+W` | Close tab |
| `Ctrl+F` | Search commits |
| `Ctrl+1` | List view |
| `Ctrl+2` | Grid view |
| `Ctrl+L` | Edit snapshot location |
| `F5` | Reload repository |
| `F9` | Toggle sidebar |
| `Space` | Preview selected file |

Open the in-app shortcuts dialog for the complete, current list.

## Development Status

Temporal Explorer is under active development. Repository browsing and the core
desktop integration are functional, but interfaces and repository operations may
continue to evolve before a stable 1.0 release.

## License

Temporal Explorer is licensed under GPL-3.0-or-later. See [COPYING](COPYING).
