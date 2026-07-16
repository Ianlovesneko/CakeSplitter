import { describe, expect, it } from 'vitest';

import {
  IncrementalSha256,
  MAX_FILENAME_BYTES,
  MAX_JSON_NESTING,
  MAX_MANIFEST_BYTES,
  MAX_SLICE_COUNT,
  ManifestValidationError,
  parseManifest,
  planSlices,
  validateManifest,
  validatePortableFilename,
} from '../src/index';

const emptyManifest = {
  format: 'cakesplitter',
  version: '1.0',
  packageId: 'ff7cb026-f7ec-4d17-a3e4-8083217ec688',
  createdAt: '2026-07-16T04:00:00Z',
  original: {
    filename: 'empty.bin',
    size: 0,
    sha256: 'e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855',
  },
  targetSliceSize: 1024,
  sliceCount: 0,
  slices: [],
};

describe('manifest validation', () => {
  it('accepts an empty Cake', () => {
    expect(validateManifest(emptyManifest).sliceCount).toBe(0);
  });

  it.each([
    ['path traversal', { ...emptyManifest, original: { ...emptyManifest.original, filename: '../x' } }],
    ['unsupported version', { ...emptyManifest, version: '2.0' }],
    ['malformed hash', { ...emptyManifest, original: { ...emptyManifest.original, sha256: 'bad' } }],
    ['unknown field', { ...emptyManifest, absolutePath: 'C:\\secret' }],
  ])('rejects %s', (_label, value) => {
    expect(() => validateManifest(value)).toThrow(ManifestValidationError);
  });

  it('rejects malformed JSON', () => {
    expect(() => parseManifest('{nope')).toThrow(ManifestValidationError);
  });

  it('rejects manifests above the byte and nesting limits', () => {
    expect(() => parseManifest(' '.repeat(MAX_MANIFEST_BYTES + 1))).toThrow(/byte limit/u);
    const nested = `${'['.repeat(MAX_JSON_NESTING + 1)}0${']'.repeat(MAX_JSON_NESTING + 1)}`;
    expect(() => parseManifest(nested)).toThrow(/nesting/u);
  });

  it('rejects slice tables above the explicit count limit', () => {
    expect(() =>
      validateManifest({
        ...emptyManifest,
        sliceCount: MAX_SLICE_COUNT + 1,
      }),
    ).toThrow(/supported maximum/u);
  });

  it.each([
    'CON',
    'con.txt',
    'COM1.bin',
    'LPT9',
    'COM¹.log',
    'bad<name.bin',
    'bad|name.bin',
    ' leading.bin',
    'trailing\u{2003}',
  ])('rejects non-portable filename %j', (filename) => {
    expect(() => validatePortableFilename(filename)).toThrow(ManifestValidationError);
  });

  it('enforces filename bytes and keeps ordinary portable names', () => {
    expect(() => validatePortableFilename('a'.repeat(MAX_FILENAME_BYTES + 1))).toThrow(/UTF-8 bytes/u);
    for (const filename of ['console.bin', 'COM0.bin', 'LPT10.bin', '生日蛋糕.bin']) {
      expect(() => validatePortableFilename(filename)).not.toThrow();
    }
  });
});

describe('slice planning', () => {
  it('uses a minimum three digit width and preserves extensions', () => {
    expect(planSlices('archive.tar.bin', 5, 3)).toEqual([
      { index: 1, filename: 'archive.tar.bin.001.slice', offset: 0, size: 3 },
      { index: 2, filename: 'archive.tar.bin.002.slice', offset: 3, size: 2 },
    ]);
  });

  it('rejects plans above the supported Slice count before allocation', () => {
    expect(() => planSlices('many.bin', MAX_SLICE_COUNT + 1, 1)).toThrow(/supported maximum/u);
  });
});

describe('incremental SHA-256', () => {
  it.each([
    ['', 'e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855'],
    ['abc', 'ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad'],
    [
      'The quick brown fox jumps over the lazy dog',
      'd7a8fbb307d7809469ca9abcb0082e4f8d5651e46d3cdb762d02d0bf37c9e592',
    ],
  ])('hashes %j', (input, expected) => {
    const bytes = new TextEncoder().encode(input);
    const state = new IncrementalSha256();
    state.update(bytes.subarray(0, 2));
    state.update(bytes.subarray(2));
    expect(state.digestHex()).toBe(expected);
  });
});
