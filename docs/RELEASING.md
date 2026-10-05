# Releasing Suneiro

Pushing a `v*` tag runs `.github/workflows/release.yml`, which builds a universal macOS app, signs it with the Developer ID certificate, notarizes it with Apple and publishes a GitHub release with:

- `Suneiro_<version>_universal.dmg`: the installer people download
- `Suneiro.dmg`: the same installer under a fixed name, so `https://github.com/steebchen/suneiro/releases/latest/download/Suneiro.dmg` always serves the newest version (the website's download button links there)
- `Suneiro.app.tar.gz` + `.sig`: the update payload and its signature
- `latest.json`: what installed apps poll

## Cutting a release

```sh
pnpm bump 0.2.0
git commit -am "Release v0.2.0" && git tag v0.2.0 && git push origin HEAD v0.2.0
```

The workflow fails early if the tag and the app version differ.

## How auto-update works

`apps/desktop/src-tauri/src/updater.rs` uses `tauri-plugin-updater`. Release builds fetch the endpoint in `tauri.conf.json` (`plugins.updater.endpoints`) 10 seconds after launch and every 4 hours. If `latest.json` has a newer version, the app downloads the archive, checks its signature against `plugins.updater.pubkey`, and replaces the `.app` on disk. Nothing is asked of the user. The running app is never restarted on its own, because agents may be mid-turn: the sidebar shows **Restart to update**, and otherwise the new version starts with the next launch. **Suneiro → Check for Updates…** runs the same check on demand. Development builds never update.

The endpoint must be readable without credentials. `https://github.com/<owner>/<repo>/releases/latest/download/latest.json` only works for a public repository; for a private one, publish releases to a public repository or a bucket and change the endpoint.

`createUpdaterArtifacts` lives in `tauri.release.conf.json` (passed by the workflow), so a local `pnpm build` works without the signing key.

## One-time setup

GitHub repository secrets (Settings → Secrets and variables → Actions):

| Secret | What it is |
| --- | --- |
| `TAURI_SIGNING_PRIVATE_KEY` | Contents of the updater private key (`pnpm --filter desktop tauri signer generate`). Its public half is `plugins.updater.pubkey`. |
| `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` | Password of that key; leave unset if it has none. |
| `APPLE_CERTIFICATE` | Base64 of the **Developer ID Application** certificate exported as `.p12` (`base64 -i cert.p12`). |
| `APPLE_CERTIFICATE_PASSWORD` | Password chosen when exporting the `.p12`. |
| `APPLE_SIGNING_IDENTITY` | `Developer ID Application: <Name> (<TEAMID>)`, as shown by `security find-identity -v -p codesigning`. |
| `APPLE_API_KEY` | Key ID of an App Store Connect API key (for notarization). |
| `APPLE_API_ISSUER` | Issuer ID shown above the key list. |
| `APPLE_API_KEY_P8` | Contents of the downloaded `AuthKey_<KEYID>.p8`. |

The updater key is permanent: installed apps only accept updates signed with it, so losing it means users must reinstall by hand. Keep a backup outside the repository.

Getting the Apple pieces (needs the Account Holder or an Admin of a paid Apple Developer Program team):

1. **Certificate:** Keychain Access → Certificate Assistant → Request a Certificate From a Certificate Authority → save to disk. At <https://developer.apple.com/account/resources/certificates/add> choose **Developer ID Application**, upload the request, download the `.cer` and double-click it. In Keychain Access → My Certificates, right-click it → Export → `.p12`.
2. **Notarization key:** <https://appstoreconnect.apple.com/access/integrations/api> → Team Keys → generate a key with the **Developer** role and download the `.p8` (only possible once).

## Checking a build

```sh
codesign -dv --verbose=4 /Applications/Suneiro.app   # Authority=Developer ID Application, flags=runtime
spctl -a -vv /Applications/Suneiro.app               # accepted, source=Notarized Developer ID
```
