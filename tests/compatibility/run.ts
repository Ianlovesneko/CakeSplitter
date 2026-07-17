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
const compatibilityCases = [
  { filename: 'empty.bin', original: Buffer.alloc(0), sliceSize: 4 },
  { filename: 'one-byte.bin', original: Buffer.from([0x7f]), sliceSize: 4 },
  { filename: 'exact-boundary.bin', original: Buffer.from('abcdefgh'), sliceSize: 4 },
  { filename: 'final-short.bin', original: Buffer.from('abcdefghi'), sliceSize: 4 },
  {
    filename: '生日蛋糕.archive.tar.bin',
    original: Buffer.from('CakeSplitter cross-runtime fixture\n0123456789abcdef\n', 'utf8'),
    sliceSize: 7,
  },
] as const;

try {
  await rustPackageRebuiltByWeb();
  await webPackageRebuiltByRust();
  await consistentInvalidManifestResults();
  console.log('Compatibility PASS: Rust boundary packages inspected and rebuilt by Web contract.');
  console.log('Compatibility PASS: Web boundary packages inspected, verified, and rebuilt by Rust CLI.');
  console.log('Compatibility PASS: malicious manifest fixtures rejected by both runtimes.');
} finally {
  await rm(temporary, { recursive: true, force: true });
}

async function rustPackageRebuiltByWeb() {
  for (const [caseIndex, fixture] of compatibilityCases.entries()) {
    const inputDirectory = path.join(temporary, `rust-input-${caseIndex}`);
    const packageDirectory = path.join(temporary, `rust-package-${caseIndex}`);
    await mkdir(inputDirectory);
    await mkdir(packageDirectory);
    const input = path.join(inputDirectory, fixture.filename);
    await writeFile(input, fixture.original);

    runCargo([
      'run', '--quiet', '-p', 'cakesplitter-cli', '--', 'split', input,
      '--slice-size', String(fixture.sliceSize), '--output-dir', packageDirectory,
    ]);

    const manifestPath = path.join(packageDirectory, manifestFilename(fixture.filename));
    runCargo(['run', '--quiet', '-p', 'cakesplitter-cli', '--', 'inspect', manifestPath]);
    runCargo(['run', '--quiet', '-p', 'cakesplitter-cli', '--', 'verify', manifestPath]);
    const manifest = parseManifest(await readFile(manifestPath, 'utf8'));
    const rebuilt: Buffer[] = [];
    for (const slice of manifest.slices) {
      const bytes = await readFile(path.join(packageDirectory, slice.filename));
      assertEqual(bytes.length, slice.size, `Rust Slice size ${fixture.filename} ${slice.index}`);
      assertEqual(sha256(bytes), slice.sha256, `Rust Slice hash ${fixture.filename} ${slice.index}`);
      rebuilt.push(bytes);
    }
    const webRebuilt = Buffer.concat(rebuilt);
    assertEqual(webRebuilt.length, fixture.original.length, `Web rebuilt size ${fixture.filename}`);
    assertEqual(sha256(webRebuilt), manifest.original.sha256, `Web rebuilt hash ${fixture.filename}`);
    assertBuffersEqual(webRebuilt, fixture.original, `Web rebuilt bytes ${fixture.filename}`);
    const rustRebuiltPath = path.join(packageDirectory, `rust-rebuilt-${caseIndex}.bin`);
    runCargo([
      'run', '--quiet', '-p', 'cakesplitter-cli', '--', 'merge', manifestPath,
      '--output', rustRebuiltPath,
    ]);
    const rustRebuilt = await readFile(rustRebuiltPath);
    assertEqual(sha256(rustRebuilt), sha256(webRebuilt), `Rust/Web hash ${fixture.filename}`);
    assertBuffersEqual(rustRebuilt, webRebuilt, `Rust/Web bytes ${fixture.filename}`);
  }
}

async function webPackageRebuiltByRust() {
  for (const [caseIndex, fixture] of compatibilityCases.entries()) {
    const packageDirectory = path.join(temporary, `web-package-${caseIndex}`);
    await mkdir(packageDirectory);
    const plan = planSlices(fixture.filename, fixture.original.length, fixture.sliceSize);
    const slices: SliceEntry[] = [];
    const webChunks: Buffer[] = [];
    for (const entry of plan) {
      const bytes = fixture.original.subarray(entry.offset, entry.offset + entry.size);
      await writeFile(path.join(packageDirectory, entry.filename), bytes);
      webChunks.push(bytes);
      slices.push({ ...entry, sha256: sha256(bytes) });
    }
    const manifest: CakeManifest = {
      format: FORMAT_IDENTIFIER,
      version: FORMAT_VERSION,
      packageId: randomUUID(),
      createdAt: new Date('2026-07-17T12:00:00Z').toISOString(),
      original: {
        filename: fixture.filename,
        size: fixture.original.length,
        sha256: sha256(fixture.original),
      },
      targetSliceSize: fixture.sliceSize,
      sliceCount: slices.length,
      slices,
    };
    parseManifest(JSON.stringify(manifest));
    const manifestPath = path.join(packageDirectory, manifestFilename(fixture.filename));
    await writeFile(manifestPath, `${JSON.stringify(manifest, null, 2)}\n`);

    runCargo(['run', '--quiet', '-p', 'cakesplitter-cli', '--', 'inspect', manifestPath]);
    runCargo(['run', '--quiet', '-p', 'cakesplitter-cli', '--', 'verify', manifestPath]);
    const rebuiltPath = path.join(packageDirectory, `rust-rebuilt-${caseIndex}.bin`);
    runCargo([
      'run', '--quiet', '-p', 'cakesplitter-cli', '--', 'merge', manifestPath,
      '--output', rebuiltPath,
    ]);
    const rustRebuilt = await readFile(rebuiltPath);
    const webRebuilt = Buffer.concat(webChunks);
    assertEqual(rustRebuilt.length, fixture.original.length, `Rust rebuilt size ${fixture.filename}`);
    assertEqual(sha256(rustRebuilt), sha256(webRebuilt), `Rust/Web hash ${fixture.filename}`);
    assertBuffersEqual(rustRebuilt, webRebuilt, `Rust/Web bytes ${fixture.filename}`);
    assertBuffersEqual(rustRebuilt, fixture.original, `Rust rebuilt Web package ${fixture.filename}`);

    if (fixture.filename === 'final-short.bin' && manifest.slices.length > 1) {
      const last = manifest.slices.at(-1)!;
      const lastPath = path.join(packageDirectory, last.filename);
      const originalLast = await readFile(lastPath);
      await rm(lastPath);
      assertEqual(
        runCargo(['run', '--quiet', '-p', 'cakesplitter-cli', '--', 'verify', manifestPath], true).status === 0,
        false,
        'Rust rejects a missing Web Slice',
      );
      await writeFile(lastPath, Buffer.alloc(originalLast.length, 0xff));
      assertEqual(
        runCargo(['run', '--quiet', '-p', 'cakesplitter-cli', '--', 'verify', manifestPath], true).status === 0,
        false,
        'Rust rejects a modified Web Slice',
      );
    }
  }
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
