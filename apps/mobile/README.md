# corntchap app (Tauri v2)

A thin native shell around the corntchap web console for Android and iOS (macOS, Windows and Linux work too, as a bonus). Name `corntchap`, version `0.1.0`, identifier `digital.camer.corntchap` (the identifier is the Play Store and App Store id: change it in `src-tauri/tauri.conf.json` **before** the first store upload, because it cannot change afterwards).

## Architecture: the app loads the console from your server

Each organisation runs its own server (for example https://corntchap.camer.digital), so the app does **not** bundle a copy of the console. It has one webview:

1. On launch it shows a small bundled page (`src/`, plain HTML/CSS/JS) that asks for the server URL.
2. Rust (`src-tauri/src/lib.rs`) checks `GET <url>/healthz` (falling back to `/.well-known/oauth-authorization-server`) natively, so there is no CORS issue, saves the origin in the app's config directory (`server.json`), and navigates the webview to `<url>/`.
3. From then on the app opens straight into the console.

Why remote and not bundled: sign-in is a same-origin cookie session through the server's OIDC flow (`/auth/oidc/login`, Keycloak, `/auth/oidc/callback`), the console calls same-origin `/api/v1`, `/api/v2` and SSE, and the server sends a strict CSP. Loading the real origin keeps all of that working with no server changes, and the console updates the moment the server does, with no store review.

### The server URL

- **First launch**: type it (default `https://corntchap.camer.digital`). A bare host such as `corntchap.example.org` gets `https://`. Only the origin is kept.
- **Change it later**: hold **three fingers** on the screen for about 1.5 seconds on any console page, or open the deep link `corntchap://server` (the picker) or `corntchap://server?url=https://other.example.org` (the picker prefilled). Reinstalling or clearing app data also resets it. On desktop, use the deep link.
- Plain `http://` is accepted for development servers. Android blocks cleartext traffic by default in release builds, so use https for real use.

### Native marker

An initialization script sets `document.documentElement.dataset.native = "1"` on every page, so the console can use `html[data-native]` CSS (it already disables overscroll bounce). The console uses `viewport-fit=cover` and `env(safe-area-inset-*)`, which work in both mobile webviews.

### Navigation and security

- Allowed in the webview: the bundled page, the configured server origin, and the OIDC provider. The provider's host is learned: after the console navigates to `/auth/oidc/login`, https hosts reached during the next 10 minutes (or until `/auth/oidc/callback`) are remembered as the IdP and allowed.
- Any other http(s), `mailto:` or `tel:` link (GitHub, docs) opens in the system browser through the opener plugin. `target=_blank` and `window.open` follow the same rule and never create a second webview.
- **IPC is for the bundled picker only.** `capabilities/default.json` has no `remote` block and grants only `allow-get-server` and `allow-set-server`; the commands are also restricted to those two in `build.rs`. The console served from your server has no IPC access.

## Prerequisites

- Rust (the repo's `rust-toolchain.toml` applies) and Node 22 with pnpm. Run `pnpm install` in `apps/mobile`.
- Linux desktop builds: `libwebkit2gtk-4.1-dev libgtk-3-dev librsvg2-dev`.
- **Android**: Android Studio (or command-line tools) with SDK platform 35, build-tools 35, **NDK** (27.x) and JDK 17. Set `JAVA_HOME`, `ANDROID_HOME` (e.g. `~/Library/Android/sdk`) and `NDK_HOME` (`$ANDROID_HOME/ndk/<version>`). Rust targets: `rustup target add aarch64-linux-android armv7-linux-androideabi i686-linux-android x86_64-linux-android`.
- **iOS** (macOS only): Xcode and `rustup target add aarch64-apple-ios aarch64-apple-ios-sim x86_64-apple-ios`. A paid Apple Developer account for devices, TestFlight and the App Store.

## Commands

Run from `apps/mobile`:

```bash
pnpm install
pnpm tauri info          # check the toolchain

# desktop (bonus)
pnpm dev

# Android: generate the project once (it lands in src-tauri/gen/android, which is git-ignored)
pnpm android:init
pnpm android:dev                 # run on a device or emulator
pnpm android:build --apk         # release APK (unsigned; see signing)
pnpm android:build --apk --debug # debug-signed APK, installs as is
pnpm android:build --aab         # Play Store bundle

# iOS (macOS)
pnpm ios:init                    # generates src-tauri/gen/apple (git-ignored)
pnpm ios:dev                     # simulator or device
pnpm ios:build --export-method app-store-connect

# regenerate icons from console/public/icons/app-icon-1024.png
pnpm icons
```

Generated Android and iOS projects stay out of version control (`src-tauri/gen/`). If you start customising them (extra permissions, a network security config, an iOS launch screen), remove the `gen/` line from `.gitignore` and commit them from then on.

## Release signing

CI is `.github/workflows/mobile.yml`: run it manually or push a tag `app-v0.1.0`. Without secrets it makes a debug APK and a best-effort iOS simulator build. With secrets it makes signed release builds. Repository secrets:

| platform | secret | content |
|---|---|---|
| Android | `ANDROID_KEYSTORE_BASE64` | `base64 -w0 release.keystore` |
| Android | `ANDROID_KEYSTORE_PASSWORD`, `ANDROID_KEY_ALIAS`, `ANDROID_KEY_PASSWORD` | keystore settings |
| iOS | `APPLE_CERTIFICATE` | base64 of the Apple Distribution `.p12` |
| iOS | `APPLE_CERTIFICATE_PASSWORD` | its password |
| iOS | `APPLE_PROVISIONING_PROFILE` | base64 of the App Store `.mobileprovision` for `digital.camer.corntchap` |
| iOS | `APPLE_DEVELOPMENT_TEAM` | the 10-character team id |

**Play Store**: create a keystore once (`keytool -genkeypair -v -keystore release.keystore -alias corntchap -keyalg RSA -keysize 2048 -validity 10000`), keep it and its passwords safe (losing it blocks updates unless you use Play App Signing, which is recommended), build the AAB, and upload it in the Play Console. Pin the upload key in Play App Signing.

**App Store**: register the identifier, create an App Store distribution certificate and provisioning profile in the Apple Developer portal, build with `--export-method app-store-connect`, then upload the `.ipa` with Transporter or `xcrun altool`, and submit through App Store Connect / TestFlight.

## Keycloak / OIDC

Nothing changes in Keycloak. The flow never leaves the server's origin except for the IdP pages: the server starts it at `/auth/oidc/login` and Keycloak redirects back to the server's own `$PUBLIC_URL/auth/oidc/callback`, the same redirect URI the browser console already uses. The app registers no redirect URI of its own, and the session cookie belongs to the server origin. The IdP's login page must be https and render in a normal webview (no popups).

## Known limitations

- Needs the server reachable: there is no offline mode. If the server is down or the URL is wrong after it was saved, the webview shows the platform error page. Use the three-finger hold or the `corntchap://server` deep link to go back to the picker; on Android a custom-scheme navigation from the page may not reach the app's navigation hook on every webview version, in which case clear the app data.
- The window background is cream while a page loads, even in dark mode (a brief flash).
- The three-finger gesture and the external-link hook depend on Tauri's navigation handler, which fires for top-level navigations only. Links inside iframes are not intercepted.
- The IdP host is learned only through a sign-in that begins at `/auth/oidc/login`. An IdP reached any other way opens in the system browser.
- The Windows and Linux desktop builds are untested. The iOS and Android builds have not been run by the author of this scaffold: CI (`mobile.yml`) and the steps above are the first real test. The iOS simulator CI step is best effort.
- GitHub Actions in `mobile.yml` that are not already used by other workflows (`setup-java`, `setup-android`, `setup-node`, `pnpm/action-setup`) are pinned to major tags, as `ci.yml` does for pnpm and Node. Pin them to commit SHAs if you want the same strictness as the others.
