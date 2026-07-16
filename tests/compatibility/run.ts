import { createHash, randomUUID } from 'node:crypto';
import { mkdtemp, mkdir, readFile, readdir, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { spawnSync } from 'node:child_process';

import {
  FORMAT_IDENTIFIER,
  FORMAT_VERSION,
  manifestFilename,
  parseManifest,
  planSlices,
  type CakeManifest,
  type SliceEntry,
} from '@cakesplitter/shared-types';

const repository = process.cwd();
const cargoTarget = path.join(tmpdir(), 'cakesplitter-cargo-target');
const temporary = await mkdtemp(path.join(tmpdir(), 'cakesplitter-compatibility-'));

try {
  await rustPackageRebuiltByWeb();
  await webPackageRebuiltByRust();
  await consistentInvalidManifestResults();
  console.log('Compatibility PASS: Rust package validated and rebuilt by Web contract.');
  console.log('Compatibility PASS: Web package verified and rebuilt by Rust CLI.');
  console.log('Compatibility PASS: malicious manifest fixtures rejected by both runtimes.');
} finally {
  await rm(temporary, { recursive: true, force: true });
}

async function rustPackageRebuiltByWeb() {
  const inputDirectory = path.join(temporary, 'rust-input');
  const packageDirectory = path.join(temporary, 'rust-package');
  await mkdir(inputDirectory);
  await mkdir(packageDirectory);
  const filename = '生日蛋糕.archive.bin';
  const original = Buffer.from('CakeSplitter cross-runtime fixture\n0123456789abcdef\n', 'utf8');
  const input = path.join(inputDirectory, filename);
  await writeFile(input, original);

  runCargo([
    'run',
    '--quiet',
    '-p',
    'cakesplitter-cli',
    '--',
    'split',
    input,
    '--slice-size',
    '7',
    '--output-dir',
    packageDirectory,
  ]);

  const manifestPath = path.join(packageDirectory, manifestFilename(filename));
  const manifest = parseManifest(await readFile(manifestPath, 'utf8'));
  const rebuilt: Buffer[] = [];
  for (const slice of manifest.slices) {
    const bytes = await readFile(path.join(packageDirectory, slice.filename));
    assertEqual(bytes.length, slice.size, `Rust Slice size ${slice.index}`);
    assertEqual(sha256(bytes), slice.sha256, `Rust Slice hash ${slice.index}`);
    rebuilt.push(bytes);
  }
  const webRebuilt = Buffer.concat(rebuilt);
  assertEqual(sha256(webRebuilt), manifest.original.sha256, 'Web rebuilt hash');
  assertBuffersEqual(webRebuilt, original, 'Web rebuilt bytes');
  const rustRebuiltPath = path.join(packageDirectory, 'rust-rebuilt.bin');
  runCargo([
    'run',
    '--quiet',
    '-p',
    'cakesplitter-cli',
    '--',
    'merge',
    manifestPath,
    '--output',
    rustRebuiltPath,
  ]);
  const rustRebuilt = await readFile(rustRebuiltPath);
  assertEqual(sha256(rustRebuilt), sha256(webRebuilt), 'Rust/Web hash for Rust package');
  assertBuffersEqual(rustRebuilt, webRebuilt, 'Rust/Web bytes for Rust package');
}

async function webPackageRebuiltByRust() {
  const packageDirectory = path.join(temporary, 'web-package');
  await mkdir(packageDirectory);
  const filename = 'web.generated.tar.bin';
  const original = Buffer.from('web-worker-compatible-bytes-0123456789', 'utf8');
  const plan = planSlices(filename, original.length, 6);
  const slices: SliceEntry[] = [];
  const webChunks: Buffer[] = [];
  for (const entry of plan) {
    const bytes = original.subarray(entry.offset, entry.offset + entry.size);
    await writeFile(path.join(packageDirectory, entry.filename), bytes);
    webChunks.push(bytes);
    slices.push({ ...entry, sha256: sha256(bytes) });
  }
  const manifest: CakeManifest = {
    format: FORMAT_IDENTIFIER,
    version: FORMAT_VERSION,
    packageId: randomUUID(),
    createdAt: new Date('2026-07-16T04:00:00Z').toISOString(),
    original: {
      filename,
      size: original.length,
      sha256: sha256(original),
    },
    targetSliceSize: 6,
    sliceCount: slices.length,
    slices,
  };
  parseManifest(JSON.stringify(manifest));
  const manifestPath = path.join(packageDirectory, manifestFilename(filename));
  await writeFile(manifestPath, `${JSON.stringify(manifest, null, 2)}\n`);

  runCargo([
    'run',
    '--quiet',
    '-p',
    'cakesplitter-cli',
    '--',
    'verify',
    manifestPath,
  ]);
  const rebuiltPath = path.join(packageDirectory, 'rust-rebuilt.bin');
  runCargo([
    'run',
    '--quiet',
    '-p',
    'cakesplitter-cli',
    '--',
    'merge',
    manifestPath,
    '--output',
    rebuiltPath,
  ]);
  const rustRebuilt = await readFile(rebuiltPath);
  const webRebuilt = Buffer.concat(webChunks);
  assertEqual(sha256(rustRebuilt), sha256(webRebuilt), 'Rust/Web hash for Web package');
  assertBuffersEqual(rustRebuilt, webRebuilt, 'Rust/Web bytes for Web package');
  assertBuffersEqual(rustRebuilt, original, 'Rust rebuilt Web package');
}

async function consistentInvalidManifestResults() {
  const fixturesDirectory = path.join(repository, 'tests', 'fixtures', 'invalid');
  const fixtures = (await readdir(fixturesDirectory)).filter((name) => name.endsWith('.cake.json'));
  assertEqual(fixtures.length >= 3, true, 'Invalid fixture coverage');
  for (const fixture of fixtures) {
    const fixturePath = path.join(fixturesDirectory, fixture);
    let webRejected = false;
    try {
      parseManifest(await readFile(fixturePath, 'utf8'));
    } catch {
      webRejected = true;
    }
    assertEqual(webRejected, true, `Web rejects ${fixture}`);
    const rust = runCargo(
      [
        'run',
        '--quiet',
        '-p',
        'cakesplitter-cli',
        '--',
        'inspect',
        fixturePath,
      ],
      true,
    );
    assertEqual(rust.status === 0, false, `Rust rejects ${fixture}`);
  }
}

function runCargo(arguments_: string[], allowFailure = false) {
  const result = spawnSync('cargo', arguments_, {
    cwd: repository,
    encoding: 'utf8',
    env: { ...process.env, CARGO_TARGET_DIR: cargoTarget },
    shell: process.platform === 'win32',
  });
  if (!allowFailure && result.status !== 0) {
    throw new Error(
      `cargo ${arguments_.join(' ')} failed (${result.status ?? 'no status'})\n${result.stdout}\n${result.stderr}`,
    );
  }
  return result;
}

function sha256(bytes: Uint8Array) {
  return createHash('sha256').update(bytes).digest('hex');
}

function assertEqual(actual: unknown, expected: unknown, label: string) {
  if (actual !== expected) {
    throw new Error(`${label}: expected ${String(expected)}, found ${String(actual)}`);
  }
}

function assertBuffersEqual(actual: Uint8Array, expected: Uint8Array, label: string) {
  if (!Buffer.from(actual).equals(Buffer.from(expected))) {
    throw new Error(`${label}: byte sequences differ`);
  }
}
