export type WorkerControlCommand = 'pause' | 'resume' | 'cancel';
export type StartAdmission = 'accept' | 'duplicate_task_id' | 'worker_busy';

export function classifyStart(
  activeTaskId: string | undefined,
  incomingTaskId: string,
): StartAdmission {
  if (activeTaskId === undefined) return 'accept';
  return activeTaskId === incomingTaskId ? 'duplicate_task_id' : 'worker_busy';
}

export class WorkerTaskControl {
  cancelled = false;
  paused = false;
  private readonly resumeWaiters: Array<() => void> = [];

  apply(command: WorkerControlCommand): 'paused' | 'running' | 'cancelled' | undefined {
    switch (command) {
      case 'cancel':
        this.cancelled = true;
        this.paused = false;
        this.releaseWaiters();
        return 'cancelled';
      case 'pause':
        if (this.paused || this.cancelled) return undefined;
        this.paused = true;
        return 'paused';
      case 'resume':
        if (!this.paused || this.cancelled) return undefined;
        this.paused = false;
        this.releaseWaiters();
        return 'running';
    }
  }

  async checkpoint(stillActive: () => boolean): Promise<void> {
    if (!stillActive() || this.cancelled) throw cancelledError();
    while (this.paused) {
      await new Promise<void>((resolve) => this.resumeWaiters.push(resolve));
      if (!stillActive() || this.cancelled) throw cancelledError();
    }
  }

  release(): void {
    this.cancelled = true;
    this.paused = false;
    this.releaseWaiters();
  }

  private releaseWaiters(): void {
    for (const resume of this.resumeWaiters.splice(0)) resume();
  }
}

function cancelledError(): DOMException {
  return new DOMException('Operation cancelled', 'AbortError');
}
