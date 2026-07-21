import { invoke } from '@tauri-apps/api/core';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';

const TASK_STATUSES = [
  'planned',
  'queued',
  'running',
  'pausing',
  'paused',
  'resuming',
  'cancelling',
  'cancelled',
  'interrupted',
  'permission-required',
  'failed',
  'completed',
] as const;

const TASK_OPERATIONS = ['split', 'merge', 'inspect', 'verify'] as const;
const TASK_PRIORITIES = ['high', 'normal', 'low'] as const;
const PREFLIGHT_STATES = [
  'ready',
  'ready-with-warning',
  'blocked',
  'reselection-required',
] as const;
const CONFLICT_CLASSES = [
  'informational-overlap',
  'recoverable-conflict',
  'duplicate-task',
  'hard-conflict',
] as const;
const CONFLICT_TYPES = [
  'shared-input',
  'same-source',
  'same-manifest',
  'same-package-directory',
  'same-output',
  'overlapping-output',
  'same-package-id',
  'source-used-as-destination',
  'destination-inside-package',
  'duplicate-operation',
] as const;
const ERROR_CATEGORIES = [
  'source',
  'destination',
  'package',
  'integrity',
  'permission',
  'space',
  'recovery',
  'queue',
  'storage',
  'capability',
  'internal',
] as const;
const RECOVERY_ACTIONS = [
  'retry',
  'reselect-source',
  'reselect-destination',
  'reselect-package',
  'free-space',
  'close-conflicting-application',
  'remove-conflict',
  'none',
] as const;
const STARTUP_RECOVERY_STATES = [
  'ready',
  'recovery-required',
  'quarantined',
  'capacity-exceeded',
  'unsupported-version',
  'corrupt',
] as const;
export const MAX_DESKTOP_TASK_SNAPSHOTS = 564;
export const MAX_UNEXPECTED_SLICE_DIAGNOSTICS = 1_024;
export const MAX_RENDERED_PACKAGE_DIAGNOSTIC_ROWS = 20;
const SELECTION_KINDS = [
  'sourceFile',
  'manifestFile',
  'packageFolder',
  'outputFolder',
  'outputFile',
  'sliceFiles',
] as const;

export type TaskStatus = (typeof TASK_STATUSES)[number];
export type TaskOperation = (typeof TASK_OPERATIONS)[number];
export type TaskPriority = (typeof TASK_PRIORITIES)[number];
export type PreflightState = (typeof PREFLIGHT_STATES)[number];
export type ConflictClass = (typeof CONFLICT_CLASSES)[number];
export type ConflictType = (typeof CONFLICT_TYPES)[number];
export type ErrorCategory = (typeof ERROR_CATEGORIES)[number];
export type RecoveryAction = (typeof RECOVERY_ACTIONS)[number];
export type SelectionKind = (typeof SELECTION_KINDS)[number];
export type StartupRecoveryState = (typeof STARTUP_RECOVERY_STATES)[number];

export interface StartupRecoveryReport {
  state: StartupRecoveryState;
  recoveredTasks: number;
  quarantinedRecords: number;
  capacityExceededRecords: number;
}

export interface RuntimeInfo {
  applicationVersion: string;
  formatVersion: string;
  platform: string;
  automaticUpdates: boolean;
  telemetry: boolean;
  backgroundService: boolean;
  signedBuild: boolean;
  startupRecovery: StartupRecoveryReport;
}

export interface SelectionSummary {
  token: string;
  kind: SelectionKind;
  displayName: string;
  size: number | null;
  count: number;
}

export interface ProcessingPlan {
  totalBytes: number;
  sliceSize: number;
  sliceCount: number;
  requiredFreeBytes: number;
  minimumRequiredBytes: number;
  recommendedFreeBytes: number;
  availableFreeBytes: number;
  temporaryBytes: number;
  recoveryOverheadBytes: number;
  expectedOutputCount: number;
}

