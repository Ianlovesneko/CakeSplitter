import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';

import { describe, expect, it } from 'vitest';

import {
  CLI_SCHEMA_VERSION,
  CliContractValidationError,
  validateCliFinalResult,
  validateCliJsonlEvent,
  validateCliJsonlStream,
  validateCliStructuredError,
} from '../src/index';

const operationId = 'ff7cb026-f7ec-4d17-a3e4-8083217ec688';
const error = {
  code: 'source_changed',
  category: 'source',
  message: 'The selected source changed.',
  technicalMessage: 'source changed',
  retryable: false,
  suggestedAction: 'Reselect the original source.',
  operationId,
};

describe('CLI schema version 1', () => {
  it('ships three parseable versioned JSON schemas', () => {
    for (const filename of [
      'cli-final-result.schema.json',
      'cli-jsonl-event.schema.json',
      'cli-error.schema.json',
    ]) {
      const schema = JSON.parse(readFileSync(resolve('specs', filename), 'utf8')) as Record<string, unknown>;
      expect(schema.$schema).toBe('https://json-schema.org/draft/2020-12/schema');
      expect(schema.additionalProperties).toBe(false);
    }
  });

  it('validates completed and failed final documents', () => {
    const base = {
      schemaVersion: CLI_SCHEMA_VERSION,
      applicationVersion: '0.8.1',
      command: 'split',
      warnings: [],
      startedAt: '2026-07-21T12:00:00.000Z',
      completedAt: '2026-07-21T12:00:01.000Z',
      durationMs: 1000,
    };
    expect(validateCliFinalResult({ ...base, status: 'completed', result: {}, error: null }).status)
      .toBe('completed');
    expect(validateCliFinalResult({ ...base, status: 'failed', result: null, error }).error?.code)
      .toBe('source_changed');
  });

  it('validates JSONL events and rejects unknown or malformed fields', () => {
    expect(validateCliJsonlEvent({
      schemaVersion: 1,
      event: 'completed',
      command: 'verify',
      operationId,
      timestamp: '2026-07-21T12:00:01.000Z',
      sequence: 4,
      payload: { status: 'completed' },
    }).sequence).toBe(4);
    expect(() => validateCliJsonlEvent({ schemaVersion: 1 })).toThrow(CliContractValidationError);
    expect(() => validateCliStructuredError({ ...error, absolutePath: 'C:\\private' }))
      .toThrow(CliContractValidationError);
    expect(() => validateCliJsonlEvent({
      schemaVersion: 1,
      event: 'completed',
      command: 'verify',
      operationId,
      timestamp: '1',
      sequence: 4,
      payload: {},
    })).toThrow(CliContractValidationError);
  });

  it('validates stream sequencing and schema length limits', () => {
    const stream = [
      { schemaVersion: 1, event: 'started', command: 'verify', operationId, timestamp: '2026-07-21T12:00:00Z', sequence: 1, payload: {} },
      { schemaVersion: 1, event: 'completed', command: 'verify', operationId, timestamp: '2026-07-21T12:00:01Z', sequence: 2, payload: { status: 'completed' } },
    ];
    expect(validateCliJsonlStream(stream)).toHaveLength(2);
    expect(() => validateCliJsonlStream(stream.slice().reverse())).toThrow(CliContractValidationError);
    expect(() => validateCliStructuredError({ ...error, message: 'x'.repeat(2001) }))
      .toThrow(CliContractValidationError);
    expect(() => validateCliFinalResult({
      schemaVersion: 1,
      applicationVersion: '0.8.1',
      command: 'verify',
      status: 'completed',
      result: {},
      warnings: ['x'.repeat(2001)],
      error: null,
      startedAt: '2026-07-21T12:00:00Z',
      completedAt: '2026-07-21T12:00:01Z',
      durationMs: 1,
    })).toThrow(CliContractValidationError);
  });

  it('validates the canonical batch result and batch JSONL stream', () => {
    const runId = operationId;
    const operation = {
      id: 'verify-package',
      command: 'verify',
      status: 'completed',
      attemptCount: 1,
      result: { verified: true },
      error: null,
    };
    const result = validateCliFinalResult({
      schemaVersion: 1,
      applicationVersion: '0.8.1',
      command: 'batch',
      status: 'completed',
      result: { operations: [operation] },
      warnings: [],
      error: null,
      startedAt: '2026-07-23T12:00:00.000Z',
      completedAt: '2026-07-23T12:00:01.000Z',
      durationMs: 1000,
      runId,
      jobName: 'nightly-package-check',
      jobSpecDigest: 'a'.repeat(64),
      failurePolicy: 'stop',
      operationCounts: { completed: 1 },
      operations: [operation],
    });
    expect(result.command).toBe('batch');
    const stream = validateCliJsonlStream([
      { schemaVersion: 1, event: 'started', command: 'batch', operationId: runId, runId, timestamp: '2026-07-23T12:00:00Z', sequence: 1, payload: { runId } },
      { schemaVersion: 1, event: 'operation-progress', command: 'batch', operationId, runId, timestamp: '2026-07-23T12:00:00Z', sequence: 2, payload: { runId, operationId } },
      { schemaVersion: 1, event: 'batch-completed', command: 'batch', operationId: runId, runId, timestamp: '2026-07-23T12:00:01Z', sequence: 3, payload: { runId, status: 'completed' } },
    ]);
    expect(stream.at(-1)?.event).toBe('batch-completed');
    expect(() => validateCliJsonlStream([
      { schemaVersion: 1, event: 'started', command: 'batch', operationId: runId, runId, timestamp: '2026-07-23T12:00:00Z', sequence: 1, payload: { runId } },
      { schemaVersion: 1, event: 'batch-completed', command: 'batch', operationId: runId, runId: operationId.replace(/.$/u, '0'), timestamp: '2026-07-23T12:00:01Z', sequence: 2, payload: { runId } },
    ])).toThrow(CliContractValidationError);
    expect(() => validateCliJsonlStream([
      { schemaVersion: 1, event: 'started', command: 'batch', operationId: runId, runId, timestamp: '2026-07-23T12:00:00Z', sequence: 1, payload: { runId } },
      { schemaVersion: 1, event: 'batch-completed', command: 'batch', operationId: runId, runId, timestamp: '2026-07-23T12:00:01Z', sequence: 2, payload: { runId } },
      { schemaVersion: 1, event: 'operation-progress', command: 'batch', operationId: 'verify-package', runId, timestamp: '2026-07-23T12:00:02Z', sequence: 3, payload: { runId, operationId: 'verify-package' } },
    ])).toThrow(CliContractValidationError);
    expect(() => validateCliJsonlEvent({
      schemaVersion: 1,
      event: 'progress',
      command: 'verify',
      operationId,
      timestamp: '2026-07-23T12:00:01Z',
      sequence: Number.MAX_SAFE_INTEGER + 1,
      payload: {},
    })).toThrow(CliContractValidationError);
  });

  it('validates sanitized cross-language contract fixtures', () => {
    const fixtureRoot = resolve('tests', 'fixtures', 'cli-contract');
    for (const filename of ['final-result.json', 'batch-final-result.json']) {
      validateCliFinalResult(JSON.parse(readFileSync(resolve(fixtureRoot, filename), 'utf8')) as unknown);
    }
    for (const filename of ['stream.jsonl', 'batch-failure.jsonl', 'batch-cancelled.jsonl']) {
      const values = readFileSync(resolve(fixtureRoot, filename), 'utf8')
        .trim()
        .split(/\r?\n/u)
        .map((line) => JSON.parse(line) as unknown);
      validateCliJsonlStream(values);
    }
  });
});
