# Jellium Desktop - EAJelly

Jellium Desktop - EAJelly is a preconfigured fork of [Jellium Desktop](https://github.com/andrewrabert/jellium-desktop), the unofficial [Jellyfin](https://jellyfin.org) desktop client built with [CEF](https://github.com/chromiumembedded/cef) and [mpv](https://mpv.io/).

This fork opens the eajelly service directly and never asks users to enter a server address. At startup it briefly checks the private LAN server. If that endpoint returns valid Jellyfin server information, the app uses it; otherwise it automatically uses `http://eajelly.xyz`. Users proceed directly to the normal username and password screen.

The upstream project and its contributors remain the foundation of this client. Fork-specific source and issue tracking are available in the [eajelly repository](https://github.com/eaforlife/jellium-desktop-eajelly).

## Downloads and installation

Download the installer for your system from the [latest GitHub release](https://github.com/eaforlife/jellium-desktop-eajelly/releases/latest). Release filenames begin with `JelliumDesktop`; the installed eajelly application has a different name from the old **Jellyfin Desktop** client.

### Windows

1. Uninstall the old **Jellyfin Desktop** application from **Settings > Apps > Installed apps**. Do not uninstall your Jellyfin server.
2. Download `JelliumDesktop-<version>-windows-x64-setup.exe` for a standard Intel/AMD PC, or `JelliumDesktop-<version>-windows-arm64-setup.exe` for Windows on ARM.
3. Run the installer. It will stop and show a warning if the old Jellyfin Desktop client is still installed.
4. Open **Jellium Desktop - EAJelly** from the Start menu or desktop shortcut. The new application is listed under this name, not **Jellyfin Desktop**.

Portable `.zip` builds are also attached to each release. Extract the complete archive before running `jellium-desktop.exe`; uninstall the old Jellyfin Desktop client first because portable builds do not run the installer prerequisite check.

### macOS

Download `JelliumDesktop-<version>-macos-arm64.dmg` for Apple Silicon or `JelliumDesktop-<version>-macos-x86_64.dmg` for an Intel Mac. Open it and drag **Jellium Desktop - EAJelly** into Applications.

The builds are not Apple-notarized. If macOS quarantines the app, run:

```sh
sudo xattr -cr "/Applications/Jellium Desktop - EAJelly.app"
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

## Hardware decoding

Hardware decoding defaults to **Auto**, and changes made in the client settings apply to the next video. On Windows, Auto asks mpv to use a supported decoder such as D3D11VA and falls back to software decoding when the GPU, driver, codec profile, or output path is incompatible. The **NVIDIA NVDEC (copy-back)** option uses `nvdec-copy`, avoiding CUDA/D3D11 zero-copy interoperability failures. It still decodes in hardware, but copies decoded frames through system memory before rendering, so Windows Task Manager can emphasize 3D/copy activity instead of the per-process Video Decode graph. Prefer **Auto** on Windows when it selects D3D11VA successfully.

Jellyfin reporting **Direct Play** only means the server is sending the original file without transcoding; it does not confirm that the client GPU is decoding it. Some GPU **3D** activity is normal because video presentation and window compositing still use the GPU. On multi-GPU systems, Windows may show video decoding on a different adapter or engine graph.

To verify the decoder selected by mpv, play the video and inspect `%LOCALAPPDATA%\jellium-desktop\Logs\jellium-desktop.log` for `Effective video decoder: hardware (...)` or `Effective video decoder: software`. If Auto falls back to software for an HEVC file, update the GPU driver and test **D3D11VA** or **NVIDIA NVDEC (copy-back)** in client settings.

## Versions and updates

The eajelly release line starts at `3.0.0-eajelly`. On Windows, the app checks GitHub Releases shortly after launch. When an update is available, choose **Download and install** to download the architecture-matched installer; the app closes and opens the installer when it is ready. You can also right-click and choose **Check for Updates** or open **About**. macOS and Linux currently use the manual release-page flow.

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
