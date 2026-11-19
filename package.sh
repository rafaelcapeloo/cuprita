#!/usr/bin/env bash
# package.sh: Cuprita as a Linux package for the distribution it runs on, from a release build,
# in target/package: an RPM where rpmbuild is (Fedora, RHEL, openSUSE: `sudo dnf install
# target/package/cuprita-*.rpm`), a .deb where dpkg-deb is (Debian, Ubuntu: `sudo apt install
# ./target/package/cuprita_*.deb`). Each is built where it installs, as the app links that
# system's C library. It holds the app as /usr/bin/cuprita, its launcher, icons and licence, the
# page `cuprita --serve` shows (/usr/share/cuprita/web, which `web.sh` builds), KiCad's symbol and
# footprint libraries and 3D models (/usr/share/cuprita/kicad, which `packaging/libraries.sh`
# fetches), and recommends KiCad, whose ngspice Cuprita finds with nothing to set up.
set -euo pipefail
cd "$(dirname "$0")"
rpm=$(command -v rpmbuild || true)
deb=$(command -v dpkg-deb || true)
[ -n "$rpm$deb" ] || { echo "package.sh needs rpmbuild (sudo dnf install rpm-build) or dpkg-deb (Debian and Ubuntu have it)"; exit 1; }
version=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
cargo build --release -p cuprita
./web.sh
packaging/libraries.sh
top=$(mktemp -d)
trap 'rm -rf "$top"' EXIT
here=$PWD
summary="PCB and schematic editor for KiCad projects"
about="A fast PCB and schematic editor for KiCad projects: open the project you
have or start one, and take it from the schematic to the fab's files in one
window. Files stay KiCad's."
if [ -n "$rpm" ]; then
  cat > "$top/cuprita.spec" <<EOF
Name: cuprita
Version: $version
Release: 1
Summary: $summary
License: GPL-3.0-or-later
Recommends: kicad
%global debug_package %{nil}

%description
$about

%install
install -Dm755 $here/target/release/cuprita %{buildroot}%{_bindir}/cuprita
install -Dm644 $here/crates/app/assets/cuprita.desktop %{buildroot}%{_datadir}/applications/cuprita.desktop
install -Dm644 $here/crates/app/assets/cuprita.svg %{buildroot}%{_datadir}/icons/hicolor/scalable/apps/cuprita.svg
install -Dm644 $here/crates/app/assets/cuprita.png %{buildroot}%{_datadir}/icons/hicolor/256x256/apps/cuprita.png
install -Dm644 -t %{buildroot}%{_datadir}/licenses/cuprita $here/LICENSE $here/packaging/NOTICE.txt $here/packaging/CRATES.txt $here/crates/app/assets/LICENSE-*.txt
install -Dm644 -t %{buildroot}%{_datadir}/cuprita/web $here/target/web/index.html $here/target/web/*.js $here/target/web/*.wasm $here/target/web/*.svg
cp -R $here/target/libraries %{buildroot}%{_datadir}/cuprita/kicad

%files
%{_datadir}/cuprita
%{_bindir}/cuprita
%{_datadir}/applications/cuprita.desktop
%{_datadir}/icons/hicolor/scalable/apps/cuprita.svg
%{_datadir}/icons/hicolor/256x256/apps/cuprita.png
%{_datadir}/licenses/cuprita
EOF
  rpmbuild -bb --quiet --define "_topdir $top" --define "_rpmdir $here/target/package" --define "_build_name_fmt %%{NAME}-%%{VERSION}-%%{RELEASE}.%%{ARCH}.rpm" "$top/cuprita.spec"
  echo "Made $(ls target/package/cuprita-"$version"-1.*.rpm)"
fi
if [ -n "$deb" ]; then
  root=$top/deb
  install -Dm755 target/release/cuprita "$root/usr/bin/cuprita"
  install -Dm644 crates/app/assets/cuprita.desktop "$root/usr/share/applications/cuprita.desktop"
  install -Dm644 crates/app/assets/cuprita.svg "$root/usr/share/icons/hicolor/scalable/apps/cuprita.svg"
  install -Dm644 crates/app/assets/cuprita.png "$root/usr/share/icons/hicolor/256x256/apps/cuprita.png"
  install -Dm644 LICENSE "$root/usr/share/doc/cuprita/copyright"
  install -Dm644 -t "$root/usr/share/doc/cuprita" packaging/NOTICE.txt packaging/CRATES.txt crates/app/assets/LICENSE-*.txt
  install -Dm644 -t "$root/usr/share/cuprita/web" target/web/index.html target/web/*.js target/web/*.wasm target/web/*.svg
  cp -R target/libraries "$root/usr/share/cuprita/kicad"
  # The C library the app links, as new as its symbols ask (dpkg-shlibdeps' answer, without a
  # Debian source tree); the maintainer is whoever builds it, as dh_make takes them.
  libc=$(objdump -T target/release/cuprita | grep -o 'GLIBC_[0-9.]*' | sort -uV | tail -1 | cut -d_ -f2)
  arch=$(dpkg --print-architecture)
  file=target/package/cuprita_${version}-1_$arch.deb
  mkdir -p "$root/DEBIAN" target/package
  cat > "$root/DEBIAN/control" <<EOF
Package: cuprita
Version: $version-1
Architecture: $arch
Maintainer: ${DEBFULLNAME:-$(git config user.name)} <${DEBEMAIL:-$(git config user.email)}>
Installed-Size: $(du -sk "$root/usr" | cut -f1)
Depends: libc6 (>= $libc), libgcc-s1
Recommends: kicad
Section: electronics
Priority: optional
Description: $summary
$(sed 's/^/ /' <<< "$about")
EOF
  dpkg-deb --build --root-owner-group "$root" "$file" >/dev/null
  echo "Made $file"
fi
