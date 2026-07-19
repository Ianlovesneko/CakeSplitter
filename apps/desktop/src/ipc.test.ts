import { describe, expect, it } from 'vitest';

import { parseRuntimeInfo, parseSelection, parseSettings, parseTask } from './ipc';

const id = '9cd16d17-3b92-4884-8f65-e0d64d11c93e';
const hash = 'a'.repeat(64);

function validTask(): Record<string, unknown> {
  return {
    id,
    operation: 'split',
    applicationVersion: '0.4.0-dev',
    formatVersion: '1.0',
    displayName: 'sample.bin',
    destinationName: 'package',
    plan: {
      totalBytes: 2048,
      sliceSize: 1024,
      sliceCount: 2,
      requiredFreeBytes: 4096,
    },
    progress: {
      bytesProcessed: 1024,
      totalBytes: 2048,
      currentSlice: 1,
      sliceCount: 2,
      stage: 'Writing Slice',
    },
    status: 'running',
    failure: null,
    result: null,
    recoveryEligible: false,
    createdAt: '2026-07-18T00:00:00.000Z',
    updatedAt: '2026-07-18T00:00:01.000Z',
  };
}

describe('desktop IPC runtime validation', () => {
  it('accepts the exact runtime, selection, settings, and task schemas', () => {
    expect(
      parseRuntimeInfo({
        applicationVersion: '0.4.0-dev',
        formatVersion: '1.0',
        platform: 'windows-x64',
        automaticUpdates: false,
        telemetry: false,
        backgroundService: false,
        signedBuild: false,
      }).telemetry,
    ).toBe(false);
    expect(
      parseSelection({
        token: id,
        kind: 'sourceFile',
        displayName: 'sample.bin',
        size: 2048,
        count: 1,
      }).size,
    ).toBe(2048);
    expect(
      parseSettings({
        defaultSliceSize: 1024,
        confirmDestructiveActions: true,
        reduceMotion: false,
      }).defaultSliceSize,
    ).toBe(1024);
    expect(parseTask(validTask()).status).toBe('running');
  });

  it('rejects unknown fields and unsupported enum values', () => {
    expect(() => parseTask({ ...validTask(), sourcePath: 'C:\\private.bin' })).toThrow(
      /outside the expected schema/u,
    );
    expect(() => parseTask({ ...validTask(), status: 'secret-state' })).toThrow(
      /unsupported enum/u,
    );
  });

  it('rejects unsafe numbers, invalid hashes, and overlong diagnostics', () => {
    const unsafe = validTask();
    unsafe.plan = {
      totalBytes: Number.MAX_SAFE_INTEGER + 1,
      sliceSize: 1024,
      sliceCount: 2,
      requiredFreeBytes: 4096,
    };
    expect(() => parseTask(unsafe)).toThrow(/unsafe numeric/u);

    const invalidResult = validTask();
    invalidResult.result = {
      type: 'split',
      manifestFilename: 'sample.bin.cake.json',
      sourceSha256: hash.toUpperCase(),
    };
    expect(() => parseTask(invalidResult)).toThrow(/invalid SHA-256/u);

    const longFailure = validTask();
    longFailure.failure = { code: 'io_error', message: 'x'.repeat(2_001) };
    expect(() => parseTask(longFailure)).toThrow(/overlong/u);
  });
});