export interface TaskConflict {
  conflictingTaskId: string;
  class: ConflictClass;
  conflictType: ConflictType;
  affectedResource: string;
  recommendedAction: string;
}

export interface PreflightWarning {
  code: string;
  message: string;
}

export interface PreflightResult {
  state: PreflightState;
  checkedAt: string;
  minimumRequiredBytes: number;
  recommendedFreeBytes: number;
  availableFreeBytes: number;
  temporaryBytes: number;
  recoveryOverheadBytes: number;
  expectedOutputCount: number;
  warnings: PreflightWarning[];
  conflicts: TaskConflict[];
}

export interface TaskProgress {
  bytesProcessed: number;
  totalBytes: number;
  currentSlice: number;
  sliceCount: number;
  stage: string;
}

export interface TaskFailure {
  code: string;
  message: string;
  technicalMessage: string;
  category: ErrorCategory;
  retryable: boolean;
  recoveryAction: RecoveryAction;
  occurredAt: string;
  attempt: number;
}

export interface InspectionSummary {
  packageId: string;
  formatVersion: string;
  originalFilename: string;
  originalSize: number;
  originalSha256: string;
  expectedSliceCount: number;
  foundSliceCount: number;
  missing: string[];
  corrupted: string[];
  unexpected: string[];
  verified: boolean;
}

export type TaskResult =
  | { type: 'split'; manifestFilename: string; sourceSha256: string }
  | { type: 'merge'; outputFilename: string; outputSha256: string }
  | { type: 'inspection'; inspection: InspectionSummary };

export interface TaskSnapshot {
  id: string;
  revision: number;
  operation: TaskOperation;
  applicationVersion: string;
  formatVersion: string;
  priority: TaskPriority;
  queueOrder: number;
  queuePosition: number | null;
  displayName: string;
  destinationName: string | null;
  plan: ProcessingPlan;
  preflight: PreflightResult | null;
  progress: TaskProgress;
  status: TaskStatus;
  failure: TaskFailure | null;
  failureHistory: TaskFailure[];
  result: TaskResult | null;
  attemptCount: number;
  startedAt: string | null;
  finishedAt: string | null;
  durationMs: number | null;
  recoveryEligible: boolean;
  createdAt: string;
  updatedAt: string;
}

export interface DesktopSettings {
  defaultSliceSize: number;
  confirmDestructiveActions: boolean;
  reduceMotion: boolean;
  maximumTerminalHistory: number;
  terminalHistoryDays: number;
}

export interface StorageSummary {
  databaseBytes: number;
  activeTasks: number;
  nonterminalTasks: number;
  terminalHistoryTasks: number;
  quarantinedRecords: number;
  incompleteOutputReferences: number;
  diagnosticBundleCount: number;
  maximumTerminalHistory: number;
  terminalHistoryDays: number;
}

export interface ExportResult {
  displayName: string;
  bytesWritten: number;
  kind: string;
  revealToken: string;
}

export interface CommandError {
  code: string;
  message: string;
  retryable: boolean;
  recoveryAction: string | null;
  conflict: TaskConflict | null;
}

export async function getRuntimeInfo(): Promise<RuntimeInfo> {
  return parseRuntimeInfo(await invoke<unknown>('get_runtime_info'));
}

export async function chooseSourceFile(): Promise<SelectionSummary | null> {
  return parseNullableSelection(await invoke<unknown>('choose_source_file'));
}

export async function chooseManifestFile(): Promise<SelectionSummary | null> {
  return parseNullableSelection(await invoke<unknown>('choose_manifest_file'));
}

export async function choosePackageFolder(): Promise<SelectionSummary | null> {
  return parseNullableSelection(await invoke<unknown>('choose_package_folder'));
}

export async function chooseOutputFolder(): Promise<SelectionSummary | null> {
  return parseNullableSelection(await invoke<unknown>('choose_output_folder'));
}

export async function chooseOutputFile(
  suggestedName: string,
): Promise<SelectionSummary | null> {
  return parseNullableSelection(
    await invoke<unknown>('choose_output_file', { suggestedName }),
  );
}

