#!/usr/bin/env node
const fs = require('fs');
const path = require('path');
const os = require('os');
const { pipeline } = require('stream/promises');
const https = require('https');
const tar = require('tar');
const extractZip = require('extract-zip');

const pkg = require('../package.json');

const PLATFORM_ALIASES = {
  darwin: 'darwin',
  win32: 'windows',
};

const ARCH_ALIASES = {
  x64: 'x64',
  arm64: 'arm64',
};

const platform = PLATFORM_ALIASES[process.platform];
const arch = ARCH_ALIASES[process.arch];
const binaryName = process.platform === 'win32' ? 'screenrec.exe' : 'screenrec';
const destDir = path.resolve(__dirname, '..', 'dist');
const destPath = path.join(destDir, binaryName);

(async () => {
  try {
    await fs.promises.mkdir(destDir, { recursive: true });

    if (!platform || !arch) {
      throw new Error(`Unsupported platform or architecture: ${process.platform}-${process.arch}`);
    }

    if (await fileExists(destPath)) {
      return;
    }

    const overridePath = process.env.SCREENREC_BINARY_PATH;
    if (overridePath) {
      await copyBinary(overridePath, destPath);
      return;
    }

    const localBinary = path.resolve(__dirname, '..', 'prebuilt', `${platform}-${arch}`, binaryName);
    if (await fileExists(localBinary)) {
      await copyBinary(localBinary, destPath);
      return;
    }

    const archiveExt = process.platform === 'win32' ? 'zip' : 'tar.gz';
    const localArchive = path.resolve(
      __dirname,
      '..',
      'prebuilt',
      `${platform}-${arch}.${archiveExt}`,
    );
    if (await fileExists(localArchive)) {
      await extractArchive(localArchive, destDir);
      await ensureBinaryExists(destPath);
      return;
    }

    const baseUrl =
      process.env.SCREENREC_BINARY_BASE_URL ||
      `https://github.com/OmegaLabs/omega-screen-recorder-1/releases/download/v${pkg.version}`;
    const archiveName = `screenrec-${platform}-${arch}.${archiveExt}`;
    const downloadUrl = `${baseUrl}/${archiveName}`;
    console.log(`Downloading prebuilt screenrec binary (${platform}-${arch}) from ${downloadUrl}`);

    await downloadAndExtract(downloadUrl, archiveExt, destDir);
    await ensureBinaryExists(destPath);
  } catch (err) {
    console.error('Failed to set up screenrec binary.');
    console.error(err.message || err);
    console.error('');
    console.error('Troubleshooting steps:');
    console.error('  1. Ensure a prebuilt binary exists under prebuilt/<platform>-<arch>/screenrec');
    console.error('  2. Provide SCREENREC_BINARY_PATH pointing to a local screenrec binary');
    console.error('  3. Publish release assets and set SCREENREC_BINARY_BASE_URL to their location');
    process.exit(1);
  }
})();

async function fileExists(filePath) {
  try {
    await fs.promises.access(filePath, fs.constants.F_OK);
    return true;
  } catch {
    return false;
  }
}

async function copyBinary(src, dest) {
  await fs.promises.copyFile(src, dest);
  if (process.platform !== 'win32') {
    await fs.promises.chmod(dest, 0o755);
  }
  console.log(`screenrec binary installed from ${src}`);
}

async function extractArchive(archivePath, outDir) {
  await fs.promises.mkdir(outDir, { recursive: true });
  if (archivePath.endsWith('.zip')) {
    await extractZip(archivePath, { dir: outDir });
  } else {
    await tar.x({
      file: archivePath,
      cwd: outDir,
    });
  }
  console.log(`Extracted ${archivePath}`);
}

async function downloadAndExtract(url, ext, outDir) {
  const tmpDir = await fs.promises.mkdtemp(path.join(os.tmpdir(), 'screenrec-'));
  const archivePath = path.join(tmpDir, `archive.${ext}`);
  await downloadFile(url, archivePath);
  await extractArchive(archivePath, outDir);
  await fs.promises.rm(tmpDir, { recursive: true, force: true });
}

async function downloadFile(url, dest) {
  await new Promise((resolve, reject) => {
    const request = https.get(url, (response) => {
      if (response.statusCode && response.statusCode >= 400) {
        reject(new Error(`Download failed with status ${response.statusCode}`));
        return;
      }
      const fileStream = fs.createWriteStream(dest);
      pipeline(response, fileStream)
        .then(resolve)
        .catch(reject);
    });
    request.on('error', reject);
  });
}

async function ensureBinaryExists(binaryPath) {
  if (!(await fileExists(binaryPath))) {
    throw new Error(
      `Expected screenrec binary at ${binaryPath}, but it was not found after extraction.`,
    );
  }
  if (process.platform !== 'win32') {
    await fs.promises.chmod(binaryPath, 0o755);
  }
}
