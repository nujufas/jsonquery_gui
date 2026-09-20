# Publishing to Google Play

Everything the repo can do for a Play release is automated (`scripts/`,
`play/`, `.github/workflows/android.yml`). What can only be done by a person in
Play Console is listed here, in order. Times assume a **personal** developer
account created after 13 Nov 2023 — see step 6.

## 0. Decide the application id (permanent)

`io.github.nujufas.jsonquery` (in `app/build.gradle.kts`, `applicationId`). It
cannot change once the app is published. Change it *before* the first upload if
you want a different one — also `namespace`, the Kotlin package, and the JNI
names in `rust/src/native.rs` (`Java_io_github_nujufas_jsonquery_…`).

## 1. One-time account and app setup (Play Console, by hand)

1. Register at <https://play.google.com/console/signup> (one-time US$25). A
   personal account also needs identity verification.
2. **Create app** → name `jsonquery: JSON viewer & query`, default language
   English (US), *App*, *Free*. Accept the declarations.
3. Complete the **Set up your app** tasks. The answers this app needs are in
   [`play-console-answers.md`](play-console-answers.md); the privacy policy is
   [`privacy-policy.md`](privacy-policy.md) — Play needs a **public URL**, use
   `https://github.com/nujufas/jsonquery_gui/blob/master/android/docs/privacy-policy.md`
   (or publish it with GitHub Pages).
4. **Play App Signing** is mandatory for new apps and is enabled automatically
   when you upload your first bundle: Google keeps the real signing key, and
   you sign uploads with the **upload key** made by `scripts/keystore.sh`.

## 2. Make the upload key and the first bundle

```sh
android/scripts/keystore.sh            # creates android/signing/ (git-ignored) -- BACK IT UP
android/scripts/build.sh aab           # android/dist/jsonquery-<version>.aab
android/scripts/preflight.sh           # must print "preflight passed"
```

`preflight.sh` checks what Play would reject: target API 36, a 64-bit library,
16 KB page alignment, a real (non-debug) signature, and the store listing.

## 3. The first release is uploaded by hand

Google Play's API can't create an app or accept its *first* bundle. In Play
Console: **Testing → Internal testing → Create new release** → upload
`android/dist/jsonquery-<version>.aab` (the native debug symbols are embedded in
the bundle, so Play can symbolicate crashes with nothing extra to upload). Add
yourself as an internal tester and install from the opt-in link.

Fill in the **Main store listing** from `play/metadata/android/en-US/`
(title, short and full description, `images/icon.png`,
`images/featureGraphic.png`, `images/phoneScreenshots/*`).

## 4. Automate later releases (optional)

1. Play Console → **Setup → API access** → link (or create) a Google Cloud
   project; in Cloud Console enable the **Google Play Android Developer API**
   and create a **service account** (no roles needed there), then create a JSON
   key for it.
2. Back in Play Console → **Users and permissions → Invite new users** → the
   service account's email → grant **Release apps to testing tracks** (and, for
   production, **Release to production…**).
3. Save the key as `android/signing/play-service-account.json` (git-ignored) or
   in the `PLAY_SERVICE_ACCOUNT_JSON` environment variable.

Then, for every release:

```sh
# 1. bump the version: `version` in the root Cargo.toml (workspace), and
android/scripts/bump-version.sh                  # versionCode +1 (Play insists it only goes up)
# 2. write the release notes for it
$EDITOR android/play/metadata/android/en-US/changelogs/<versionCode>.txt
# 3. build, check, upload as a draft on the internal track
android/scripts/build.sh aab
android/scripts/publish.sh --track internal      # runs preflight.sh first; add --dry-run to just see the plan
#    --listing also syncs the store text/graphics/screenshots from play/metadata
```

The upload is a **draft** release unless you pass `--status completed`: review
and roll it out in Play Console. Tagging `v<version>` on GitHub does the same
in CI (`.github/workflows/android.yml`; secrets are listed at its top).

## 5. Promote through the tracks

Internal testing → (Closed testing) → Production, in Play Console
(**Promote release**), or upload straight to a track with `--track`.

## 6. Production access for new personal accounts

Personal accounts created after 13 Nov 2023 must run a **closed test with at
least 12 testers opted in continuously for 14 days** before they can apply for
production access (Play Console → **Dashboard → Apply for production**).
Organisation accounts and older personal accounts are exempt. Plan for it: set
up a closed-testing track and recruit testers early — testers must actually use
the app.

## What Google requires that the build already handles

| Requirement | How |
|---|---|
| Android App Bundle | `scripts/build.sh aab` |
| Target API 36 (new uploads since 2026-08-31) | `targetSdk = 36` in `app/build.gradle.kts`; checked by `preflight.sh` |
| 64-bit native code | `arm64-v8a` and `x86_64` are built; checked by `preflight.sh` |
| 16 KB page size support | NDK r29 aligns native libraries to 16 KB; checked by `preflight.sh`; tests run on an Android 16 emulator image with 16 KB pages |
| Signed with an upload key | `signing/` + Gradle `signingConfigs.release` |
| Privacy policy URL, Data safety form | [`privacy-policy.md`](privacy-policy.md), [`play-console-answers.md`](play-console-answers.md) |
| No dangerous permissions | only `INTERNET` (for "Open URL"); files go through the system picker |