export async function chooseSliceFiles(): Promise<SelectionSummary | null> {
  return parseNullableSelection(await invoke<unknown>('choose_slice_files'));
}

export async function planSplit(
  sourceToken: string,
  outputToken: string,
  sliceSize: number,
): Promise<ProcessingPlan> {
  return parsePlan(
    await invoke<unknown>('plan_split', { sourceToken, outputToken, sliceSize }),
  );
}

export async function preflightSplit(
  sourceToken: string,
  outputToken: string,
  sliceSize: number,
): Promise<PreflightResult> {
  return parsePreflight(
    await invoke<unknown>('preflight_split', { sourceToken, outputToken, sliceSize }),
  );
}

export async function previewMerge(packageToken: string): Promise<InspectionSummary> {
  return parseInspection(
    await invoke<unknown>('preview_merge', { packageToken }),
  );
}

export async function preflightMerge(
  packageToken: string,
  outputToken: string,
): Promise<PreflightResult> {
  return parsePreflight(
    await invoke<unknown>('preflight_merge', { packageToken, outputToken }),
  );
}

export async function enqueueSplit(
  sourceToken: string,
  outputToken: string,
  sliceSize: number,
  priority: TaskPriority = 'normal',
): Promise<TaskSnapshot> {
  return parseTask(
    await invoke<unknown>('enqueue_split', {
      sourceToken,
      outputToken,
      sliceSize,
      priority,
    }),
  );
}

export async function enqueueMerge(
  packageToken: string,
  outputToken: string,
  priority: TaskPriority = 'normal',
): Promise<TaskSnapshot> {
  return parseTask(
    await invoke<unknown>('enqueue_merge', { packageToken, outputToken, priority }),
  );
}

export async function enqueueInspect(
  packageToken: string,
  verifyHashes: boolean,
  priority: TaskPriority = 'normal',
): Promise<TaskSnapshot> {
  return parseTask(
    await invoke<unknown>('inspect_package', { packageToken, verifyHashes, priority }),
  );
}

export async function enqueueVerify(
  packageToken: string,
  priority: TaskPriority = 'normal',
): Promise<TaskSnapshot> {
  return parseTask(await invoke<unknown>('verify_package', { packageToken, priority }));
}

export async function listTasks(): Promise<TaskSnapshot[]> {
  return parseTaskList(await invoke<unknown>('list_tasks'));
}

export function parseTaskList(value: unknown): TaskSnapshot[] {
  if (!Array.isArray(value) || value.length > MAX_DESKTOP_TASK_SNAPSHOTS) {
    throw new Error('Desktop IPC returned an invalid task list.');
  }
  return value.map(parseTask);
}

export async function controlTask(
  command: 'pause_task' | 'resume_task' | 'cancel_task' | 'retry_task',
  taskId: string,
): Promise<TaskSnapshot> {
  return parseTask(await invoke<unknown>(command, { taskId }));
}

export async function setTaskPriority(
  taskId: string,
  priority: TaskPriority,
): Promise<TaskSnapshot> {
  return parseTask(await invoke<unknown>('set_task_priority', { taskId, priority }));
}

export async function reorderTask(
  taskId: string,
  direction: 'earlier' | 'later',
): Promise<TaskSnapshot[]> {
  return parseTaskList(await invoke<unknown>('reorder_task', { taskId, direction }));
}

export async function removeTask(taskId: string): Promise<void> {
  parseVoid(await invoke<unknown>('remove_task', { taskId }));
}

export async function clearAllTasks(): Promise<void> {
  parseVoid(await invoke<unknown>('clear_all_tasks'));
}

export async function clearCompletedHistory(): Promise<number> {
  return safeInteger(await invoke<unknown>('clear_completed_history'));
}

export async function clearFailedHistory(): Promise<number> {
  return safeInteger(await invoke<unknown>('clear_failed_history'));
}

