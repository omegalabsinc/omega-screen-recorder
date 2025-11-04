#!/usr/bin/env node
const path = require('path');
const fs = require('fs');
const { spawnSync } = require('child_process');

const binaryName = process.platform === 'win32' ? 'screenrec.exe' : 'screenrec';
const binaryPath = path.resolve(__dirname, '..', 'dist', binaryName);

console.log('screenrec doctor\n===============');
console.log(`Platform: ${process.platform}`);
console.log(`Arch:     ${process.arch}`);
console.log(`Binary:   ${binaryPath}`);

if (fs.existsSync(binaryPath)) {
  console.log('✅ Binary found.');
  const result = spawnSync(binaryPath, ['--help'], { stdio: 'inherit' });
  process.exit(result.status ?? 0);
} else {
  console.log('❌ Binary not found.');
  console.log('');
  console.log('Troubleshooting steps:');
  console.log('  • Ensure a prebuilt binary exists under prebuilt/<platform>-<arch>/');
  console.log('  • Re-run installation with SCREENREC_BINARY_PATH set to your compiled binary.');
  console.log('  • Set SCREENREC_BINARY_BASE_URL to a release URL and reinstall.');
  process.exit(1);
}
