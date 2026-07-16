import { describe, expect, it } from 'vitest';

import {
  MAX_BROWSER_FALLBACK_BYTES,
  MAX_BROWSER_SELECTED_FILES,
  MAX_JSON_NESTING,
} from '@cakesplitter/shared-types';

import { parseWorkerRequest, parseWorkerResponse, WorkerProtocolError } from './protocol';

const emptyManifest = JSON.stringify({
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
});
const emptyManifestValue: unknown = JSON.parse(emptyManifest) as unknown;

describe('Worker request validation', () => {
  it('accepts a well-formed split request', () => {
    const file = new File(['cake'], 'cake.bin');
    expect(parseWorkerRequest({ type: 'split', file, sliceSize: 2 })).toEqual({
      type: 'split',
      file,
      sliceSize: 2,
    });
  });

  it.each([
    ['unknown command', { type: 'launch' }],
    ['unexpected field', { type: 'cancel', surprise: true }],
    ['non-File input', { type: 'split', file: {}, sliceSize: 1 }],
    ['zero Slice size', { type: 'split', file: new File([], 'cake.bin'), sliceSize: 0 }],
    ['non-integer Slice size', { type: 'split', file: new File([], 'cake.bin'), sliceSize: 1.5 }],
    ['non-array selection', { type: 'inspect', manifestText: emptyManifest, files: {} }],
  ])('rejects %s', (_label, request) => {
    expect(() => parseWorkerRequest(request)).toThrow();
  });

  it('rejects malformed and over-nested manifests before dispatch', () => {
    expect(() =>
      parseWorkerRequest({ type: 'inspect', manifestText: '{nope', files: [] }),
    ).toThrow();
    const nested = `${'['.repeat(MAX_JSON_NESTING + 1)}0${']'.repeat(MAX_JSON_NESTING + 1)}`;
    expect(() =>
      parseWorkerRequest({ type: 'inspect', manifestText: nested, files: [] }),
    ).toThrow();
  });

  it('rejects an oversized selected-file array before iterating it', () => {
    const file = new File([], 'cake.bin');
    const files = Array<File>(MAX_BROWSER_SELECTED_FILES + 1).fill(file);
    expect(() => parseWorkerRequest({ type: 'inspect', manifestText: emptyManifest, files })).toThrow(
      /file count/u,
    );
  });

  it('rejects compatibility Merge outputs over the memory limit', () => {
    const size = MAX_BROWSER_FALLBACK_BYTES + 1;
    const base = JSON.parse(emptyManifest) as Record<string, unknown> & {
      original: Record<string, unknown>;
    };
    const oversized = JSON.stringify({
      ...base,
      original: {
        ...base.original,
        size,
      },
      targetSliceSize: size,
      sliceCount: 1,
      slices: [
        {
          index: 1,
          filename: 'empty.bin.001.slice',
          offset: 0,
          size,
          sha256: 'e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855',
        },
      ],
    });
    expect(() => parseWorkerRequest({ type: 'merge', manifestText: oversized, files: [] })).toThrow(
      /limited/u,
    );
  });
});

describe('Worker response validation', () => {
  it('accepts bounded progress and error messages', () => {
    expect(
      parseWorkerResponse({
        type: 'progress',
        operation: 'split',
        bytesProcessed: 1,
        totalBytes: 2,
        currentSlice: 1,
        sliceCount: 1,
        message: 'Working',
      }),
    ).toMatchObject({ type: 'progress', operation: 'split' });
    expect(
      parseWorkerResponse({ type: 'error', state: 'failed', code: 'bad', message: 'Rejected' }),
    ).toMatchObject({ type: 'error', state: 'failed' });
  });

  it.each([
    ['unknown response', { type: 'surprise' }],
    ['extra field', { type: 'error', state: 'failed', code: 'bad', message: 'x', extra: 1 }],
    [
      'invalid progress number',
      {
        type: 'progress',
        operation: 'split',
        bytesProcessed: Number.NaN,
        totalBytes: 2,
        currentSlice: 1,
        sliceCount: 1,
        message: 'Working',
      },
    ],
    [
      'out-of-bounds progress',
      {
        type: 'progress',
        operation: 'split',
        bytesProcessed: 3,
        totalBytes: 2,
        currentSlice: 1,
        sliceCount: 1,
        message: 'Working',
      },
    ],
    ['non-Blob download', { type: 'download', filename: 'cake.bin', blob: {} }],
    [
      'green inspection result without evidence',
      {
        type: 'result',
        operation: 'inspect',
        mode: 'read-only',
        message: 'Verified',
        inspection: {
          manifest: emptyManifestValue,
          foundSliceCount: 0,
          missing: ['missing.slice'],
          corrupted: [],
          duplicates: [],
          unexpected: [],
          verified: true,
          slices: [],
        },
      },
    ],
    [
      'inconsistent result fields',
      {
        type: 'result',
        operation: 'merge',
        mode: 'read-only',
        message: 'Done',
      },
    ],
  ])('rejects %s', (_label, response) => {
    expect(() => parseWorkerResponse(response)).toThrow(WorkerProtocolError);
  });
});