export async function clearQuarantine(): Promise<number> {
  return safeInteger(await invoke<unknown>('clear_quarantine'));
}

export async function getStorageSummary(): Promise<StorageSummary> {
  return parseStorageSummary(await invoke<unknown>('get_storage_summary'));
}

export async function exportReceipt(
  taskId: string,
  outputToken: string,
  format: 'markdown' | 'json',
  includePathDetail: boolean,
): Promise<ExportResult> {
  return parseExportResult(
    await invoke<unknown>('export_receipt', {
      taskId,
      outputToken,
      format,
      includePathDetail,
    }),
  );
}

export async function exportDiagnosticBundle(
  outputToken: string,
): Promise<ExportResult> {
  return parseExportResult(
    await invoke<unknown>('export_diagnostic_bundle', { outputToken }),
  );
}

export async function revealExport(revealToken: string): Promise<void> {
  parseVoid(await invoke<unknown>('reveal_export', { revealToken }));
}

export async function getSettings(): Promise<DesktopSettings> {
  return parseSettings(await invoke<unknown>('get_settings'));
}

export async function updateSettings(
  settings: DesktopSettings,
): Promise<DesktopSettings> {
  return parseSettings(await invoke<unknown>('update_settings', { settings }));
}

export async function prepareAppClose(
  action: 'check' | 'keepOpen' | 'cancelTasks' | 'interruptAndExit',
): Promise<string[]> {
  return parseStringArray(await invoke<unknown>('prepare_app_close', { action }), 500);
}

export async function onTaskUpdate(
  handler: (task: TaskSnapshot) => void,
  onInvalid?: (message: string) => void,
): Promise<UnlistenFn> {
  return listen<unknown>('task-update', (event) => {
    dispatchValidatedEvent(event.payload, parseTask, handler, onInvalid);
  });
}

export async function onNativeDrop(
  handler: (selection: SelectionSummary) => void,
  onInvalid?: (message: string) => void,
): Promise<UnlistenFn> {
  return listen<unknown>('native-drop', (event) => {
    dispatchValidatedEvent(event.payload, parseSelection, handler, onInvalid);
  });
}

export async function onNativeDropError(
  handler: (error: CommandError) => void,
  onInvalid?: (message: string) => void,
): Promise<UnlistenFn> {
  return listen<unknown>('native-drop-error', (event) => {
    dispatchValidatedEvent(event.payload, parseCommandError, handler, onInvalid);
  });
}

export async function onCloseRequested(
  handler: (taskIds: string[]) => void,
  onInvalid?: (message: string) => void,
): Promise<UnlistenFn> {
  return listen<unknown>('close-requested-with-active-tasks', (event) => {
    dispatchValidatedEvent(
      event.payload,
      (payload) => parseStringArray(payload, 1),
      handler,
      onInvalid,
    );
  });
}

export function dispatchValidatedEvent<T>(
  payload: unknown,
  parser: (value: unknown) => T,
  handler: (value: T) => void,
  onInvalid?: (message: string) => void,
): void {
  try {
    handler(parser(payload));
  } catch (cause) {
    onInvalid?.(errorMessage(cause));
  }
}

export function errorMessage(error: unknown): string {
  try {
    return parseCommandError(error).message;
  } catch {
    return error instanceof Error ? error.message : 'The local operation failed safely.';
  }
}

export function parseRuntimeInfo(value: unknown): RuntimeInfo {
  const record = exactRecord(value, [
    'applicationVersion',
    'formatVersion',
    'platform',
    'automaticUpdates',
    'telemetry',
    'backgroundService',
    'signedBuild',
    'startupRecovery',
  ]);
  return {
    applicationVersion: stringValue(record.applicationVersion),
    formatVersion: stringValue(record.formatVersion),
    platform: stringValue(record.platform),
    automaticUpdates: booleanValue(record.automaticUpdates),
    telemetry: booleanValue(record.telemetry),
    backgroundService: booleanValue(record.backgroundService),
    signedBuild: booleanValue(record.signedBuild),
    startupRecovery: parseStartupRecovery(record.startupRecovery),
  };
}

