# Project Notes

## Build / Run
All app code is Rust; the cargo workspace lives in `src/` and produces the `jellium-desktop` binary. Everything is driven through `just` — recipes are OS-gated via `[macos]`/`[linux]`/`[windows]` attributes, so the same command works everywhere:
```
just deps      # one-time: submodules, CEF download, macOS brew packages
just build     # build + stage a runnable tree in build/ (+ .app bundle on macOS)
just test      # run the workspace test suite (depends on build)
just run       # run with debug logging → logs to build/run.log
just run-mpv   # run the bundled mpv CLI directly (mpv-only debugging)
just clean     # remove build/ and dist/, cargo clean
just fmt        # format the workspace (cargo fmt)
just fmt-check  # check formatting without writing
just clippy     # clippy with -D warnings + unwrap/expect/panic denies
just lint       # fmt-check + clippy
just strict-lint # lint + clippy pedantic/nursery
just appimage build # [linux] build AppImage
just flatpak build  # [linux] build Flatpak bundle
just dmg            # [macos] build Apple Disk Image (.dmg)
```
Platform-specific entry points live in `dev/linux/`, `dev/macos/`, `dev/windows/`, imported by the top-level justfile.

## Before Committing
Run `just fmt` and `just lint` before every commit; both must pass clean (lint runs `fmt-check` + `clippy`, and CI rejects unformatted or lint-failing code).

For a documentation-only commit, still run at least `just fmt-check`,
`cargo metadata --format-version 1 --locked --manifest-path src/Cargo.toml`,
and `git diff --check`. Preserve unrelated working-tree changes and confirm
the final diff contains only the requested files.

## Upstream Merge Procedure

Treat `upstream` as a source tree whose commits may be coupled across many
crates. Before merging:

1. Fetch the upstream remote and record the newest upstream commit and its
   author timestamp.
2. Require the newest upstream commit to be at least five full days old before
   merging it. If it is newer, defer the merge and report the commit date;
   merge sooner only when the user explicitly overrides this stabilization
   window.
3. Review the complete upstream range and compare the coupled boundary crates:
   `jfn_cef`, `jfn_rust`, `config`, `platform_abi`, `playback`, `windows`,
   `macos`, `x11`, `wayland`, and `Cargo.lock`. Do not merge one side of an
   API migration in isolation.
4. Prefer a coherent upstream generation for a coupled crate family, then
   reapply this fork's customizations on top. Afterward audit the fork-specific
   diff and verify that custom server behavior, the app-owned miniplayer/PiP
   control, branding, packaging, and workflow changes still exist and use the
   new APIs.
5. Resolve conflicts by following the resulting API contract, not by keeping
   both old and new symbols. Search for stale names and conflict markers after
   every migration, especially platform lifecycle methods, scale types,
   surface presentation, unsafe platform access, CEF startup APIs, and typed
   hardware-decoding settings.
6. Regenerate `src/Cargo.lock` only with the repository's pinned dependencies
   when required, then rerun the exact locked metadata command. Never commit a
   manually edited or stale lockfile.

Before pushing a merged change, run the relevant target checks locally and
inspect every affected GitHub Actions workflow. For Windows bindgen builds,
verify that a loadable native `libclang.dll` is selected through
`LIBCLANG_PATH`; installing a package is insufficient if the DLL or its
dependencies cannot be loaded by the MSVC-hosted Rust process. For packaged
desktop builds, smoke-test startup and inspect the platform log, not just the
compiler output.

Platform startup ordering is significant: asynchronous MPV property
observations may not have reached playback state when native platform setup
runs. A non-event startup path may use a synchronous property fallback when
needed; never perform synchronous MPV reads from an MPV event callback.

## Release and Push Scope

Do not create, move, delete, or push tags unless the user explicitly requests
release/tag work. If the user asks for a commit to `main` only, push only the
requested branch and leave tags and releases unchanged. Before a release tag,
the tree must be clean, the locked metadata check must pass, all relevant
workflow filters must match the tag, and the affected platform artifacts must
be buildable.

Release tag names are always `vX.Y.Z-eajelly` (for example,
`v3.2.1-eajelly`), including the leading `v`. The Cargo package version stays
`X.Y.Z-eajelly` without the leading `v` because Cargo requires a valid SemVer
value. Before tagging, confirm `cargo xtask version` matches the tag after
removing only the leading `v`; never create the unprefixed tag variant.

## Update Detection and Release Propagation

Release assets appearing on GitHub does not by itself prove that clients can
detect an update. Before diagnosing a client, allow every platform workflow to
finish; a release build can take up to 30 minutes. Then verify the GitHub
latest-release API returns the expected tag, the release is not draft, and the
architecture-specific assets are attached to that same release.

The client update path has two separate contracts that must both be checked:

- The renderer updater calls `/repos/eaforlife/jellium-desktop-eajelly/releases/latest`,
  compares the release tag with the embedded app version, and selects the
  exact Windows installer suffix (`-windows-x64-setup.exe` or
  `-windows-arm64-setup.exe`). The injected `__UPDATE_ASSET_SUFFIX__` must be
  replaced at runtime; a literal placeholder makes every asset lookup fail.
- The selected asset is handed to native `installUpdate`, which must be in the
  CEF injection function list and routed to the trusted installer verifier.
  Verify the URL prefix, SHA-256 digest, declared size, and installer `MZ`
  header before launch.

Useful focused checks:

