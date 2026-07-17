import { describe, expect, it } from 'vitest';

import {
  MAX_BROWSER_FALLBACK_BYTES,
  MAX_BROWSER_SELECTED_FILES,
  MAX_JSON_NESTING,
} from '@cakesplitter/shared-types';

import { parseWorkerRequest, parseWorkerResponse, WorkerProtocolError } from './protocol';

const identity = { requestId: 'request-12345678', taskId: 'task-12345678' };
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
  it('accepts identified start and control requests', () => {
    const file = new File(['cake'], 'cake.bin');
    expect(
      parseWorkerRequest({
        ...identity,
        type: 'start',
        operation: 'split',
        file,
        sliceSize: 2,
        outputMode: 'fallback',
      }),
    ).toMatchObject({ ...identity, type: 'start', operation: 'split', outputMode: 'fallback' });
    expect(
      parseWorkerRequest({ ...identity, type: 'control', command: 'cancel' }),
    ).toEqual({ ...identity, type: 'control', command: 'cancel' });
  });

  it('accepts a structural direct-folder handle only with direct mode', () => {
    const directory = directoryHandle();
    expect(
      parseWorkerRequest({
        ...identity,
        type: 'start',
        operation: 'split',
        file: new File(['cake'], 'cake.bin'),
        sliceSize: 2,
        outputMode: 'direct',
        directory,
      }),
    ).toMatchObject({ outputMode: 'direct', directory });
    expect(() =>
      parseWorkerRequest({
        ...identity,
        type: 'start',
        operation: 'split',
        file: new File(['cake'], 'cake.bin'),
        sliceSize: 2,
        outputMode: 'fallback',
        directory,
      }),
    ).toThrow(/must not receive/u);
  });

  it.each([
    ['unknown command', { ...identity, type: 'launch' }],
    ['missing identity', { type: 'control', command: 'cancel' }],
    ['invalid identity', { requestId: '../bad', taskId: 'task', type: 'control', command: 'cancel' }],
    ['unknown control', { ...identity, type: 'control', command: 'restart' }],
    ['unexpected field', { ...identity, type: 'control', command: 'cancel', surprise: true }],
    [
      'non-File input',
      { ...identity, type: 'start', operation: 'split', file: {}, sliceSize: 1, outputMode: 'fallback' },
    ],
    [
      'zero Slice size',
      {
        ...identity,
        type: 'start',
        operation: 'split',
        file: new File([], 'cake.bin'),
        sliceSize: 0,
        outputMode: 'fallback',
      },
    ],
    [
      'direct mode without directory',
      {
        ...identity,
        type: 'start',
        operation: 'split',
        file: new File([], 'cake.bin'),
        sliceSize: 1,
        outputMode: 'direct',
      },
    ],
    [
      'non-array selection',
      { ...identity, type: 'start', operation: 'inspect', manifestText: emptyManifest, files: {} },
    ],
  ])('rejects %s', (_label, request) => {
    expect(() => parseWorkerRequest(request)).toThrow();
  });

  it('rejects malformed and over-nested manifests before dispatch', () => {
    expect(() =>
      parseWorkerRequest({
        ...identity,
        type: 'start',
        operation: 'inspect',
        manifestText: '{nope',
        files: [],
      }),
    ).toThrow();
    const nested = `${'['.repeat(MAX_JSON_NESTING + 1)}0${']'.repeat(MAX_JSON_NESTING + 1)}`;
    expect(() =>
      parseWorkerRequest({
        ...identity,
        type: 'start',
        operation: 'inspect',
        manifestText: nested,
        files: [],
      }),
    ).toThrow();
  });

  it('rejects oversized selected arrays and fallback Merge output', () => {
    const file = new File([], 'cake.bin');
    const files = Array<File>(MAX_BROWSER_SELECTED_FILES + 1).fill(file);
    expect(() =>
      parseWorkerRequest({
        ...identity,
        type: 'start',
        operation: 'inspect',
        manifestText: emptyManifest,
        files,
      }),
    ).toThrow(/file count/u);

    const size = MAX_BROWSER_FALLBACK_BYTES + 1;
    const base = JSON.parse(emptyManifest) as Record<string, unknown> & {
      original: Record<string, unknown>;
    };
    const oversized = JSON.stringify({
      ...base,
      original: { ...base.original, size },
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
    expect(() =>
      parseWorkerRequest({
        ...identity,
        type: 'start',
        operation: 'merge',
        manifestText: oversized,
        files: [],
        outputMode: 'fallback',
      }),
    ).toThrow(/limited/u);
  });
});