export function parseStartupRecovery(value: unknown): StartupRecoveryReport {
  const record = exactRecord(value, [
    'state',
    'recoveredTasks',
    'quarantinedRecords',
    'capacityExceededRecords',
  ]);
  return {
    state: enumValue(record.state, STARTUP_RECOVERY_STATES),
    recoveredTasks: safeInteger(record.recoveredTasks),
    quarantinedRecords: safeInteger(record.quarantinedRecords),
    capacityExceededRecords: safeInteger(record.capacityExceededRecords),
  };
}

export function parseSelection(value: unknown): SelectionSummary {
  const record = exactRecord(value, ['token', 'kind', 'displayName', 'size', 'count']);
  return {
    token: uuidValue(record.token),
    kind: enumValue(record.kind, SELECTION_KINDS),
    displayName: boundedString(record.displayName, 500),
    size: record.size === null ? null : safeInteger(record.size),
    count: safeInteger(record.count),
  };
}

export function parsePlan(value: unknown): ProcessingPlan {
  const record = exactRecord(value, [
    'totalBytes',
    'sliceSize',
    'sliceCount',
    'requiredFreeBytes',
    'minimumRequiredBytes',
    'recommendedFreeBytes',
    'availableFreeBytes',
    'temporaryBytes',
    'recoveryOverheadBytes',
    'expectedOutputCount',
  ]);
  return {
    totalBytes: safeInteger(record.totalBytes),
    sliceSize: safeInteger(record.sliceSize),
    sliceCount: safeInteger(record.sliceCount),
    requiredFreeBytes: safeInteger(record.requiredFreeBytes),
    minimumRequiredBytes: safeInteger(record.minimumRequiredBytes),
    recommendedFreeBytes: safeInteger(record.recommendedFreeBytes),
    availableFreeBytes: safeInteger(record.availableFreeBytes),
    temporaryBytes: safeInteger(record.temporaryBytes),
    recoveryOverheadBytes: safeInteger(record.recoveryOverheadBytes),
    expectedOutputCount: safeInteger(record.expectedOutputCount),
  };
}

export function parsePreflight(value: unknown): PreflightResult {
  const record = exactRecord(value, [
    'state',
    'checkedAt',
    'minimumRequiredBytes',
    'recommendedFreeBytes',
    'availableFreeBytes',
    'temporaryBytes',
    'recoveryOverheadBytes',
    'expectedOutputCount',
    'warnings',
    'conflicts',
  ]);
  return {
    state: enumValue(record.state, PREFLIGHT_STATES),
    checkedAt: timestampValue(record.checkedAt),
    minimumRequiredBytes: safeInteger(record.minimumRequiredBytes),
    recommendedFreeBytes: safeInteger(record.recommendedFreeBytes),
    availableFreeBytes: safeInteger(record.availableFreeBytes),
    temporaryBytes: safeInteger(record.temporaryBytes),
    recoveryOverheadBytes: safeInteger(record.recoveryOverheadBytes),
    expectedOutputCount: safeInteger(record.expectedOutputCount),
    warnings: parseBoundedArray(record.warnings, 20, parsePreflightWarning),
    conflicts: parseBoundedArray(record.conflicts, 20, parseConflict),
  };
}

function parsePreflightWarning(value: unknown): PreflightWarning {
  const record = exactRecord(value, ['code', 'message']);
  return {
    code: boundedString(record.code, 80),
    message: boundedString(record.message, 500),
  };
}

function parseConflict(value: unknown): TaskConflict {
  const record = exactRecord(value, [
    'conflictingTaskId',
    'class',
    'conflictType',
    'affectedResource',
    'recommendedAction',
  ]);
  return {
    conflictingTaskId: boundedString(record.conflictingTaskId, 80),
    class: enumValue(record.class, CONFLICT_CLASSES),
    conflictType: enumValue(record.conflictType, CONFLICT_TYPES),
    affectedResource: boundedString(record.affectedResource, 500),
    recommendedAction: boundedString(record.recommendedAction, 500),
  };
}

