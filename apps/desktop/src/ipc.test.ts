import { describe, expect, it } from 'vitest';

import {
  MAX_DESKTOP_TASK_SNAPSHOTS,
  MAX_UNEXPECTED_SLICE_DIAGNOSTICS,
  dispatchValidatedEvent,
  parseInspection,
  parsePreflight,
  parseRuntimeInfo,
  parseSelection,
  parseSettings,
  parseTask,
  parseTaskList,
} from './ipc';

const id = '9cd16d17-3b92-4884-8f65-e0d64d11c93e';
const hash = 'a'.repeat(64);

function validTask(): Record<string, unknown> {
  return {
    id,
    revision: 1,
    operation: 'split',
    applicationVersion: '0.6.0-dev',
    formatVersion: '1.0',
    priority: 'normal',
    queueOrder: 1,
    queuePosition: null,
    displayName: 'sample.bin',
    destinationName: 'package',
    plan: {
      totalBytes: 2048,
      sliceSize: 1024,
      sliceCount: 2,
      requiredFreeBytes: 4096,
      minimumRequiredBytes: 4096,
      recommendedFreeBytes: 8192,
      availableFreeBytes: 16384,
      temporaryBytes: 1024,
      recoveryOverheadBytes: 512,
      expectedOutputCount: 3,
    },
    preflight: null,
    progress: {
      bytesProcessed: 1024,
      totalBytes: 2048,
      currentSlice: 1,
      sliceCount: 2,
      stage: 'Writing Slice',
    },
    status: 'running',
    failure: null,
    failureHistory: [],
    result: null,
    attemptCount: 1,
    startedAt: '2026-07-18T00:00:00.000Z',
    finishedAt: null,
    durationMs: null,
    recoveryEligible: false,
    createdAt: '2026-07-18T00:00:00.000Z',
    updatedAt: '2026-07-18T00:00:01.000Z',
  };
}

describe('desktop IPC runtime validation', () => {
  it('accepts the exact runtime, selection, settings, and task schemas', () => {
    expect(
      parseRuntimeInfo({
        applicationVersion: '0.6.0-dev',
        formatVersion: '1.0',
        platform: 'windows-x64',
        automaticUpdates: false,
        telemetry: false,
        backgroundService: false,
        signedBuild: false,
        startupRecovery: {
          state: 'ready',
          recoveredTasks: 0,
          quarantinedRecords: 0,
          capacityExceededRecords: 0,
        },
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
        maximumTerminalHistory: 500,
        terminalHistoryDays: 90,
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
      minimumRequiredBytes: 4096,
      recommendedFreeBytes: 8192,
      availableFreeBytes: 16384,
      temporaryBytes: 1024,
      recoveryOverheadBytes: 512,
      expectedOutputCount: 3,
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
    longFailure.failure = {
      code: 'io_error',
      message: 'x'.repeat(2_001),
      technicalMessage: 'local filesystem failure',
      category: 'destination',
      retryable: true,
      recoveryAction: 'retry',
      occurredAt: '2026-07-18T00:00:01.000Z',
      attempt: 1,
    };
    expect(() => parseTask(longFailure)).toThrow(/overlong/u);
  });

  it('validates bounded preflight warnings and conflicts', () => {
    const preflight = {
      state: 'ready-with-warning',
      checkedAt: '2026-07-18T00:00:00.000Z',
      minimumRequiredBytes: 1,
      recommendedFreeBytes: 2,
      availableFreeBytes: 1,
      temporaryBytes: 0,
      recoveryOverheadBytes: 0,
      expectedOutputCount: 2,
      warnings: [{ code: 'low_space', message: 'Free space is below the recommendation.' }],
      conflicts: [{
        conflictingTaskId: 'current-selection',
        class: 'informational-overlap',
        conflictType: 'shared-input',
        affectedResource: 'sample.bin',
        recommendedAction: 'Review the overlap before continuing.',
      }],
    };
    expect(parsePreflight(preflight).state).toBe('ready-with-warning');
    expect(() => parsePreflight({
      ...preflight,
      warnings: Array.from({ length: 21 }, () => preflight.warnings[0]),
    })).toThrow(/bounded array/u);
  });

  it('enforces native-aligned task and inspection response limits', () => {
    const exactTasks = Array.from({ length: MAX_DESKTOP_TASK_SNAPSHOTS }, (_, index) => ({
      ...validTask(),
      id: `9cd16d17-3b92-4884-8f65-${index.toString(16).padStart(12, '0')}`,
    }));
    expect(parseTaskList(exactTasks)).toHaveLength(MAX_DESKTOP_TASK_SNAPSHOTS);
    expect(() => parseTaskList([...exactTasks, validTask()])).toThrow(/invalid task list/u);

    const inspection = {
      packageId: id,
      formatVersion: '1.0',
      originalFilename: 'sample.bin',
      originalSize: 1,
      originalSha256: hash,
      expectedSliceCount: 1,
      foundSliceCount: 1,
      missing: [],
      corrupted: [],
      unexpected: Array.from(
        { length: MAX_UNEXPECTED_SLICE_DIAGNOSTICS },
        (_, index) => `unexpected-${index}.slice`,
      ),
      verified: false,
    };
    expect(parseInspection(inspection).unexpected).toHaveLength(
      MAX_UNEXPECTED_SLICE_DIAGNOSTICS,
    );
    expect(() => parseInspection({
      ...inspection,
      unexpected: [...inspection.unexpected, 'one-too-many.slice'],
    })).toThrow(/invalid array/u);
  });

  it('contains one malformed event without disabling later valid events', () => {
    const accepted: string[] = [];
    const rejected: string[] = [];
    dispatchValidatedEvent(
      { ...validTask(), status: 'malformed' },
      parseTask,
      (task) => accepted.push(task.status),
      (message) => rejected.push(message),
    );
    dispatchValidatedEvent(
      validTask(),
      parseTask,
      (task) => accepted.push(task.status),
      (message) => rejected.push(message),
    );
    expect(rejected).toHaveLength(1);
    expect(accepted).toEqual(['running']);
  });
});