```sh
cargo metadata --format-version 1 --locked --manifest-path src/Cargo.toml
cargo test --manifest-path src/Cargo.toml -p jfn-platform-abi --locked --lib
node --check src/web/native-shim.js
rg -n "__UPDATE_ASSET_SUFFIX__|replace_first.*UPDATE_ASSET_SUFFIX|versionParts|isNewer" src
curl -fsSL https://api.github.com/repos/eaforlife/jellium-desktop-eajelly/releases/latest | jq '{tag_name,draft,assets:[.assets[].name]}'
```

In a packaged client, inspect `navigator.userAgent` and
`window.jmpInfo.version`; compare the numeric app version with the latest
release tag after removing its leading `v` and `-eajelly` suffix. A 30-minute
workflow build window is normal, so do not diagnose a missing asset or update
before all platform workflows finish.

The app-owned PiP button requires all three layers: `setPictureInPicture`
must be in the CEF injection allowlist, routed by `business_web`, and
implemented by the platform backend. Keep playback-finished/canceled/error
events clearing the browser PiP state, and test both entering and exiting PiP
after a video ends.

For the `windows` crate APIs, check the generated return type before writing
error handling: APIs such as `SetWindowPos` return `Result<(), Error>` and
must use `.is_ok()`/`?`, while BOOL-returning APIs expose `.as_bool()`.

Never hold the Windows window-state mutex while calling Win32 window APIs;
`SetWindowPos` can synchronously re-enter the window hook and deadlock. Keep
the app-owned PiP control in an isolated Shadow DOM host with important inline
positioning so server CSS cannot remove or restyle its internal button.

When changing release or updater code, test the packaged artifact—not only the
workspace build—by opening About and by checking the automatic update prompt
against a newer release. Record the release tag, asset names, workflow
completion state, API response, and client log before changing version logic.

The browser User-Agent is part of the server integration contract. Keep the
explicit `Jellium Desktop/<version>` token (plus the legacy compact token) in
the CEF browser User-Agent so server JavaScript can identify the client and
extract its version. Test `navigator.userAgent` in a packaged build; changing
only the MPV User-Agent does not affect server JavaScript.

Updater code is compiled in every platform workspace even when installation
is Windows-only. Keep its dependencies declared in `src/jfn_cef/Cargo.toml`
and the workspace dependency table, update `src/Cargo.lock`, and use the
current logging API (`jfn_logging::Category` and `jfn_logging::Level`) rather
than removed compatibility constants. A passing Windows-only code path is not
enough: run locked metadata and a focused `jfn-cef` check before retagging.

## Token-Efficient Agent Workflow

Start with `git status --short`, then use targeted `rg` searches and bounded
file slices. Do not dump lockfiles, generated assets, or full workflow logs;
inspect only the matching sections and relevant failure tail. Read the nearest
implementation and its tests before broad repository exploration. Run the
smallest relevant formatter, metadata check, test, or package build first;
reserve full multi-platform builds for release validation. Summaries should
record commands, pass/fail results, and remaining uncertainty without copying
large tool outputs.

## Before Version Tags / Releases
Before committing a new version or creating a release tag, run the same locked
metadata lookup used by CI:
`cargo metadata --format-version 1 --locked --manifest-path src/Cargo.toml`.
If Cargo reports that it would update `src/Cargo.lock`, regenerate the lockfile
with the repository's pinned dependencies, commit the resulting `Cargo.lock`,
and rerun the locked lookup before tagging. Never tag a release with a dirty
or out-of-date lockfile.

When converting iced widgets with `.into()` into a local variable, give the
variable an explicit `Element<'_, Message, Theme, Renderer>` type whenever no
surrounding expression provides the expected type. Otherwise iced's generic
theme/renderer parameters can fail to infer in CI even when the code works in
an editor.

## Constants
Every timing, interval or threshold derives from a value the system reports or
from a value a requirements note records. A plan that is silent on a value the
work needs is a blocker back to planning: stop and ask. Inventing the value and
writing a justification for it is the failure mode, not the mitigation.

## Architecture
- **CEF** (Chromium Embedded Framework) — hosts jellyfin-web as an embedded browser; handles JS-to-Rust IPC for player control commands and renders the UI as an overlay texture above the video layer. Multi-process: browser process (main app, owns CefBrowser), renderer process (V8/Blink), GPU process. IPC via `CefProcessMessage`. Bindings via `cef-dll-sys`; project glue lives in `src/jfn_cef`.
- **mpv** (fork in `third_party/mpv`) — video playback; the desktop client injects native shims to override browser media playback. mpv owns its own window + GPU; libmpv is used only for the control plane (properties/commands/events).
- Wayland subsurface for video layer (Linux). Platform crates: `src/macos`, `src/windows`, plus Wayland/X11 paths under `src/`.

## mpv Integration
- **Never call sync mpv API (`mpv_get_property`, etc.) from event callbacks** - causes deadlock during video init. Use property observation or async variants instead.
- Settings that affect mpv must update both the persisted config and the live handle. Route live updates through `jfn_mpv::api::apply_runtime_setting`; boot options remain the fallback for the next process/media load.
- On Windows, keep the decoder spellings aligned with mpv: `d3d11va` is the D3D11VA backend and `nvdec-copy` is the supported NVIDIA copy-back backend. Treat legacy `nvdec` as an input alias, not as a stored value.
- A requested hardware decoder is not proof of GPU use. Verify mpv's `hwdec-current` after media load and log whether it selected hardware or fell back to software.

## mpv Event Flow
mpv is the authoritative source of playback state. All state (position, speed, pause, seeking, etc.) flows from mpv property observations outward to the JS UI and OS media sessions. The JS UI and MPRIS/macOS media sessions are consumers — they never determine playback state, they only reflect what mpv reports. This means things like rate changes, seek completion, and position updates come from mpv, not from JS round-trips or manual bookkeeping.