export function parseInspection(value: unknown): InspectionSummary {
  const record = exactRecord(value, [
    'packageId',
    'formatVersion',
    'originalFilename',
    'originalSize',
    'originalSha256',
    'expectedSliceCount',
    'foundSliceCount',
    'missing',
    'corrupted',
    'unexpected',
    'verified',
  ]);
  return {
    packageId: uuidValue(record.packageId),
    formatVersion: boundedString(record.formatVersion, 32),
    originalFilename: boundedString(record.originalFilename, 500),
    originalSize: safeInteger(record.originalSize),
    originalSha256: hashValue(record.originalSha256),
    expectedSliceCount: safeInteger(record.expectedSliceCount),
    foundSliceCount: safeInteger(record.foundSliceCount),
    missing: parseStringArray(record.missing, 50_000),
    corrupted: parseStringArray(record.corrupted, 50_000),
    unexpected: parseStringArray(record.unexpected, MAX_UNEXPECTED_SLICE_DIAGNOSTICS),
    verified: booleanValue(record.verified),
  };
}

export function parseTask(value: unknown): TaskSnapshot {
  const record = exactRecord(value, [
    'id',
    'revision',
    'operation',
    'applicationVersion',
    'formatVersion',
    'priority',
    'queueOrder',
    'queuePosition',
    'displayName',
    'destinationName',
    'plan',
    'preflight',
    'progress',
    'status',
    'failure',
    'failureHistory',
    'result',
    'attemptCount',
    'startedAt',
    'finishedAt',
    'durationMs',
    'recoveryEligible',
    'createdAt',
    'updatedAt',
  ]);
  return {
    id: uuidValue(record.id),
    revision: safeInteger(record.revision),
    operation: enumValue(record.operation, TASK_OPERATIONS),
    applicationVersion: boundedString(record.applicationVersion, 64),
    formatVersion: boundedString(record.formatVersion, 32),
    priority: enumValue(record.priority, TASK_PRIORITIES),
    queueOrder: safeInteger(record.queueOrder),
    queuePosition: nullableSafeInteger(record.queuePosition),
    displayName: boundedString(record.displayName, 500),
    destinationName:
      record.destinationName === null ? null : boundedString(record.destinationName, 500),
    plan: parsePlan(record.plan),
    preflight: record.preflight === null ? null : parsePreflight(record.preflight),
    progress: parseProgress(record.progress),
    status: enumValue(record.status, TASK_STATUSES),
    failure: record.failure === null ? null : parseFailure(record.failure),
    failureHistory: parseBoundedArray(record.failureHistory, 10, parseFailure),
    result: record.result === null ? null : parseResult(record.result),
    attemptCount: safeInteger(record.attemptCount),
    startedAt: nullableTimestamp(record.startedAt),
    finishedAt: nullableTimestamp(record.finishedAt),
    durationMs: nullableSafeInteger(record.durationMs),
    recoveryEligible: booleanValue(record.recoveryEligible),
    createdAt: timestampValue(record.createdAt),
    updatedAt: timestampValue(record.updatedAt),
  };
}

export function parseSettings(value: unknown): DesktopSettings {
  const record = exactRecord(value, [
    'defaultSliceSize',
    'confirmDestructiveActions',
    'reduceMotion',
    'maximumTerminalHistory',
    'terminalHistoryDays',
  ]);
  const defaultSliceSize = safeInteger(record.defaultSliceSize);
  if (defaultSliceSize < 1) {
    throw new Error('Desktop IPC returned an invalid Slice size.');
  }
  return {
    defaultSliceSize,
    confirmDestructiveActions: booleanValue(record.confirmDestructiveActions),
    reduceMotion: booleanValue(record.reduceMotion),
    maximumTerminalHistory: positiveSafeInteger(record.maximumTerminalHistory),
    terminalHistoryDays: positiveSafeInteger(record.terminalHistoryDays),
  };
}

