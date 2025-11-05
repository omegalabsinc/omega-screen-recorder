# Ready-to-Publish Checklist — `@omega/screenrec-cli`

Tick each item as you go. After every section is checked off, you can confidently publish the package and direct users to install it.

---

## ✅ 1. Assets & Binaries
- [ ] Windows x64 binary present (`npm/screenrec-cli/prebuilt/windows-x64/screenrec.exe` or `screenrec-windows-x64.zip`)
- [ ] Optional: Windows arm64 binary present (if supported)
- [ ] Optional (future): macOS arm64/x64 binaries staged or ready in CI
- [ ] Checksums generated for every archive (`*.sha256`)
- [ ] Binaries signed (macOS `codesign`, Windows `signtool`) or decision documented

- [ ] macOS signing plan documented for v2 (if binaries not yet shipped)

## ✅ 2. GitHub Actions Outputs
- [ ] `Build Screenrec Binaries` workflow ran for the target tag
- [ ] Artifacts downloaded and verified locally **or** release assets uploaded (standard naming `screenrec-<platform>-<arch>.(tar.gz|zip)`)
- [ ] `SCREENREC_BINARY_BASE_URL` ready (e.g., `https://github.com/<org>/<repo>/releases/download/vX.Y.Z`)

## ✅ 3. npm Package Prep
- [ ] Version bumped in `npm/screenrec-cli/package.json` (matches release tag)
- [ ] `npm install` (or `npm ci`) succeeds locally
- [ ] `npm run doctor` verifies binary resolution on macOS
- [ ] Windows smoke test performed (help command + short recording)
- [ ] macOS smoke test performed (arm64 +, if possible, x64/Rosetta)
- [ ] `npm pack` inspected (tarball contains `dist/screenrec`/expected files)

## ✅ 4. Documentation & Comms
- [ ] `npm/screenrec-cli/README.md` updated with current version, checksum table, usage examples
- [ ] Root `README.md` includes npm install instructions
- [ ] Release notes/changelog drafted (features, known issues, env vars)
- [ ] Support/FAQ notes ready (permissions, antivirus, idle-frame behavior)

## ✅ 5. Credentials & Automation
- [ ] `NPM_TOKEN` secret added to repository
- [ ] Optional: GitHub release created (tag `vX.Y.Z`, description, assets attached)
- [ ] `SCREENREC_BINARY_BASE_URL` confirmed (set in workflow or documented)
- [ ] CI publish workflow (`Publish npm Package`) reviewed, dry-run understood

## ✅ 6. Publish Steps
- [ ] Manual: `npm publish --access public` (if not using automation)
- [ ] Automated: Trigger “Publish npm Package” workflow with target tag
- [ ] Monitor workflow logs (dry-run + final publish)

## ✅ 7. Post-Publish Validation
- [ ] Fresh install test: `npm install -g @omega/screenrec-cli`
- [ ] Default command works: `screenrec --duration 5 --audio none --output ./smoke`
- [ ] Interactions file generated when using `--track-interactions`
- [ ] Permissions prompts documented (macOS screen recording, Windows accessibility)
- [ ] npm package page verified (README renders, metadata correct)

## ✅ 8. Release Announcement
- [ ] Blog/announcement draft ready
- [ ] Social posts or community messages queued (Discord, Twitter/X, etc.)
- [ ] Support channel monitored for early user feedback

---

When every checkbox above is done, the npm package is production-ready. Keep this file updated for future releases, and tag the commit once you’re satisfied. Good luck! 💪🚀
