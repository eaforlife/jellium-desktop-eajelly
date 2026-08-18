#!/usr/bin/env sh
# Wrap the self-contained AppImage in a Debian package. The launcher enables
# extract-and-run so the package does not require FUSE on the target machine.
set -eu

SCRIPT_DIR="$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)"
REPO_ROOT="$(CDPATH= cd -- "${SCRIPT_DIR}/../../.." && pwd)"
VERSION="$(cargo run --quiet --manifest-path "${REPO_ROOT}/src/xtask/Cargo.toml" -- version)"
MACHINE_ARCH="$(uname -m)"

case "${MACHINE_ARCH}" in
    x86_64) DEB_ARCH=amd64 ;;
    aarch64) DEB_ARCH=arm64 ;;
    *) echo "unsupported Debian architecture: ${MACHINE_ARCH}" >&2; exit 1 ;;
esac

APPIMAGE="${REPO_ROOT}/dist/JelliumDesktop-${VERSION}-${MACHINE_ARCH}.AppImage"
PACKAGE_ROOT="${REPO_ROOT}/build/deb/package"
OUTPUT="${REPO_ROOT}/dist/JelliumDesktop-${VERSION}-debian-${DEB_ARCH}.deb"

if [ ! -f "${APPIMAGE}" ]; then
    echo "error: ${APPIMAGE} not found; build the AppImage first" >&2
    exit 1
fi

rm -rf "${PACKAGE_ROOT}"
mkdir -p \
    "${PACKAGE_ROOT}/DEBIAN" \
    "${PACKAGE_ROOT}/opt/jellium-desktop" \
    "${PACKAGE_ROOT}/usr/bin" \
    "${PACKAGE_ROOT}/usr/share/applications" \
    "${PACKAGE_ROOT}/usr/share/icons/hicolor/scalable/apps"

sed \
    -e "s/@VERSION@/${VERSION}/g" \
    -e "s/@ARCH@/${DEB_ARCH}/g" \
    "${SCRIPT_DIR}/control.in" > "${PACKAGE_ROOT}/DEBIAN/control"

install -m 0755 "${APPIMAGE}" "${PACKAGE_ROOT}/opt/jellium-desktop/JelliumDesktop.AppImage"
install -m 0755 "${SCRIPT_DIR}/jellium-desktop" "${PACKAGE_ROOT}/usr/bin/jellium-desktop"
install -m 0644 \
    "${REPO_ROOT}/resources/linux/net.nullsum.JelliumDesktop.desktop" \
    "${PACKAGE_ROOT}/usr/share/applications/net.nullsum.JelliumDesktop.desktop"
install -m 0644 \
    "${REPO_ROOT}/resources/linux/net.nullsum.JelliumDesktop.svg" \
    "${PACKAGE_ROOT}/usr/share/icons/hicolor/scalable/apps/net.nullsum.JelliumDesktop.svg"

dpkg-deb --build --root-owner-group "${PACKAGE_ROOT}" "${OUTPUT}"
echo "Debian package: ${OUTPUT}"