export function parseStorageSummary(value: unknown): StorageSummary {
  const record = exactRecord(value, [
    'databaseBytes',
    'activeTasks',
    'nonterminalTasks',
    'terminalHistoryTasks',
    'quarantinedRecords',
    'incompleteOutputReferences',
    'diagnosticBundleCount',
    'maximumTerminalHistory',
    'terminalHistoryDays',
  ]);
  return {
    databaseBytes: safeInteger(record.databaseBytes),
    activeTasks: safeInteger(record.activeTasks),
    nonterminalTasks: safeInteger(record.nonterminalTasks),
    terminalHistoryTasks: safeInteger(record.terminalHistoryTasks),
    quarantinedRecords: safeInteger(record.quarantinedRecords),
    incompleteOutputReferences: safeInteger(record.incompleteOutputReferences),
    diagnosticBundleCount: safeInteger(record.diagnosticBundleCount),
    maximumTerminalHistory: positiveSafeInteger(record.maximumTerminalHistory),
    terminalHistoryDays: positiveSafeInteger(record.terminalHistoryDays),
  };
}

export function parseExportResult(value: unknown): ExportResult {
  const record = exactRecord(value, [
    'displayName',
    'bytesWritten',
    'kind',
    'revealToken',
  ]);
  return {
    displayName: boundedString(record.displayName, 500),
    bytesWritten: safeInteger(record.bytesWritten),
    kind: boundedString(record.kind, 80),
    revealToken: uuidValue(record.revealToken),
  };
}

function parseNullableSelection(value: unknown): SelectionSummary | null {
  return value === null ? null : parseSelection(value);
}

function parseProgress(value: unknown): TaskProgress {
  const record = exactRecord(value, [
    'bytesProcessed',
    'totalBytes',
    'currentSlice',
    'sliceCount',
    'stage',
  ]);
  return {
    bytesProcessed: safeInteger(record.bytesProcessed),
    totalBytes: safeInteger(record.totalBytes),
    currentSlice: safeInteger(record.currentSlice),
    sliceCount: safeInteger(record.sliceCount),
    stage: boundedString(record.stage, 500),
  };
}

function parseFailure(value: unknown): TaskFailure {
  const record = exactRecord(value, [
    'code',
    'message',
    'technicalMessage',
    'category',
    'retryable',
    'recoveryAction',
    'occurredAt',
    'attempt',
  ]);
  return {
    code: boundedString(record.code, 80),
    message: boundedString(record.message, 2_000),
    technicalMessage: boundedString(record.technicalMessage, 2_000),
    category: enumValue(record.category, ERROR_CATEGORIES),
    retryable: booleanValue(record.retryable),
    recoveryAction: enumValue(record.recoveryAction, RECOVERY_ACTIONS),
    occurredAt: timestampValue(record.occurredAt),
    attempt: safeInteger(record.attempt),
  };
}

function parseResult(value: unknown): TaskResult {
  const record = objectValue(value);
  if (record.type === 'split') {
    const split = exactRecord(record, ['type', 'manifestFilename', 'sourceSha256']);
    return {
      type: 'split',
      manifestFilename: boundedString(split.manifestFilename, 500),
      sourceSha256: hashValue(split.sourceSha256),
    };
  }
  if (record.type === 'merge') {
    const merge = exactRecord(record, ['type', 'outputFilename', 'outputSha256']);
    return {
      type: 'merge',
      outputFilename: boundedString(merge.outputFilename, 500),
      outputSha256: hashValue(merge.outputSha256),
    };
  }
  if (record.type === 'inspection') {
    const inspection = exactRecord(record, ['type', 'inspection']);
    return { type: 'inspection', inspection: parseInspection(inspection.inspection) };
  }
  throw new Error('Desktop IPC returned an unknown task result.');
}

