import { renderToString } from 'react-dom/server';
import { describe, expect, it } from 'vitest';

import { TasksWorkspace } from './App';
import type { StorageSummary, TaskSnapshot } from './ipc';

const hash = 'a'.repeat(64);

function task(index: number): TaskSnapshot {
  const id = `00000000-0000-4000-8000-${index.toString().padStart(12, '0')}`;
  return {
    id,
    revision: 1,
    operation: index % 2 === 0 ? 'split' : 'merge',
    applicationVersion: '0.5.0',
    formatVersion: '1.0',
    priority: ['high', 'normal', 'low'][index % 3] as TaskSnapshot['priority'],
    queueOrder: index + 1,
    queuePosition: null,
    displayName: `history-${index}.bin`,
    destinationName: `output-${index}`,
    plan: {
      totalBytes: 1024,
      sliceSize: 512,
      sliceCount: 2,
      requiredFreeBytes: 2048,
      minimumRequiredBytes: 2048,
      recommendedFreeBytes: 4096,
      availableFreeBytes: 8192,
      temporaryBytes: 512,
      recoveryOverheadBytes: 256,
      expectedOutputCount: 3,
    },
    preflight: null,
    progress: {
      bytesProcessed: 1024,
      totalBytes: 1024,
      currentSlice: 2,
      sliceCount: 2,
      stage: 'Completed',
    },
    status: 'completed',
    failure: null,
    failureHistory: [],
    result: index % 2 === 0
      ? { type: 'split', manifestFilename: `history-${index}.bin.cake.json`, sourceSha256: hash }
      : { type: 'merge', outputFilename: `output-${index}`, outputSha256: hash },
    attemptCount: 1,
    startedAt: '2026-07-20T00:00:00.000Z',
    finishedAt: '2026-07-20T00:00:01.000Z',
    durationMs: 1000,
    recoveryEligible: false,
    createdAt: '2026-07-20T00:00:00.000Z',
    updatedAt: '2026-07-20T00:00:01.000Z',
  };
}

const storage: StorageSummary = {
  databaseBytes: 4_000_000,
  activeTasks: 0,
  nonterminalTasks: 0,
  terminalHistoryTasks: 500,
  quarantinedRecords: 0,
  incompleteOutputReferences: 0,
  diagnosticBundleCount: 0,
  maximumTerminalHistory: 500,
  terminalHistoryDays: 90,
};

describe('bounded task history rendering', () => {
  it('renders the maximum 500-row history without dropping task identity', () => {
    const tasks = Array.from({ length: 500 }, (_, index) => task(index));
    const started = performance.now();
    const html = renderToString(
      <TasksWorkspace
        tasks={tasks}
        storage={storage}
        recoveryState="ready"
        busy={false}
        onControl={() => undefined}
        onPriority={() => undefined}
        onReorder={() => undefined}
        onReceipt={() => undefined}
        onRemove={() => undefined}
        onClearAll={() => undefined}
      />,
    );
    const elapsed = performance.now() - started;
    console.info(`HISTORY_RENDER_METRICS rows=500 duration_ms=${elapsed.toFixed(3)} html_bytes=${html.length}`);
    expect(html).toContain('history-0.bin');
    expect(html).toContain('history-499.bin');
    expect(elapsed).toBeLessThan(2_000);
  });
});
