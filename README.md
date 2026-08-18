# Jellium Desktop eajelly

Jellium Desktop eajelly is a preconfigured fork of [Jellium Desktop](https://github.com/andrewrabert/jellium-desktop), the unofficial [Jellyfin](https://jellyfin.org) desktop client built with [CEF](https://github.com/chromiumembedded/cef) and [mpv](https://mpv.io/).

This fork opens the eajelly service directly and never asks users to enter a server address. At startup it briefly checks the private LAN server. If that endpoint returns valid Jellyfin server information, the app uses it; otherwise it automatically uses `http://eajelly.xyz`. Users proceed directly to the normal username and password screen.

The upstream project and its contributors remain the foundation of this client. Fork-specific source and issue tracking are available in the [eajelly repository](https://github.com/eaforlife/jellium-desktop-eajelly).

## Downloads and installation

Download the installer for your system from the [latest GitHub release](https://github.com/eaforlife/jellium-desktop-eajelly/releases/latest). Release assets are produced automatically for version tags.

### Windows

Download the `windows-x64-setup.exe` installer for standard Intel/AMD Windows PCs, or `windows-arm64-setup.exe` for Windows on ARM. Run the installer and launch **Jellium Desktop eajelly** from the Start menu. Portable `.zip` builds are also attached to each release.

If the old **Jellyfin Desktop** app is installed, remove it from **Windows Settings > Apps** first. The Jellium Desktop eajelly installer checks for that legacy app and will not continue until it has been uninstalled.

### macOS

Download the DMG matching your Mac (`arm64` for Apple Silicon or `x86_64` for Intel), open it, and drag **Jellium Desktop** into Applications.

The builds are not Apple-notarized. If macOS quarantines the app, run:

```sh
sudo xattr -cr "/Applications/Jellium Desktop.app"
```

### Debian and Ubuntu

Download the `.deb` matching your system (`amd64` or `arm64`), then install it with:

```sh
sudo apt install ./JelliumDesktop-*-debian-*.deb
```

The Debian launcher uses the bundled AppImage in extract-and-run mode, so FUSE is not required. A standalone AppImage and a Flatpak bundle are also published. To use the AppImage directly:

```sh
chmod +x JelliumDesktop-*.AppImage
./JelliumDesktop-*.AppImage
```

## Versions and updates

The eajelly release line starts at `3.0.0-eajelly`. Right-click in the app and choose **Check for Updates** (or open **About**) to compare the installed version with the latest GitHub release and open its download page when an update is available.

The GitHub Actions workflows build Windows installers and portable archives, macOS DMGs, Linux AppImages, Debian packages, and a Flatpak bundle. Tag builds attach their outputs to the GitHub release automatically.

## Development

This project uses [just](https://github.com/casey/just) as its command runner. The main commands are:

```text
just deps       # install/fetch platform dependencies
just build      # build and stage the app
just test       # run workspace tests
just lint       # formatting and clippy checks
just run        # build and run the app
just appimage build
just flatpak build
just dmg
```

See [CLAUDE.md](CLAUDE.md) for architecture notes and contributor guidance.
