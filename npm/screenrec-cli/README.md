# @omega/screenrec-cli

Node.js wrapper around the Omega Focus Rust screen recorder binary. Install this package to get a ready-to-use `screenrec` CLI via `npm`.

## Installation

```bash
npm install -g @omega/screenrec-cli
screenrec --output ./captures/session --duration 120 --audio none
```

> You can omit the `record` subcommand. The wrapper assumes recording mode by default.

### Binary distribution

This package expects a prebuilt screen recorder binary for each supported platform:

| Platform            | Archive/Binary                               |
| ------------------- | --------------------------------------------- |
| macOS (arm64/x64)   | `prebuilt/darwin-<arch>/screenrec` or `*.tar.gz` |
| Windows (x64/arm64) | `prebuilt/windows-<arch>/screenrec.exe` or `*.zip` |

When publishing, populate the `prebuilt/` directory with the correct assets or host them on a CDN/GitHub release and set `SCREENREC_BINARY_BASE_URL` before `npm publish`.

During `npm install` the script will:

1. Look for a platform-specific binary under `prebuilt/<platform>-<arch>/`.
2. Look for an archive (`.tar.gz` or `.zip`) in `prebuilt/`.
3. Download from `SCREENREC_BINARY_BASE_URL` or the default GitHub releases URL.
4. Fallback to `SCREENREC_BINARY_PATH` if you explicitly supply a path.

## Troubleshooting

- Run `npm run doctor` to verify the binary and environment.
- Ensure Node.js ≥ 16.
- Provide `SCREENREC_BINARY_BASE_URL` or `SCREENREC_BINARY_PATH` if no binary was bundled.

## Development workflow

1. Build the Rust binary with `cargo build --release`.
2. Copy the binary into `npm/screenrec-cli/prebuilt/<platform>-<arch>/`.
3. Run `npm install` in this directory to fetch JS dependencies.
4. From `npm/screenrec-cli` run `npm publish` (after version bump).

See the root repository README for CLI usage instructions.
