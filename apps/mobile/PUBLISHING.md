# Publishing the corntchap app

Goal: the app on your phone through **TestFlight** (iOS) and **Google Play internal testing** (Android), built by `.github/workflows/mobile.yml`. Do parts 1 and 2 once; after that, releasing is one tag (part 3). Part 4 is the local Android loop on your Mac.

App id (both stores): `digital.camer.corntchap`. It cannot change after the first upload.

The workflow works without any secret (debug APK, best-effort iOS simulator build, no upload), so you can run it first to check the plain build: Actions, "mobile", "Run workflow", leave `publish` off.

## How versions work

- **Version name**: from the tag (`app-v0.1.2` gives `0.1.2`); on a manual run, from `version` in `src-tauri/tauri.conf.json`.
- **Android versionCode / iOS build number**: `github.run_number`, so it always increases. The workflow passes both to Tauri with `--config` (`bundle.android.versionCode`, `bundle.iOS.bundleVersion`); nothing is committed.
- Play rejects a versionCode it has already seen, and App Store Connect rejects a repeated build number for the same version. Both are satisfied by the run number.

## 1. Google Play (internal testing)

1. **Create the app.** Play Console, "Create app": name `corntchap`, default language, "App", "Free". Accept the declarations. Then open the dashboard and finish the required tasks (privacy policy URL, app access, ads, content rating, target audience, data safety, store listing). Internal testing needs fewer of them than production, but the console tells you which are blocking.
   - Set the package name when you upload the first bundle: it must be `digital.camer.corntchap`.
   - App access: the app needs a server and an account, so choose "All or some functionality is restricted" and give the reviewer a server URL and a test login.
2. **Generate the upload keystore** (once, on your Mac). Keep the file and password somewhere safe; Play App Signing holds the real app-signing key, so a lost upload key can be reset through Play support.
   ```bash
   keytool -genkeypair -v -storetype PKCS12 -keystore upload-keystore.jks \
     -alias upload -keyalg RSA -keysize 2048 -validity 10000
   ```
   It asks for one password (store and key share it) and your name details. The alias is `upload`.
