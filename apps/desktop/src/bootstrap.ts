import {
  MAX_DESKTOP_TASK_SNAPSHOTS,
  type StartupRecoveryState,
  type TaskSnapshot,
} from './ipc';

export type RecoveryDisplayState = StartupRecoveryState | 'snapshot-unavailable';

export async function installDesktopListeners(
  registrations: Array<() => Promise<() => void>>,
): Promise<{ unlisten: Array<() => void>; errors: unknown[] }> {
  const settled = await Promise.allSettled(registrations.map((register) => register()));
  return {
    unlisten: settled
      .filter((result): result is PromiseFulfilledResult<() => void> => result.status === 'fulfilled')
      .map((result) => result.value),
    errors: settled
      .filter((result): result is PromiseRejectedResult => result.status === 'rejected')
      .map((result) => result.reason as unknown),
  };
}

export function reconcileTaskSnapshots(
  current: TaskSnapshot[],
  incoming: TaskSnapshot[],
): TaskSnapshot[] {
  const byId = new Map(current.map((task) => [task.id, task]));
  for (const task of incoming) {
    const existing = byId.get(task.id);
    if (existing === undefined || task.revision >= existing.revision) {
      byId.set(task.id, task);
    }
  }
  return [...byId.values()]
    .sort((a, b) => b.updatedAt.localeCompare(a.updatedAt))
    .slice(0, MAX_DESKTOP_TASK_SNAPSHOTS);
}

export function canClearTaskState(
  busy: boolean,
  taskCount: number,
  recoveryState: RecoveryDisplayState,
): boolean {
  return !busy && (taskCount > 0 || recoveryState !== 'ready');
}