describe('Worker response validation', () => {
  it('accepts identified bounded progress, state, and error messages', () => {
    expect(
      parseWorkerResponse({
        ...identity,
        type: 'progress',
        operation: 'split',
        status: 'running',
        bytesProcessed: 1,
        totalBytes: 2,
        currentSlice: 1,
        sliceCount: 1,
        speedBytesPerSecond: 2,
        message: 'Working',
      }),
    ).toMatchObject({ type: 'progress', operation: 'split' });
    expect(
      parseWorkerResponse({
        ...identity,
        type: 'state',
        operation: 'split',
        status: 'paused',
        message: 'Paused safely',
      }),
    ).toMatchObject({ type: 'state', status: 'paused' });
    expect(
      parseWorkerResponse({
        ...identity,
        type: 'error',
        operation: 'merge',
        status: 'permission-required',
        code: 'permission_revoked',
        message: 'Reselect the output directory.',
      }),
    ).toMatchObject({ type: 'error', status: 'permission-required' });
  });

  it('accepts completed and incomplete results only when evidence agrees', () => {
    expect(
      parseWorkerResponse({
        ...identity,
        type: 'result',
        operation: 'inspect',
        status: 'completed',
        mode: 'read-only',
        message: 'Verified',
        inspection: inspection(true),
      }),
    ).toMatchObject({ status: 'completed' });
    expect(
      parseWorkerResponse({
        ...identity,
        type: 'result',
        operation: 'inspect',
        status: 'incomplete',
        mode: 'read-only',
        message: 'Missing',
        inspection: inspection(false),
      }),
    ).toMatchObject({ status: 'incomplete' });
  });

  it.each([
    ['unknown response', { ...identity, type: 'surprise' }],
    [
      'extra field',
      {
        ...identity,
        type: 'error',
        operation: 'split',
        status: 'failed',
        code: 'bad',
        message: 'x',
        extra: 1,
      },
    ],
    [
      'invalid progress number',
      {
        ...identity,
        type: 'progress',
        operation: 'split',
        status: 'running',
        bytesProcessed: Number.NaN,
        totalBytes: 2,
        currentSlice: 1,
        sliceCount: 1,
        speedBytesPerSecond: 0,
        message: 'Working',
      },
    ],
    [
      'out-of-bounds progress',
      {
        ...identity,
        type: 'progress',
        operation: 'split',
        status: 'running',
        bytesProcessed: 3,
        totalBytes: 2,
        currentSlice: 1,
        sliceCount: 1,
        speedBytesPerSecond: 0,
        message: 'Working',
      },
    ],
    [
      'non-Blob download',
      { ...identity, type: 'download', operation: 'merge', filename: 'cake.bin', blob: {} },
    ],
    [
      'green inspection result without evidence',
      {
        ...identity,
        type: 'result',
        operation: 'inspect',
        status: 'completed',
        mode: 'read-only',
        message: 'Verified',
        inspection: inspection(false),
      },
    ],
    [
      'inconsistent result fields',
      {
        ...identity,
        type: 'result',
        operation: 'merge',
        status: 'completed',
        mode: 'read-only',
        message: 'Done',
      },
    ],
  ])('rejects %s', (_label, response) => {
    expect(() => parseWorkerResponse(response)).toThrow(WorkerProtocolError);
  });
});

function directoryHandle() {
  return {
    kind: 'directory',
    name: 'output',
    getFileHandle() {},
    removeEntry() {},
    isSameEntry() {},
  } as unknown as FileSystemDirectoryHandle;
}

function inspection(verified: boolean) {
  return {
    manifest: emptyManifestValue,
    foundSliceCount: 0,
    missing: verified ? [] : ['missing.slice'],
    corrupted: [],
    duplicates: [],
    unexpected: [],
    verified,
    slices: [],
  };
}
