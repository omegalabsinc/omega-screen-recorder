#!/usr/bin/env node
const { spawn } = require('child_process');
const path = require('path');
const fs = require('fs');

const binaryName = process.platform === 'win32' ? 'screenrec.exe' : 'screenrec';
const binaryPath = path.resolve(__dirname, '..', 'dist', binaryName);

if (!fs.existsSync(binaryPath)) {
  console.error(`screenrec binary not found at: ${binaryPath}`);
  console.error('Please reinstall the package or run `npm run doctor` for troubleshooting guidance.');
  process.exit(1);
}

const args = process.argv.slice(2);
const firstArg = args[0];

const knownCommands = new Set([
  'record',
  'screenshot',
  'help',
  '--help',
  '-h',
  'version',
  '--version',
  '-V',
]);

const finalArgs =
  args.length === 0 || firstArg?.startsWith('-') || !knownCommands.has(firstArg)
    ? ['record', ...args]
    : args;

const child = spawn(binaryPath, finalArgs, {
  stdio: 'inherit',
});

child.on('close', (code, signal) => {
  if (signal) {
    process.kill(process.pid, signal);
  } else {
    process.exit(code ?? 0);
  }
});

child.on('error', (err) => {
  console.error('Failed to start screenrec binary:', err);
  process.exit(1);
});