3. **Opt in to Play App Signing** when Play Console asks (default for new apps). Your keystore is the *upload* key.
4. **First upload by hand.** Google requires the very first AAB of a new app to be uploaded in the console (it verifies the signature and package id); the API cannot do it. Build it locally or take it from CI:
   - CI: run the workflow with `publish` off, after setting the three `ANDROID_*` secrets below; download the `corntchap-android` artifact and take `app/build/outputs/bundle/universalRelease/app-universal-release.aab`.
   - Local: `pnpm android:init`, create `src-tauri/gen/android/keystore.properties` and patch `app/build.gradle.kts` as in the [Tauri Android signing guide](https://v2.tauri.app/distribute/sign/android/), then `pnpm android:build --aab`.
   Then Play Console, Testing, **Internal testing**, "Create new release", upload the AAB, save, "Review release", "Start rollout to Internal testing". Create a testers list (your Google account email) under the Testers tab and open the opt-in link on your phone.
5. **Service account for CI.**
   1. Google Cloud console: create a project (or use one), enable the **Google Play Android Developer API**, then IAM, Service accounts, create `corntchap-play-upload`, then Keys, Add key, JSON. Download the JSON.
   2. Play Console, Users and permissions (or Setup, API access), "Invite new users", paste the service account email, and give it access to the corntchap app with the **Release** permissions (release to production/testing tracks; "Release apps to testing tracks" is enough).
   3. Allow a few minutes for permissions to propagate.
6. **GitHub secrets** (repo Settings, Secrets and variables, Actions):

   | secret | value |
   |---|---|
   | `ANDROID_KEYSTORE_BASE64` | `base64 -i upload-keystore.jks \| tr -d '\n'` (macOS) or `base64 -w0 upload-keystore.jks` (Linux) |
   | `ANDROID_KEYSTORE_PASSWORD` | the keystore password from step 2 |
   | `ANDROID_KEY_ALIAS` | `upload` |
   | `PLAY_SERVICE_ACCOUNT_JSON` | the whole JSON file content (paste it as is) |

   The upload step uses `status: completed`, which works once the app has a rolled-out release (step 4). Before that, Play only accepts drafts.

## 2. App Store Connect (TestFlight)

1. **Register the bundle id.** developer.apple.com, Certificates, Identifiers & Profiles, Identifiers, "+", App IDs, App: description `corntchap`, explicit Bundle ID `digital.camer.corntchap`. No special capabilities are needed. (Deep links use a custom URL scheme, which needs none.)
2. **Create the app record.** appstoreconnect.apple.com, Apps, "+", New App: iOS, name `corntchap`, primary language, bundle id from step 1, SKU `corntchap`, User access "Full Access".
3. **API key for uploads.** App Store Connect, Users and Access, Integrations, App Store Connect API, "Team Keys" (or "Individual Keys" on some accounts), "+", name `corntchap-ci`, access **App Manager**. Note the **Issuer ID** (above the table) and the **Key ID**; download the `.p8` (possible only once, and the button appears after a page reload).
4. **Distribution certificate.** Either create a certificate signing request in Keychain Access (Certificate Assistant, "Request a Certificate From a Certificate Authority", save to disk) and, in the developer portal, Certificates, "+", **Apple Distribution**, upload the request, download the `.cer`, double-click it; or let Xcode do it (Settings, Accounts, Manage Certificates, "+", Apple Distribution). Then in Keychain Access, My Certificates, right-click the "Apple Distribution: ..." entry, Export as `distribution.p12` with a password.
5. **Provisioning profile.** Developer portal, Profiles, "+", Distribution, **App Store Connect**, App ID `digital.camer.corntchap`, the certificate from step 4, name it `corntchap AppStore`, download `corntchap_AppStore.mobileprovision`. The profile's App ID must equal `identifier` in `tauri.conf.json`, as the [Tauri iOS signing guide](https://v2.tauri.app/distribute/sign/ios/) requires.
6. **Team id.** developer.apple.com, Membership details, "Team ID" (10 characters).
7. **GitHub secrets**:

   | secret | value |
   |---|---|
   | `IOS_CERTIFICATE` | `base64 -i distribution.p12 \| pbcopy` (paste) |
   | `IOS_CERTIFICATE_PASSWORD` | the password set when exporting the `.p12` |
   | `IOS_MOBILE_PROVISION` | `base64 -i corntchap_AppStore.mobileprovision \| pbcopy` |
   | `APPLE_DEVELOPMENT_TEAM` | the Team ID |
   | `APPLE_API_KEY_ID` | the Key ID from step 3 |
   | `APPLE_API_ISSUER` | the Issuer ID from step 3 |
   | `APPLE_API_KEY_P8` | `base64 -i AuthKey_XXXXXXXXXX.p8 \| pbcopy` |

   The workflow decodes `APPLE_API_KEY_P8` to `~/.appstoreconnect/private_keys/AuthKey_<id>.p8` and uploads the `.ipa` with `xcrun altool --upload-app --type ios --apiKey ... --apiIssuer ...`.
8. **Add yourself as an internal tester.** App Store Connect, Users and Access: your Apple ID must be a user of the team. Then the app, TestFlight, Internal Testing, "+" to create a group, add yourself, enable automatic distribution for new builds. A processed build shows up on the TestFlight app on your phone (processing takes about 5 to 30 minutes after the upload; internal testers need no beta review).

## 3. Releasing

- **Tag**: `git tag app-v0.1.0 && git push origin app-v0.1.0`. Builds both platforms and publishes (a tag always publishes).
- **Manual**: Actions, "mobile", "Run workflow", tick `publish`. The version name comes from `tauri.conf.json`.
- Publish steps are skipped, not failed, when their secrets are missing; the signed artifacts are still attached to the run.

## 4. Local Android dev loop (Mac)

1. Install Android Studio. In SDK Manager install: Android SDK Platform 35, Build-Tools 35.0.0, **NDK (Side by side) 27.2.12479018**, Platform-Tools. Install JDK 17 (Android Studio's bundled JBR works).
2. Environment (e.g. `~/.zshrc`):
   ```bash
   export JAVA_HOME="/Applications/Android Studio.app/Contents/jbr/Contents/Home"
   export ANDROID_HOME="$HOME/Library/Android/sdk"
   export NDK_HOME="$ANDROID_HOME/ndk/27.2.12479018"
   export PATH="$PATH:$ANDROID_HOME/platform-tools"
   ```
3. Rust targets and dependencies:
   ```bash
   rustup target add aarch64-linux-android armv7-linux-androideabi i686-linux-android x86_64-linux-android
   cd apps/mobile && pnpm install
   ```
4. `pnpm android:init` once (generates `src-tauri/gen/android`, not committed).
5. Start an emulator (Android Studio, Device Manager) or plug in a phone with USB debugging on, then `pnpm android:dev`. Tauri starts Vite on port 1420 for the picker and reloads it live.
6. The dev build runs the picker from Vite and then loads the console from whatever server you type in. A real server needs nothing special. To point at a **local** server (`just serve` on your Mac) from a physical phone, use the Mac's LAN address (`http://192.168.x.y:8080`; plain http is allowed for development), and from the Android emulator use `http://10.0.2.2:8080`. Vite itself is reached through `TAURI_DEV_HOST`, which `tauri android dev` sets for you (the config reads it); the phone and the Mac must be on the same network.
7. `pnpm android:build --apk --debug` makes an installable debug APK (`adb install`).

## Store requirements to prepare (not generated here)

- **Privacy policy URL**: required by both stores. You must host one.
- **Icons**: the Android launcher icons and iOS `AppIcon-*` set are generated in `src-tauri/icons/`. The 1024x1024 App Store icon (`ios/AppIcon-512@2x.png`) must have **no alpha channel**; the committed iOS icons were flattened to RGB for that. `pnpm icons` (`tauri icon`) writes RGBA again, so after running it, convert the files in `src-tauri/icons/ios/` back to RGB (for example with Pillow: `Image.open(f).convert("RGB").save(f)`) before an iOS release. Play also wants a 512x512 store icon and a 1024x500 feature graphic, uploaded in the console.
- **Screenshots**: App Store Connect needs 6.9" or 6.5" iPhone screenshots for the store listing (not needed for TestFlight internal testing); Play needs at least 2 phone screenshots for the listing.
- **Export compliance**: `src-tauri/Info.plist` sets `ITSAppUsesNonExemptEncryption` to `false` (the app only uses the system's HTTPS). Tauri merges that file into the generated iOS Info.plist, per its App Store guide. Change it to `true` if you add your own cryptography.
- **Review notes** (when you go beyond internal testing): the app needs a server and a login; give the reviewers a demo server and account.
