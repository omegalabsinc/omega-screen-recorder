Place platform-specific prebuilt binaries or archives for the screenrec CLI here before publishing.

Expected naming:

- `prebuilt/darwin-arm64/screenrec`
- `prebuilt/darwin-x64/screenrec`
- `prebuilt/windows-x64/screenrec.exe`
- `prebuilt/windows-arm64/screenrec.exe`

Alternatively provide archives named `screenrec-<platform>-<arch>.tar.gz` (macOS) or `.zip` (Windows).

During installation the package copies or extracts the binary into `dist/`.
