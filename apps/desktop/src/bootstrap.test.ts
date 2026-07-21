import { describe, expect, it, vi } from 'vitest';

import {
  canClearTaskState,
  installDesktopListeners,
  reconcileTaskSnapshots,
} from './bootstrap';
import { MAX_DESKTOP_TASK_SNAPSHOTS, type TaskSnapshot } from './ipc';

function task(revision: number, updatedAt: string, id = '9cd16d17-3b92-4884-8f65-e0d64d11c93e'): TaskSnapshot {
  return {
    id,
    revision,
    operation: 'split',
    applicationVersion: '0.5.0',
    formatVersion: '1.0',
    priority: 'normal',
    queueOrder: 1,
    queuePosition: 1,
    displayName: 'sample.bin',
    destinationName: 'package',
    plan: {
      totalBytes: 1,
      sliceSize: 1,
      sliceCount: 1,
      requiredFreeBytes: 1,
      minimumRequiredBytes: 1,
      recommendedFreeBytes: 2,
      availableFreeBytes: 3,
      temporaryBytes: 0,
      recoveryOverheadBytes: 0,
      expectedOutputCount: 2,
    },
    preflight: null,
    progress: { bytesProcessed: 0, totalBytes: 1, currentSlice: 0, sliceCount: 1, stage: 'Queued' },
    status: 'queued',
    failure: null,
    failureHistory: [],
    result: null,
    attemptCount: 0,
    startedAt: null,
    finishedAt: null,
    durationMs: null,
    recoveryEligible: false,
    createdAt: '2026-07-18T00:00:00.000Z',
    updatedAt,
  };
}

describe('desktop bootstrap recovery', () => {
  it('attempts every listener independently and retains successful subscriptions', async () => {
    const stopOne = vi.fn();
    const stopTwo = vi.fn();
    const stopThree = vi.fn();
    const calls: number[] = [];
    const result = await installDesktopListeners([
      async () => { calls.push(1); return stopOne; },
      async () => { calls.push(2); throw new Error('one listener unavailable'); },
      async () => { calls.push(3); return stopTwo; },
      async () => { calls.push(4); return stopThree; },
    ]);
    expect(calls).toEqual([1, 2, 3, 4]);
    expect(result.unlisten).toHaveLength(3);
    expect(result.errors).toHaveLength(1);
  });

  it('keeps a newer event when a stale bootstrap snapshot resolves later', () => {
    const newer = task(3, '2026-07-18T00:00:03.000Z');
    const stale = task(2, '2026-07-18T00:00:02.000Z');
    expect(reconcileTaskSnapshots([newer], [stale])).toEqual([newer]);
  });

  it('bounds merged event and bootstrap task snapshots to the native retained-task cap', () => {
    const tasks = Array.from({ length: MAX_DESKTOP_TASK_SNAPSHOTS + 1 }, (_, index) =>
      task(1, new Date(index).toISOString(), `00000000-0000-4000-8000-${String(index).padStart(12, '0')}`),
    );
    const reconciled = reconcileTaskSnapshots(tasks.slice(0, -1), tasks.slice(-1));
    expect(reconciled).toHaveLength(MAX_DESKTOP_TASK_SNAPSHOTS);
    expect(reconciled[0]?.id).toBe(tasks.at(-1)?.id);
  });

  it('keeps Clear All available when snapshot validation or native recovery fails', () => {
    expect(canClearTaskState(false, 0, 'snapshot-unavailable')).toBe(true);
    expect(canClearTaskState(false, 0, 'corrupt')).toBe(true);
    expect(canClearTaskState(false, 0, 'capacity-exceeded')).toBe(true);
    expect(canClearTaskState(false, 0, 'ready')).toBe(false);
    expect(canClearTaskState(true, 1, 'corrupt')).toBe(false);
  });
});
