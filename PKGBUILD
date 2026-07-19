# Maintainer: Temporal Explorer contributors

pkgname=temporal-explorer-git
pkgver=0.2.0.r0.g0000000
pkgrel=1
pkgdesc='Browse Git repository history as a time-navigable file tree'
arch=('x86_64' 'aarch64')
url='https://github.com/johnpetersa19/Temporal-Explorer'
license=('GPL-3.0-or-later')
depends=(
  'glib2'
  'gtk4'
  'libadwaita'
  'openssl'
  'zlib'
)
makedepends=(
  'blueprint-compiler'
  'gettext'
  'git'
  'meson'
  'rust'
)
checkdepends=(
  'appstream'
  'desktop-file-utils'
)
provides=('temporal-explorer')
conflicts=('temporal-explorer')
source=("$pkgname::git+$url.git")
sha256sums=('SKIP')

pkgver() {
  cd "$pkgname" || return 1

  local project_version commit_count commit_hash
  project_version=$(sed -n "s/^[[:space:]]*version:[[:space:]]*'\([^']*\)'.*/\1/p" meson.build)
  commit_count=$(git rev-list --count HEAD)
  commit_hash=$(git rev-parse --short=7 HEAD)
  printf '%s.r%s.g%s' "$project_version" "$commit_count" "$commit_hash"
}

build() {
  arch-meson "$pkgname" build --buildtype=release
  meson compile -C build
}

check() {
  meson test -C build --print-errorlogs
}

package() {
  DESTDIR="$pkgdir" meson install -C build
  install -Dm644 "$pkgname/COPYING" \
    "$pkgdir/usr/share/licenses/$pkgname/COPYING"
}
