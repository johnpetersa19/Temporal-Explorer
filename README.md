# Temporal Explorer

> Navigate your Git repository history as a browsable file tree.

Temporal Explorer is a Linux desktop application that transforms the native history of any Git repository into a visual, time-navigable directory tree. Instead of using complex VCS commands, you simply choose a commit or point in time and explore the project exactly as it existed at that moment — without touching your current working tree.

## How It Works

The heavy lifting stays where it belongs: with the Git ecosystem and the remote forge (GitHub, GitLab, Codeberg, Gitea, Forgejo, Bitbucket, Azure DevOps, SourceHut, and others). Temporal Explorer adds a thin layer on top:

- **HistoryReader** — reads commits, trees, and blobs from the local repository via `libgit2`.
- **SnapshotResolver** — resolves a selected revision into a complete historical tree.
- **SnapshotMaterializer** — reconstructs the directory structure for the chosen snapshot so you can browse, open, compare, or copy files from the past.

## Features

- Browse any commit as a fully navigable file tree
- View deleted files and old directory structures
- Open historical file versions without restoring them
- Copy files or directories from the past to the present
- Works with repositories hosted on any Git forge
- Native GNOME integration via GTK4 + Libadwaita

## Status

Early development — MVP in progress.

## Requirements

- Linux
- GNOME runtime 48+
- Git repository (local or cloned from any remote forge)

## License

GPL-3.0-or-later — see [COPYING](COPYING).
