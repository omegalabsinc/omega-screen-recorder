# Next Steps for Testing & Publishing `@omega/screenrec-cli`

This checklist helps you validate and ship the Node.js wrapper for the screen recorder. Follow the steps in order; mark them off as you go.

---

## 1. Prepare binaries

### 1.1 Build locally (recommended when you have the hardware)

- [ ] **macOS arm64** (Apple Silicon)  
  ```bash
  cargo build --release
  cp target/release/screenrec npm/screenrec-cli/prebuilt/darwin-arm64/screenrec
  codesign --force --deep --sign - npm/screenrec-cli/prebuilt/darwin-arm64/screenrec
  ```
- [ ] **macOS x64** (Intel or Rosetta VM) – same commands, copy to `prebuilt/darwin-x64/screenrec`
- [ ] **Windows x64** (MSVC toolchain)  
  ```powershell
  cargo build --release
  copy target\release\screenrec.exe npm\screenrec-cli\prebuilt\windows-x64\screenrec.exe
  ```
- [ ] Optional: **Windows arm64** – copy to `prebuilt/windows-arm64/screenrec.exe`
- [ ] Instead of raw binaries, you may place archives named `screenrec-<platform>-<arch>.tar.gz` (macOS) or `screenrec-<platform>-<arch>.zip` (Windows) inside `prebuilt/`
- [ ] After staging files, verify permissions (`chmod +x`) and hashes as needed

### 1.2 If you don’t have the target machines

- [ ] Set up a GitHub Actions workflow or other CI service with macOS and Windows runners
- [ ] Use the provided workflow `.github/workflows/build-binaries.yml` (runs on `workflow_dispatch` or tag pushes) or adapt it for your CI
- [ ] Generate release artifacts (binaries or archives) in CI and either:
  - Download them and drop into `prebuilt/<platform>-<arch>/`, **or**
  - Publish them to GitHub Releases/CDN and record the base URL
- [ ] When using hosted assets, export `SCREENREC_BINARY_BASE_URL=https://…/releases/download/vX.Y.Z` before `npm install`/`npm publish` so the installer fetches the right archive
- [ ] As a fallback, allow advanced users to supply their own binary via `SCREENREC_BINARY_PATH=/path/to/screenrec` during install
- [ ] Document the CI pipeline and artifact locations in your release notes/changelog

### 1.3 Retrieve CI artifacts

- [ ] After the workflow finishes, download artifacts from the Actions run (each archive + `.sha256`)
- [ ] Drop the archives into `npm/screenrec-cli/prebuilt/` for local testing **or** rely on the GitHub Release assets referenced by `SCREENREC_BINARY_BASE_URL`
- [ ] Keep checksum files alongside the archives to validate integrity during staging/publishing

## 2. Local install test

- [ ] From `npm/screenrec-cli`, install dependencies: `npm install`
- [ ] Link locally: `npm link`
- [ ] Verify binary resolution:
  - `screenrec --help`
  - `screenrec record --duration 5 --audio none --output npm-test`
- [ ] Run the doctor script: `npm run doctor`

## 3. Windows validation

- [ ] On a Windows machine, copy the package directory
- [ ] Ensure `prebuilt/windows-x64/screenrec.exe` exists (or provide archive/URL)
- [ ] Run `npm install` (PowerShell or CMD)
- [ ] Execute `npx screenrec --help`
- [ ] Run a short recording to confirm permissions/output

## 4. macOS validation

- [ ] On macOS arm64 host, run `npm install`
- [ ] Confirm Gatekeeper permissions (codesign or notarize if needed)
- [ ] Execute `npx screenrec record --duration 5 --audio none`
- [ ] Check that `screenrec record` produces an MP4
- [ ] Repeat on macOS x64 (if possible) or via Rosetta

## 5. Remote download path (optional)

If you plan to skip bundling binaries in the npm tarball:

- [ ] Upload archives to GitHub releases (tag `v0.1.0`, etc.)
- [ ] Export `SCREENREC_BINARY_BASE_URL` pointing to the release asset folder before `npm install`
- [ ] Test install using only the download path (remove local `prebuilt/` directory)
- [ ] Document the URL and environment variable in release notes

## 6. Documentation updates

- [ ] Update `npm/screenrec-cli/README.md` with checksums, supported versions, and any known issues
- [ ] Update root `README.md` with npm usage instructions (already added—review for accuracy)
- [ ] Add release notes or changelog entry describing npm package availability

## 7. Publish (once tests pass)

### Manual

- [ ] Bump version in `npm/screenrec-cli/package.json`
- [ ] Run `npm pack` to inspect the tarball contents
- [ ] Ensure `dist/screenrec` exists for each platform or that download URL is available
- [ ] Publish: `npm publish --access public`
- [ ] After publishing, test fresh install: `npm install -g @omega/screenrec-cli`

### Automated (GitHub Actions)

- [ ] Add `NPM_TOKEN` (publishable npm access token) as a repository secret
- [ ] Confirm release assets exist at `https://github.com/<repo>/releases/download/<tag>/screenrec-<platform>-<arch>.(tar.gz|zip)`
- [ ] Trigger `.github/workflows/publish-npm.yml` by publishing a Git tag (`vX.Y.Z`) or via “Run workflow” and supplying the tag
- [ ] Review the dry-run job output and final publish logs under Actions → “Publish npm Package”
- [ ] Run a fresh install from npm to verify the published package

## 8. Post-release checks

- [ ] Verify npm download stats periodically
- [ ] Respond to user reports (permissions, antivirus, etc.)
- [ ] Keep Rust binary version in sync with npm package version
- [ ] Automate building/uploading binaries for future releases (CI pipeline)

---

**Tips**
- Codesign macOS binaries to avoid Gatekeeper prompts (`codesign --force --deep --sign - screenrec`)
- For Windows, consider using `signtool` if distributing widely
- Provide fallback instructions (`SCREENREC_BINARY_PATH`) for users who compile locally
- Document platform-specific permissions (macOS Screen Recording, Windows capture permissions)

Keep this file updated with future release requirements or troubleshooting notes.