function parseCommandError(value: unknown): CommandError {
  const record = exactRecord(value, [
    'code',
    'message',
    'retryable',
    'recoveryAction',
    'conflict',
  ]);
  return {
    code: boundedString(record.code, 80),
    message: boundedString(record.message, 2_000),
    retryable: booleanValue(record.retryable),
    recoveryAction:
      record.recoveryAction === null
        ? null
        : boundedString(record.recoveryAction, 80),
    conflict: record.conflict === null ? null : parseConflict(record.conflict),
  };
}

function parseStringArray(value: unknown, maximum: number): string[] {
  if (!Array.isArray(value) || value.length > maximum) {
    throw new Error('Desktop IPC returned an invalid array.');
  }
  return value.map((entry) => boundedString(entry, 500));
}

function parseBoundedArray<T>(
  value: unknown,
  maximum: number,
  parser: (entry: unknown) => T,
): T[] {
  if (!Array.isArray(value) || value.length > maximum) {
    throw new Error('Desktop IPC returned an invalid bounded array.');
  }
  return value.map(parser);
}

function exactRecord(value: unknown, keys: readonly string[]): Record<string, unknown> {
  const record = objectValue(value);
  const actual = Object.keys(record);
  if (actual.length !== keys.length || actual.some((key) => !keys.includes(key))) {
    throw new Error('Desktop IPC returned fields outside the expected schema.');
  }
  return record;
}

function objectValue(value: unknown): Record<string, unknown> {
  if (value === null || typeof value !== 'object' || Array.isArray(value)) {
    throw new Error('Desktop IPC returned a non-object value.');
  }
  return value as Record<string, unknown>;
}

function boundedString(value: unknown, maximum: number): string {
  const result = stringValue(value);
  if (result.length > maximum) {
    throw new Error('Desktop IPC returned an overlong string.');
  }
  return result;
}

function stringValue(value: unknown): string {
  if (typeof value !== 'string') {
    throw new Error('Desktop IPC returned a non-string value.');
  }
  return value;
}

function booleanValue(value: unknown): boolean {
  if (typeof value !== 'boolean') {
    throw new Error('Desktop IPC returned a non-boolean value.');
  }
  return value;
}

function safeInteger(value: unknown): number {
  if (typeof value !== 'number' || !Number.isSafeInteger(value) || value < 0) {
    throw new Error('Desktop IPC returned an unsafe numeric value.');
  }
  return value;
}

function positiveSafeInteger(value: unknown): number {
  const result = safeInteger(value);
  if (result < 1) {
    throw new Error('Desktop IPC returned a non-positive numeric value.');
  }
  return result;
}

function nullableSafeInteger(value: unknown): number | null {
  return value === null ? null : safeInteger(value);
}

function enumValue<const T extends readonly string[]>(value: unknown, choices: T): T[number] {
  if (typeof value !== 'string' || !choices.includes(value)) {
    throw new Error('Desktop IPC returned an unsupported enum value.');
  }
  return value;
}

function uuidValue(value: unknown): string {
  const result = stringValue(value);
  if (!/^[0-9a-f]{8}-[0-9a-f]{4}-[1-8][0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/iu.test(result)) {
    throw new Error('Desktop IPC returned an invalid UUID.');
  }
  return result;
}

function hashValue(value: unknown): string {
  const result = stringValue(value);
  if (!/^[0-9a-f]{64}$/u.test(result)) {
    throw new Error('Desktop IPC returned an invalid SHA-256 value.');
  }
  return result;
}

function timestampValue(value: unknown): string {
  const result = boundedString(value, 64);
  if (Number.isNaN(Date.parse(result))) {
    throw new Error('Desktop IPC returned an invalid timestamp.');
  }
  return result;
}

function nullableTimestamp(value: unknown): string | null {
  return value === null ? null : timestampValue(value);
}

function parseVoid(value: unknown): void {
  if (value !== null) {
    throw new Error('Desktop IPC returned data for a void command.');
  }
}
