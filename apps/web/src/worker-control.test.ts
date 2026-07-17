import { describe, expect, it } from 'vitest';

import { classifyStart, WorkerTaskControl } from './worker-control';

describe('Worker task admission and control', () => {
  it('distinguishes duplicate task IDs from a busy Worker', () => {
    expect(classifyStart(undefined, 'task-a')).toBe('accept');
    expect(classifyStart('task-a', 'task-a')).toBe('duplicate_task_id');
    expect(classifyStart('task-a', 'task-b')).toBe('worker_busy');
  });

  it('pauses only at a checkpoint and resumes deterministically', async () => {
    const control = new WorkerTaskControl();
    expect(control.apply('pause')).toBe('paused');
    let passed = false;
    const checkpoint = control.checkpoint(() => true).then(() => {
      passed = true;
    });
    await Promise.resolve();
    expect(passed).toBe(false);
    expect(control.apply('resume')).toBe('running');
    await checkpoint;
    expect(passed).toBe(true);
  });

  it('cancels a paused checkpoint without converting it to success', async () => {
    const control = new WorkerTaskControl();
    control.apply('pause');
    const checkpoint = control.checkpoint(() => true);
    control.apply('cancel');
    await expect(checkpoint).rejects.toMatchObject({ name: 'AbortError' });
    expect(control.cancelled).toBe(true);
  });

  it('rejects a checkpoint after the active task identity changes', async () => {
    const control = new WorkerTaskControl();
    await expect(control.checkpoint(() => false)).rejects.toMatchObject({ name: 'AbortError' });
  });
});
