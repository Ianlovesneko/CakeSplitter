import { useEffect, useMemo, useState, type ReactNode } from 'react';
import { ProgressMeter, StatusBadge } from '@cakesplitter/ui';
import {
  canClearTaskState,
  installDesktopListeners,
  reconcileTaskSnapshots,
  type RecoveryDisplayState,
} from './bootstrap';

import {
  chooseManifestFile,
  chooseOutputFile,
  chooseOutputFolder,
  choosePackageFolder,
  chooseSliceFiles,
  chooseSourceFile,
  clearAllTasks,
  clearCompletedHistory,
  clearFailedHistory,
  clearQuarantine,
  controlTask,
  enqueueInspect,
  enqueueMerge,
  enqueueSplit,
  enqueueVerify,
  errorMessage,
  exportDiagnosticBundle,
  exportReceipt,
  getRuntimeInfo,
  getSettings,
  getStorageSummary,
  listTasks,
  onCloseRequested,
  onNativeDrop,
  onNativeDropError,
  onTaskUpdate,
  planSplit,
  preflightMerge,
  preflightSplit,
  prepareAppClose,
  previewMerge,
  reorderTask,
  removeTask,
  revealExport,
  setTaskPriority,
  updateSettings,
  MAX_RENDERED_PACKAGE_DIAGNOSTIC_ROWS,
  type DesktopSettings,
  type ExportResult,
  type InspectionSummary,
  type PreflightResult,
  type ProcessingPlan,
  type RecoveryAction,
  type RuntimeInfo,
  type SelectionSummary,
  type StorageSummary,
  type TaskPriority,
  type TaskSnapshot,
  type TaskStatus,
} from './ipc';

type Workspace = 'split' | 'merge' | 'inspect' | 'tasks' | 'storage' | 'settings' | 'about';
type SizeMode = 'size' | 'count';

const MIB = 1024 * 1024;
const GIB = 1024 * MIB;
const PRESETS = [64 * MIB, 256 * MIB, 512 * MIB, GIB];

const WORKSPACES: Array<{ id: Workspace; label: string }> = [
  { id: 'split', label: 'Split' },
  { id: 'merge', label: 'Merge' },
  { id: 'inspect', label: 'Inspect' },
  { id: 'tasks', label: 'Tasks' },
  { id: 'storage', label: 'Storage' },
  { id: 'settings', label: 'Settings' },
  { id: 'about', label: 'About' },
];

export function App() {
  const [workspace, setWorkspace] = useState<Workspace>('split');
  const [runtime, setRuntime] = useState<RuntimeInfo | null>(null);
  const [settings, setSettings] = useState<DesktopSettings>({
    defaultSliceSize: 512 * MIB,
    confirmDestructiveActions: true,
    reduceMotion: false,
    maximumTerminalHistory: 500,
    terminalHistoryDays: 90,
  });
  const [tasks, setTasks] = useState<TaskSnapshot[]>([]);
  const [source, setSource] = useState<SelectionSummary | null>(null);
  const [splitOutput, setSplitOutput] = useState<SelectionSummary | null>(null);
  const [packageSelection, setPackageSelection] = useState<SelectionSummary | null>(null);
  const [mergeOutput, setMergeOutput] = useState<SelectionSummary | null>(null);
  const [inspection, setInspection] = useState<InspectionSummary | null>(null);
  const [plan, setPlan] = useState<ProcessingPlan | null>(null);
  const [splitPreflight, setSplitPreflight] = useState<PreflightResult | null>(null);
  const [mergePreflight, setMergePreflight] = useState<PreflightResult | null>(null);
  const [storage, setStorage] = useState<StorageSummary | null>(null);
  const [lastExport, setLastExport] = useState<ExportResult | null>(null);
  const [newTaskPriority, setNewTaskPriority] = useState<TaskPriority>('normal');
  const [sizeMode, setSizeMode] = useState<SizeMode>('size');
  const [sliceSize, setSliceSize] = useState(512 * MIB);
  const [sliceCount, setSliceCount] = useState(4);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string>('Ready for local file work.');
  const [closeTaskIds, setCloseTaskIds] = useState<string[] | null>(null);
  const [taskSnapshotUnavailable, setTaskSnapshotUnavailable] = useState(false);

  const upsertTask = (task: TaskSnapshot) => {
    setTasks((current) => reconcileTaskSnapshots(current, [task]));
    if (
      task.status === 'completed' &&
      task.result?.type === 'inspection'
    ) {
      setInspection(task.result.inspection);
    }
  };

  useEffect(() => {
    let disposed = false;
    const unlisten: Array<() => void> = [];
    void (async () => {
      const registrations = await installDesktopListeners([
        () => onTaskUpdate(upsertTask, setError),
        () => onNativeDrop((selection) => {
            setError(null);
            if (selection.kind === 'sourceFile') {
              setSource(selection);
              setWorkspace('split');
              setNotice(`${selection.displayName} is ready to plan.`);
            } else if (
              selection.kind === 'manifestFile' ||
              selection.kind === 'packageFolder' ||
              selection.kind === 'sliceFiles'
            ) {
              setPackageSelection(selection);
              setWorkspace('merge');
              setNotice(`${selection.displayName} is ready to inspect.`);
            }
          }, setError),
        () => onNativeDropError((dropError) => setError(dropError.message), setError),
        () => onCloseRequested(setCloseTaskIds, setError),
      ]);
      if (disposed) {
        for (const stop of registrations.unlisten) stop();
        return;
      }
      unlisten.push(...registrations.unlisten);
      for (const registrationError of registrations.errors) setError(errorMessage(registrationError));

      const [runtimeResult, taskResult, settingsResult, storageResult] = await Promise.allSettled([
        getRuntimeInfo(),
        listTasks(),
        getSettings(),
        getStorageSummary(),
      ]);
      if (disposed) return;
      if (runtimeResult.status === 'fulfilled') {
        setRuntime(runtimeResult.value);
      } else {
        setError(errorMessage(runtimeResult.reason));
      }
      if (taskResult.status === 'fulfilled') {
        setTasks((current) => reconcileTaskSnapshots(current, taskResult.value));
        setTaskSnapshotUnavailable(false);
      } else {
        setTaskSnapshotUnavailable(true);
        setError(errorMessage(taskResult.reason));
      }
      if (settingsResult.status === 'fulfilled') {
        setSettings(settingsResult.value);
        setSliceSize(settingsResult.value.defaultSliceSize);
      } else {
        setError(errorMessage(settingsResult.reason));
      }
      if (storageResult.status === 'fulfilled') {
        setStorage(storageResult.value);
      } else {
        setError(errorMessage(storageResult.reason));
      }
    })().catch((cause) => {
      if (!disposed) setError(errorMessage(cause));
    });
    return () => {
      disposed = true;
      for (const stop of unlisten) stop();
    };
  }, []);

  const effectiveSliceSize = useMemo(() => {
    if (sizeMode === 'size') return sliceSize;
    if (source?.size === null || source?.size === undefined || sliceCount < 1) return 0;
    return Math.max(1, Math.ceil(source.size / sliceCount));
  }, [sizeMode, sliceSize, sliceCount, source]);

  async function run(action: () => Promise<void>) {
    setBusy(true);
    setError(null);
    try {
      await action();
    } catch (cause) {
      setError(errorMessage(cause));
    } finally {
      setBusy(false);
    }
  }

  function requireSelection(value: SelectionSummary | null, label: string): SelectionSummary {
    if (value === null) throw new Error(`Select ${label} first.`);
    return value;
  }

  async function createPlan() {
    const selectedSource = requireSelection(source, 'a source Cake');
    const selectedOutput = requireSelection(splitOutput, 'an output folder');
    if (effectiveSliceSize < 1) throw new Error('Choose a valid Slice size or count.');
    const [next, checked] = await Promise.all([
      planSplit(selectedSource.token, selectedOutput.token, effectiveSliceSize),
      preflightSplit(selectedSource.token, selectedOutput.token, effectiveSliceSize),
    ]);
    setPlan(next);
    setSplitPreflight(checked);
    setNotice(
      checked.state === 'ready'
        ? `Plan ready: ${next.sliceCount.toLocaleString()} Slices.`
        : `Preflight ${checked.state}: review the checks before starting.`,
    );
  }

  async function startSplit() {
    const selectedSource = requireSelection(source, 'a source Cake');
    const selectedOutput = requireSelection(splitOutput, 'an output folder');
    if (effectiveSliceSize < 1) throw new Error('Choose a valid Slice size or count.');
    const task = await enqueueSplit(
      selectedSource.token,
      selectedOutput.token,
      effectiveSliceSize,
      newTaskPriority,
    );
    upsertTask(task);
    setWorkspace('tasks');
    setNotice('Split queued. The original source will remain unchanged.');
  }

  async function previewSelectedPackage() {
    const selected = requireSelection(packageSelection, 'a Cake Package');
    const next = await previewMerge(selected.token);
    setInspection(next);
    setNotice(
      next.missing.length + next.corrupted.length + next.unexpected.length === 0
        ? 'Package structure is ready for full verification.'
        : 'Package inspection found items that need attention.',
    );
  }

  async function startMerge() {
    const selectedPackage = requireSelection(packageSelection, 'a Cake Package');
    const selectedOutput = requireSelection(mergeOutput, 'a rebuilt output file');
    const checked = await preflightMerge(selectedPackage.token, selectedOutput.token);
    setMergePreflight(checked);
    const task = await enqueueMerge(
      selectedPackage.token,
      selectedOutput.token,
      newTaskPriority,
    );
    upsertTask(task);
    setWorkspace('tasks');
    setNotice('Merge queued. Output will publish only after full verification.');
  }

  async function startInspection(verify: boolean) {
    const selected = requireSelection(packageSelection, 'a Cake Package');
    const task = verify
      ? await enqueueVerify(selected.token, newTaskPriority)
      : await enqueueInspect(selected.token, false, newTaskPriority);
    upsertTask(task);
    setWorkspace('tasks');
    setNotice(verify ? 'Full verification queued.' : 'Inspection queued.');
  }

  async function pickPackage(kind: 'manifest' | 'folder' | 'slices') {
    const selected =
      kind === 'manifest'
        ? await chooseManifestFile()
        : kind === 'folder'
          ? await choosePackageFolder()
          : await chooseSliceFiles();
    if (selected !== null) {
      setPackageSelection(selected);
      setMergeOutput(null);
      setInspection(null);
      setMergePreflight(null);
    }
  }

  async function handleTaskControl(task: TaskSnapshot, command: 'pause_task' | 'resume_task' | 'cancel_task' | 'retry_task') {
    const updated = await controlTask(command, task.id);
    upsertTask(updated);
  }

  async function refreshStorage() {
    setStorage(await getStorageSummary());
  }

  async function handlePriority(task: TaskSnapshot, priority: TaskPriority) {
    upsertTask(await setTaskPriority(task.id, priority));
  }

  async function handleReorder(task: TaskSnapshot, direction: 'earlier' | 'later') {
    const changed = await reorderTask(task.id, direction);
    setTasks((current) => reconcileTaskSnapshots(current, changed));
  }

  async function clearTerminalHistory(kind: 'completed' | 'failed') {
    if (
      settings.confirmDestructiveActions &&
      !window.confirm(`Clear all ${kind} task history? Nonterminal work is preserved.`)
    ) return;
    const removed = kind === 'completed'
      ? await clearCompletedHistory()
      : await clearFailedHistory();
    setTasks(await listTasks());
    await refreshStorage();
    setNotice(`${removed} ${kind} task record${removed === 1 ? '' : 's'} cleared.`);
  }

  async function clearLocalQuarantine() {
    if (
      settings.confirmDestructiveActions &&
      !window.confirm('Clear quarantined task metadata? This cannot restore those tasks.')
    ) return;
    const removed = await clearQuarantine();
    await refreshStorage();
    setNotice(`${removed} quarantined record${removed === 1 ? '' : 's'} cleared.`);
  }

  async function saveReceipt(
    task: TaskSnapshot,
    format: 'markdown' | 'json',
    includePathDetail: boolean,
  ) {
    if (
      includePathDetail &&
      !window.confirm('Include additional local path detail? The receipt stays local, but may reveal private folder structure if shared.')
    ) return;
    const suffix = format === 'markdown' ? 'md' : 'json';
    const output = await chooseOutputFile(`cakesplitter-${task.operation}-${task.id.slice(0, 8)}.${suffix}`);
    if (output === null) return;
    const exported = await exportReceipt(task.id, output.token, format, includePathDetail);
    setLastExport(exported);
    setNotice(`Receipt exported locally as ${exported.displayName}.`);
  }

  async function saveDiagnostics() {
    const output = await chooseOutputFolder();
    if (output === null) return;
    const exported = await exportDiagnosticBundle(output.token);
    setLastExport(exported);
    await refreshStorage();
    setNotice(`Diagnostic bundle exported locally as ${exported.displayName}.`);
  }

  async function copyExportName() {
    if (lastExport === null) return;
    await navigator.clipboard.writeText(lastExport.displayName);
    setNotice('Export filename copied to the clipboard.');
  }

  async function clearHistory() {
    if (
      settings.confirmDestructiveActions &&
      !window.confirm('Clear all task records? Active tasks will be cancelled and stale writes fenced.')
    ) {
      return;
    }
    await clearAllTasks();
    setTasks([]);
    setTaskSnapshotUnavailable(false);
    setRuntime((current) => current === null ? null : {
      ...current,
      startupRecovery: {
        state: 'ready',
        recoveredTasks: 0,
        quarantinedRecords: 0,
        capacityExceededRecords: 0,
      },
    });
    setNotice('Task history cleared safely.');
    await refreshStorage();
  }

  async function saveSettings(next: DesktopSettings) {
    const saved = await updateSettings(next);
    setSettings(saved);
    setSliceSize(saved.defaultSliceSize);
    setNotice('Settings saved in local application data.');
    await refreshStorage();
  }

  return (
    <div className={settings.reduceMotion ? 'app app--reduced-motion' : 'app'}>
      <a className="skip-link" href="#workspace">Skip to workspace</a>
      <header className="topbar">
        <div className="brand" aria-label="CakeSplitter Desktop">
          <span className="brand__mark" aria-hidden="true">C</span>
          <span>
            <strong>CakeSplitter</strong>
            <small>Desktop</small>
          </span>
        </div>
        <div className="trust-strip">
          <span className="trust-dot" aria-hidden="true" />
          Local only · no uploads · no automatic updates
        </div>
        <span className="version">v{runtime?.applicationVersion ?? '0.6.0'}</span>
      </header>

      <div className="shell">
        <nav className="sidebar" aria-label="Primary workspaces">
          {WORKSPACES.map((item) => (
            <button
              key={item.id}
              className={workspace === item.id ? 'nav-button nav-button--active' : 'nav-button'}
              type="button"
              aria-current={workspace === item.id ? 'page' : undefined}
              onClick={() => setWorkspace(item.id)}
            >
              <span className="nav-icon" aria-hidden="true">{item.label.slice(0, 1)}</span>
              {item.label}
              {item.id === 'tasks' && tasks.length > 0 ? (
                <span className="nav-count">{tasks.length}</span>
              ) : null}
            </button>
          ))}
          <div className="sidebar__foot">
            <strong>Format {runtime?.formatVersion ?? '1.0'}</strong>
            <span>Windows x64 preview</span>
          </div>
        </nav>

        <main id="workspace" className="workspace" tabIndex={-1}>
          <div className="workspace__status" role="status" aria-live="polite">{notice}</div>
          {error !== null ? (
            <div className="alert alert--danger" role="alert">
              <strong>Stopped safely</strong>
              <span>{error}</span>
              <button type="button" onClick={() => setError(null)}>Dismiss</button>
            </div>
          ) : null}

          {workspace === 'split' ? (
            <SplitWorkspace
              source={source}
              output={splitOutput}
              plan={plan}
              preflight={splitPreflight}
              priority={newTaskPriority}
              sizeMode={sizeMode}
              sliceSize={sliceSize}
              sliceCount={sliceCount}
              effectiveSliceSize={effectiveSliceSize}
              busy={busy}
              onSizeMode={setSizeMode}
              onSliceSize={setSliceSize}
              onSliceCount={setSliceCount}
              onPriority={setNewTaskPriority}
              onSource={() => run(async () => {
                const selected = await chooseSourceFile();
                if (selected !== null) {
                  setSource(selected);
                  setPlan(null);
                  setSplitPreflight(null);
                }
              })}
              onOutput={() => run(async () => {
                const selected = await chooseOutputFolder();
                if (selected !== null) {
                  setSplitOutput(selected);
                  setPlan(null);
                  setSplitPreflight(null);
                }
              })}
              onPlan={() => run(createPlan)}
              onStart={() => run(startSplit)}
            />
          ) : null}

          {workspace === 'merge' ? (
            <MergeWorkspace
              packageSelection={packageSelection}
              output={mergeOutput}
              inspection={inspection}
              preflight={mergePreflight}
              priority={newTaskPriority}
              busy={busy}
              onPriority={setNewTaskPriority}
              onPickPackage={(kind) => run(() => pickPackage(kind))}
              onPreview={() => run(previewSelectedPackage)}
              onOutput={() => run(async () => {
                const suggested = inspection?.originalFilename ?? 'rebuilt-cake.bin';
                const selected = await chooseOutputFile(suggested);
                if (selected !== null) {
                  setMergeOutput(selected);
                  const selectedPackage = requireSelection(packageSelection, 'a Cake Package');
                  setMergePreflight(await preflightMerge(selectedPackage.token, selected.token));
                }
              })}
              onStart={() => run(startMerge)}
            />
          ) : null}

          {workspace === 'inspect' ? (
            <InspectWorkspace
              packageSelection={packageSelection}
              inspection={inspection}
              priority={newTaskPriority}
              busy={busy}
              onPriority={setNewTaskPriority}
              onPickPackage={(kind) => run(() => pickPackage(kind))}
              onInspect={() => run(() => startInspection(false))}
              onVerify={() => run(() => startInspection(true))}
            />
          ) : null}

          {workspace === 'tasks' ? (
            <TasksWorkspace
              tasks={tasks}
              recoveryState={taskSnapshotUnavailable ? 'snapshot-unavailable' : (runtime?.startupRecovery.state ?? 'ready')}
              busy={busy}
              onControl={(task, command) => run(() => handleTaskControl(task, command))}
              onPriority={(task, priority) => run(() => handlePriority(task, priority))}
              onReorder={(task, direction) => run(() => handleReorder(task, direction))}
              onReceipt={(task, format, includePathDetail) => run(() => saveReceipt(task, format, includePathDetail))}
              onRemove={(task) => run(async () => {
                if (
                  settings.confirmDestructiveActions &&
                  !window.confirm(`Remove ${task.displayName} from task history?`)
                ) return;
                await removeTask(task.id);
                setTasks((current) => current.filter((candidate) => candidate.id !== task.id));
              })}
              onClearAll={() => run(clearHistory)}
              storage={storage}
            />
          ) : null}

          {workspace === 'storage' ? (
            <StorageWorkspace
              storage={storage}
              lastExport={lastExport}
              busy={busy}
              onRefresh={() => run(refreshStorage)}
              onClearCompleted={() => run(() => clearTerminalHistory('completed'))}
              onClearFailed={() => run(() => clearTerminalHistory('failed'))}
              onClearQuarantine={() => run(clearLocalQuarantine)}
              onDiagnostics={() => run(saveDiagnostics)}
              onReveal={() => run(async () => {
                if (lastExport !== null) await revealExport(lastExport.revealToken);
              })}
              onCopy={() => run(copyExportName)}
            />
          ) : null}

          {workspace === 'settings' ? (
            <SettingsWorkspace settings={settings} busy={busy} onSave={(next) => run(() => saveSettings(next))} />
          ) : null}

          {workspace === 'about' ? <AboutWorkspace runtime={runtime} /> : null}
        </main>
      </div>

      {closeTaskIds !== null ? (
        <div className="modal-backdrop" role="presentation">
          <section className="modal" role="dialog" aria-modal="true" aria-labelledby="close-title">
            <span className="eyebrow">Active local work</span>
            <h2 id="close-title">Close CakeSplitter?</h2>
            <p>
              {closeTaskIds.length} task{closeTaskIds.length === 1 ? ' is' : 's are'} active.
              You can keep working, cancel safely, or interrupt at the next safe checkpoint for restart recovery.
            </p>
            <div className="modal__actions">
              <button className="button button--ghost" type="button" autoFocus onClick={() => {
                setCloseTaskIds(null);
                void prepareAppClose('keepOpen');
              }}>Keep open</button>
              <button className="button button--danger" type="button" onClick={() => run(async () => {
                await prepareAppClose('cancelTasks');
                setCloseTaskIds(null);
                setNotice('Cancellation requested. Close again after tasks stop.');
              })}>Cancel tasks</button>
              <button className="button button--primary" type="button" onClick={() => run(async () => {
                await prepareAppClose('interruptAndExit');
              })}>Interrupt and exit</button>
            </div>
          </section>
        </div>
      ) : null}
    </div>
  );
}

function WorkspaceHeading({ eyebrow, title, children }: { eyebrow: string; title: string; children: ReactNode }) {
  return (
    <div className="workspace-heading">
      <span className="eyebrow">{eyebrow}</span>
      <h1>{title}</h1>
      <p>{children}</p>
    </div>
  );
}

function SplitWorkspace(props: {
  source: SelectionSummary | null;
  output: SelectionSummary | null;
  plan: ProcessingPlan | null;
  preflight: PreflightResult | null;
  priority: TaskPriority;
  sizeMode: SizeMode;
  sliceSize: number;
  sliceCount: number;
  effectiveSliceSize: number;
  busy: boolean;
  onSizeMode: (mode: SizeMode) => void;
  onSliceSize: (size: number) => void;
  onSliceCount: (count: number) => void;
  onPriority: (priority: TaskPriority) => void;
  onSource: () => void;
  onOutput: () => void;
  onPlan: () => void;
  onStart: () => void;
}) {
  return (
    <>
      <WorkspaceHeading eyebrow="Native streamed workflow" title="Split a Cake into verified Slices">
        Select one source file and an output folder. Rust streams the source, verifies stability, and publishes the Manifest last.
      </WorkspaceHeading>
      <div className="workflow-grid">
        <section className="panel panel--wide" aria-labelledby="split-source-title">
          <div className="panel__heading">
            <div><span className="step">01</span><h2 id="split-source-title">Source Cake</h2></div>
            <StatusBadge tone={props.source === null ? 'neutral' : 'success'}>{props.source === null ? 'Not selected' : 'Selected'}</StatusBadge>
          </div>
          <button className="drop-zone" type="button" onClick={props.onSource} disabled={props.busy}>
            <strong>{props.source?.displayName ?? 'Choose a file or drop it onto the window'}</strong>
            <span>{props.source?.size === null || props.source === null ? 'The selected file is never executed.' : formatBytes(props.source.size)}</span>
          </button>
        </section>

        <section className="panel" aria-labelledby="split-plan-title">
          <div className="panel__heading"><div><span className="step">02</span><h2 id="split-plan-title">Slice plan</h2></div></div>
          <div className="segmented" role="group" aria-label="Slice planning mode">
            <button type="button" className={props.sizeMode === 'size' ? 'selected' : ''} aria-pressed={props.sizeMode === 'size'} onClick={() => props.onSizeMode('size')}>Target size</button>
            <button type="button" className={props.sizeMode === 'count' ? 'selected' : ''} aria-pressed={props.sizeMode === 'count'} onClick={() => props.onSizeMode('count')}>Target count</button>
          </div>
          {props.sizeMode === 'size' ? (
            <>
              <div className="preset-row" aria-label="Common Slice sizes">
                {PRESETS.map((preset) => <button key={preset} type="button" className={props.sliceSize === preset ? 'chip chip--selected' : 'chip'} onClick={() => props.onSliceSize(preset)}>{formatBytes(preset)}</button>)}
              </div>
              <label className="field"><span>Custom Slice size (bytes)</span><input type="number" min="1" max={Number.MAX_SAFE_INTEGER} value={props.sliceSize} onChange={(event) => props.onSliceSize(Number(event.target.value))} /></label>
            </>
          ) : (
            <label className="field"><span>Target Slice count</span><input type="number" min="1" max="50000" value={props.sliceCount} onChange={(event) => props.onSliceCount(Number(event.target.value))} /><small>Calculated Slice size: {props.effectiveSliceSize > 0 ? formatBytes(props.effectiveSliceSize) : 'select a source'}</small></label>
          )}
        </section>

        <section className="panel" aria-labelledby="split-output-title">
          <div className="panel__heading"><div><span className="step">03</span><h2 id="split-output-title">Output folder</h2></div></div>
          <button className="selection-button" type="button" onClick={props.onOutput} disabled={props.busy}>
            <span>{props.output?.displayName ?? 'Choose output folder'}</span><b aria-hidden="true">→</b>
          </button>
          <p className="fine-print">Existing outputs and late collisions fail closed. Task-owned files use an explicit <code>.partial</code> name.</p>
        </section>

        <section className="panel panel--summary" aria-labelledby="split-summary-title">
          <div className="panel__heading"><div><span className="step">04</span><h2 id="split-summary-title">Review and start</h2></div></div>
          {props.plan === null ? <p>Preview the plan to verify Slice count and required free space.</p> : (
            <dl className="metrics"><div><dt>Source</dt><dd>{formatBytes(props.plan.totalBytes)}</dd></div><div><dt>Slice size</dt><dd>{formatBytes(props.plan.sliceSize)}</dd></div><div><dt>Slice count</dt><dd>{props.plan.sliceCount.toLocaleString()}</dd></div><div><dt>Minimum free</dt><dd>{formatBytes(props.plan.minimumRequiredBytes)}</dd></div><div><dt>Recommended free</dt><dd>{formatBytes(props.plan.recommendedFreeBytes)}</dd></div><div><dt>Expected outputs</dt><dd>{props.plan.expectedOutputCount.toLocaleString()}</dd></div></dl>
          )}
          <PriorityField value={props.priority} onChange={props.onPriority} disabled={props.busy} inverted />
          {props.preflight === null ? null : <PreflightView preflight={props.preflight} />}
          <div className="actions"><button className="button button--ghost" type="button" onClick={props.onPlan} disabled={props.busy}>Preview plan</button><button className="button button--primary" type="button" onClick={props.onStart} disabled={props.busy || props.source === null || props.output === null || props.preflight?.state === 'blocked' || props.preflight?.state === 'reselection-required'}>Start Split</button></div>
        </section>
      </div>
    </>
  );
}

function MergeWorkspace(props: {
  packageSelection: SelectionSummary | null;
  output: SelectionSummary | null;
  inspection: InspectionSummary | null;
  preflight: PreflightResult | null;
  priority: TaskPriority;
  busy: boolean;
  onPriority: (priority: TaskPriority) => void;
  onPickPackage: (kind: 'manifest' | 'folder' | 'slices') => void;
  onPreview: () => void;
  onOutput: () => void;
  onStart: () => void;
}) {
  const ready = props.inspection !== null && props.inspection.missing.length === 0 && props.inspection.corrupted.length === 0 && props.inspection.unexpected.length === 0;
  return (
    <>
      <WorkspaceHeading eyebrow="Verified reconstruction" title="Merge a Cake Package">
        Package contents are inspected before streaming. The rebuilt Cake remains incomplete until its size and SHA-256 match.
      </WorkspaceHeading>
      <div className="workflow-grid">
        <section className="panel panel--wide">
          <div className="panel__heading"><div><span className="step">01</span><h2>Select package evidence</h2></div><StatusBadge tone={props.packageSelection === null ? 'neutral' : 'success'}>{props.packageSelection?.kind ?? 'Not selected'}</StatusBadge></div>
          <div className="button-row"><button className="button button--ghost" type="button" onClick={() => props.onPickPackage('manifest')} disabled={props.busy}>Manifest</button><button className="button button--ghost" type="button" onClick={() => props.onPickPackage('folder')} disabled={props.busy}>Package folder</button><button className="button button--ghost" type="button" onClick={() => props.onPickPackage('slices')} disabled={props.busy}>Slice files</button></div>
          <div className="selection-readout"><strong>{props.packageSelection?.displayName ?? 'No Cake Package selected'}</strong><span>Selections are represented by short-lived native tokens, not frontend paths.</span></div>
        </section>
        <section className="panel">
          <div className="panel__heading"><div><span className="step">02</span><h2>Package preview</h2></div>{props.inspection === null ? null : <StatusBadge tone={ready ? 'success' : 'danger'}>{ready ? 'Ready' : 'Not ready'}</StatusBadge>}</div>
          {props.inspection === null ? <p>Inspect structure before choosing the rebuilt output.</p> : <InspectionCompact inspection={props.inspection} />}
          <button className="button button--ghost" type="button" onClick={props.onPreview} disabled={props.busy || props.packageSelection === null}>Preview package</button>
        </section>
        <section className="panel">
          <div className="panel__heading"><div><span className="step">03</span><h2>Rebuilt output</h2></div></div>
          <button className="selection-button" type="button" onClick={props.onOutput} disabled={props.busy || props.packageSelection === null}><span>{props.output?.displayName ?? 'Choose a new output file'}</span><b aria-hidden="true">→</b></button>
          <p className="fine-print">The final filename must not already exist. Native no-replace publication protects late collisions.</p>
        </section>
        <section className="panel panel--summary">
          <h2>Ready to rebuild?</h2>
          <p>All Slices are revalidated and the final file is hashed before publication.</p>
          <PriorityField value={props.priority} onChange={props.onPriority} disabled={props.busy} inverted />
          {props.preflight === null ? null : <PreflightView preflight={props.preflight} />}
          <button className="button button--primary" type="button" onClick={props.onStart} disabled={props.busy || props.packageSelection === null || props.output === null || !ready || props.preflight?.state === 'blocked' || props.preflight?.state === 'reselection-required'}>Start Merge</button>
        </section>
      </div>
    </>
  );
}

function InspectWorkspace(props: {
  packageSelection: SelectionSummary | null;
  inspection: InspectionSummary | null;
  priority: TaskPriority;
  busy: boolean;
  onPriority: (priority: TaskPriority) => void;
  onPickPackage: (kind: 'manifest' | 'folder' | 'slices') => void;
  onInspect: () => void;
  onVerify: () => void;
}) {
  return (
    <>
      <WorkspaceHeading eyebrow="Evidence before action" title="Inspect or verify a Cake Package">
        Inspection reads structure and availability. Verify additionally hashes every Slice and produces a definitive readiness verdict.
      </WorkspaceHeading>
      <section className="panel panel--wide">
        <div className="button-row"><button className="button button--ghost" type="button" onClick={() => props.onPickPackage('manifest')}>Select Manifest</button><button className="button button--ghost" type="button" onClick={() => props.onPickPackage('folder')}>Select folder</button><button className="button button--ghost" type="button" onClick={() => props.onPickPackage('slices')}>Select Slices</button></div>
        <div className="selection-readout"><strong>{props.packageSelection?.displayName ?? 'No selection'}</strong><span>Unsupported or ambiguous packages fail without a success announcement.</span></div>
        <PriorityField value={props.priority} onChange={props.onPriority} disabled={props.busy} />
        <div className="actions"><button className="button button--ghost" type="button" onClick={props.onInspect} disabled={props.busy || props.packageSelection === null}>Inspect</button><button className="button button--primary" type="button" onClick={props.onVerify} disabled={props.busy || props.packageSelection === null}>Verify all Slices</button></div>
      </section>
      {props.inspection === null ? <EmptyState title="No inspection result yet">Select package evidence, then Inspect or Verify.</EmptyState> : <InspectionDetails inspection={props.inspection} />}
    </>
  );
}

function PriorityField({ value, onChange, disabled, inverted = false }: {
  value: TaskPriority;
  onChange: (priority: TaskPriority) => void;
  disabled: boolean;
  inverted?: boolean;
}) {
  return (
    <label className={inverted ? 'field priority-field priority-field--inverted' : 'field priority-field'}>
      <span>Task priority</span>
      <select value={value} onChange={(event) => onChange(event.target.value as TaskPriority)} disabled={disabled}>
        <option value="high">High</option>
        <option value="normal">Normal</option>
        <option value="low">Low</option>
      </select>
      <small>Priority affects queued order only. Fairness prevents starvation.</small>
    </label>
  );
}

function PreflightView({ preflight, compact = false }: { preflight: PreflightResult; compact?: boolean }) {
  const unsafe = preflight.state === 'blocked' || preflight.state === 'reselection-required';
  return (
    <section className={`${compact ? 'preflight preflight--compact' : 'preflight'} ${unsafe ? 'preflight--blocked' : ''}`} aria-label={`Preflight ${preflight.state}`}>
      <div className="preflight__heading"><strong>Preflight: {preflight.state}</strong><span>{new Date(preflight.checkedAt).toLocaleTimeString()}</span></div>
      <dl className="preflight__metrics"><div><dt>Available</dt><dd>{formatBytes(preflight.availableFreeBytes)}</dd></div><div><dt>Minimum</dt><dd>{formatBytes(preflight.minimumRequiredBytes)}</dd></div><div><dt>Recommended</dt><dd>{formatBytes(preflight.recommendedFreeBytes)}</dd></div><div><dt>Outputs</dt><dd>{preflight.expectedOutputCount.toLocaleString()}</dd></div></dl>
      {preflight.warnings.length === 0 ? null : <ul>{preflight.warnings.map((warning) => <li key={`${warning.code}-${warning.message}`}>{warning.message}</li>)}</ul>}
      {preflight.conflicts.length === 0 ? null : <details><summary>{preflight.conflicts.length} resource overlap{preflight.conflicts.length === 1 ? '' : 's'}</summary><ul>{preflight.conflicts.map((conflict) => <li key={`${conflict.conflictingTaskId}-${conflict.conflictType}`}><strong>{conflict.class}</strong>: {conflict.affectedResource}. {conflict.recommendedAction}</li>)}</ul></details>}
    </section>
  );
}

function StorageWorkspace(props: {
  storage: StorageSummary | null;
  lastExport: ExportResult | null;
  busy: boolean;
  onRefresh: () => void;
  onClearCompleted: () => void;
  onClearFailed: () => void;
  onClearQuarantine: () => void;
  onDiagnostics: () => void;
  onReveal: () => void;
  onCopy: () => void;
}) {
  return (
    <>
      <div className="workspace-heading workspace-heading--row"><div><span className="eyebrow">Bounded local state</span><h1>Storage and diagnostics</h1><p>Review durable task metadata, clean terminal history, and export redacted support evidence. Source bytes and Slice content are never included.</p></div><button className="button button--ghost" type="button" onClick={props.onRefresh} disabled={props.busy}>Refresh</button></div>
      {props.storage === null ? <EmptyState title="Storage summary unavailable">Refresh after the native runtime is ready.</EmptyState> : (
        <div className="workflow-grid">
          <section className="panel panel--wide"><h2>Local task state</h2><dl className="metrics metrics--compact"><div><dt>Database</dt><dd>{formatBytes(props.storage.databaseBytes)}</dd></div><div><dt>Nonterminal</dt><dd>{props.storage.nonterminalTasks}</dd></div><div><dt>Terminal history</dt><dd>{props.storage.terminalHistoryTasks}</dd></div><div><dt>Quarantine</dt><dd>{props.storage.quarantinedRecords}</dd></div><div><dt>Incomplete references</dt><dd>{props.storage.incompleteOutputReferences}</dd></div><div><dt>Diagnostic bundles</dt><dd>{props.storage.diagnosticBundleCount}</dd></div><div><dt>History limit</dt><dd>{props.storage.maximumTerminalHistory}</dd></div><div><dt>Retention</dt><dd>{props.storage.terminalHistoryDays} days</dd></div></dl></section>
          <section className="panel"><h2>History cleanup</h2><p>Only terminal history is removed. Active and queued tasks are never evicted to make room.</p><div className="button-row"><button className="button button--ghost" type="button" onClick={props.onClearCompleted} disabled={props.busy}>Clear completed</button><button className="button button--danger" type="button" onClick={props.onClearFailed} disabled={props.busy}>Clear failed</button><button className="button button--danger" type="button" onClick={props.onClearQuarantine} disabled={props.busy || props.storage.quarantinedRecords === 0}>Clear quarantine</button></div></section>
          <section className="panel"><h2>Redacted diagnostic bundle</h2><p>The bundle contains bounded summaries, capability state, limits, recent error categories, and privacy guidance. Paths, usernames, email addresses, credentials, hashes, identity records, Manifests, and file content are excluded or masked.</p><button className="button button--primary" type="button" onClick={props.onDiagnostics} disabled={props.busy}>Export diagnostics</button></section>
          {props.lastExport === null ? null : <section className="panel panel--wide export-success" role="status"><div><span className="eyebrow">Local export complete</span><h2>{props.lastExport.displayName}</h2><p>Saved locally · {formatBytes(props.lastExport.bytesWritten)}</p></div><div className="button-row"><button className="button button--ghost" type="button" onClick={props.onCopy} disabled={props.busy}>Copy filename</button><button className="button button--primary" type="button" onClick={props.onReveal} disabled={props.busy}>Show in Explorer</button></div></section>}
        </div>
      )}
    </>
  );
}

export function TasksWorkspace(props: {
  tasks: TaskSnapshot[];
  storage: StorageSummary | null;
  recoveryState: RecoveryDisplayState;
  busy: boolean;
  onControl: (task: TaskSnapshot, command: 'pause_task' | 'resume_task' | 'cancel_task' | 'retry_task') => void;
  onPriority: (task: TaskSnapshot, priority: TaskPriority) => void;
  onReorder: (task: TaskSnapshot, direction: 'earlier' | 'later') => void;
  onReceipt: (task: TaskSnapshot, format: 'markdown' | 'json', includePathDetail: boolean) => void;
  onRemove: (task: TaskSnapshot) => void;
  onClearAll: () => void;
}) {
  const active = props.tasks.find((task) => ['running', 'pausing', 'paused', 'resuming', 'cancelling'].includes(task.status));
  const next = props.tasks
    .filter((task) => task.status === 'queued')
    .sort((first, second) => (first.queuePosition ?? Number.MAX_SAFE_INTEGER) - (second.queuePosition ?? Number.MAX_SAFE_INTEGER))[0];
  const counts = countTaskStatuses(props.tasks);
  return (
    <>
      <div className="workspace-heading workspace-heading--row"><div><span className="eyebrow">One active disk task</span><h1>Tasks</h1><p>Queued work runs serially. Resume occurs at verified Slice boundaries.</p></div><button className="button button--danger" type="button" onClick={props.onClearAll} disabled={!canClearTaskState(props.busy, props.tasks.length, props.recoveryState)}>Clear All</button></div>
      <section className="dashboard-grid" aria-label="Task dashboard">
        <div><span>Active</span><strong>{counts.active}</strong><small>{active?.displayName ?? 'No active task'}</small></div>
        <div><span>Queued</span><strong>{counts.queued}</strong><small>{next === undefined ? 'Queue empty' : `Next: ${next.displayName}`}</small></div>
        <div><span>Needs attention</span><strong>{counts.attention}</strong><small>{props.storage?.quarantinedRecords ?? 0} quarantined</small></div>
        <div><span>History</span><strong>{counts.terminal}</strong><small>{props.storage === null ? 'Loading storage' : formatBytes(props.storage.databaseBytes)}</small></div>
      </section>
      {props.recoveryState !== 'ready' ? <div className="alert alert--danger" role="status"><strong>{recoveryTitle(props.recoveryState)}</strong><span>{recoveryDescription(props.recoveryState)}</span></div> : null}
      {props.tasks.length === 0 ? <EmptyState title={props.recoveryState === 'ready' ? 'No task history' : 'Recovery action available'}>{props.recoveryState === 'ready' ? 'Start a Split, Merge, Inspect, or Verify workflow.' : 'Clear All remains available even when task details cannot be loaded safely.'}</EmptyState> : (
        <div className="task-list" aria-live="polite">
          {props.tasks.map((task) => <TaskCard key={task.id} task={task} busy={props.busy} onControl={props.onControl} onPriority={props.onPriority} onReorder={props.onReorder} onReceipt={props.onReceipt} onRemove={props.onRemove} />)}
        </div>
      )}
    </>
  );
}

function recoveryTitle(state: RecoveryDisplayState): string {
  const titles: Record<RecoveryDisplayState, string> = {
    ready: 'Task state ready',
    'recovery-required': 'Interrupted work requires review',
    quarantined: 'Unsafe task records quarantined',
    'capacity-exceeded': 'Recovery capacity exceeded',
    'unsupported-version': 'Unsupported task-state version',
    corrupt: 'Corrupt task state quarantined',
    'snapshot-unavailable': 'Task snapshot unavailable',
  };
  return titles[state];
}

function recoveryDescription(state: RecoveryDisplayState): string {
  if (state === 'recovery-required') return 'Recoverable tasks are shown below and require an explicit Resume or Retry.';
  if (state === 'capacity-exceeded') return 'Only the bounded recovery set was retained; excess rows were rejected and a bounded diagnostic sample was quarantined.';
  if (state === 'unsupported-version') return 'Records from an unsupported schema were not resumed.';
  if (state === 'corrupt') return 'Checksummed state failed validation and was isolated before any file operation.';
  if (state === 'quarantined') return 'One or more invalid records were isolated before worker dispatch.';
  if (state === 'snapshot-unavailable') return 'The native snapshot could not be validated. Event listeners and Clear All remain available.';
  return 'Task state is ready.';
}

function TaskCard({ task, busy, onControl, onPriority, onReorder, onReceipt, onRemove }: {
  task: TaskSnapshot;
  busy: boolean;
  onControl: (task: TaskSnapshot, command: 'pause_task' | 'resume_task' | 'cancel_task' | 'retry_task') => void;
  onPriority: (task: TaskSnapshot, priority: TaskPriority) => void;
  onReorder: (task: TaskSnapshot, direction: 'earlier' | 'later') => void;
  onReceipt: (task: TaskSnapshot, format: 'markdown' | 'json', includePathDetail: boolean) => void;
  onRemove: (task: TaskSnapshot) => void;
}) {
  const percent = task.progress.totalBytes === 0 ? (task.status === 'completed' ? 100 : 0) : task.progress.bytesProcessed / task.progress.totalBytes * 100;
  const active = ['running', 'pausing', 'paused', 'resuming', 'cancelling'].includes(task.status);
  const [receiptDetail, setReceiptDetail] = useState(false);
  const elapsedMs = task.durationMs ?? (task.startedAt === null ? 0 : Math.max(0, Date.now() - Date.parse(task.startedAt)));
  const speed = elapsedMs > 0 ? task.progress.bytesProcessed / (elapsedMs / 1000) : 0;
  const remainingBytes = Math.max(0, task.progress.totalBytes - task.progress.bytesProcessed);
  const estimateMs = speed > 0 ? remainingBytes / speed * 1000 : 0;
  return (
    <article className="task-card">
      <div className="task-card__top"><div><span className="task-operation">{task.operation}</span><h2>{task.displayName}</h2><p>{task.destinationName === null ? task.progress.stage : `${task.progress.stage} · ${task.destinationName}`}</p></div><StatusBadge tone={taskTone(task.status)}>{task.status}</StatusBadge></div>
      <ProgressMeter value={percent} label={`${task.operation} progress for ${task.displayName}`} />
      <dl className="task-meta"><div><dt>Processed</dt><dd>{formatBytes(task.progress.bytesProcessed)} / {formatBytes(task.progress.totalBytes)}</dd></div><div><dt>Slice</dt><dd>{task.progress.currentSlice.toLocaleString()} / {task.progress.sliceCount.toLocaleString()}</dd></div><div><dt>Queue</dt><dd>{task.queuePosition === null ? 'Not queued' : `Position ${task.queuePosition}`}</dd></div><div><dt>Priority</dt><dd>{task.priority}</dd></div><div><dt>Speed</dt><dd>{speed > 0 ? `${formatBytes(speed)}/s` : '—'}</dd></div><div><dt>Elapsed</dt><dd>{formatDuration(elapsedMs)}</dd></div><div><dt>Estimate</dt><dd>{estimateMs > 0 ? formatDuration(estimateMs) : '—'}</dd></div><div><dt>Attempt</dt><dd>{task.attemptCount.toLocaleString()}</dd></div></dl>
      {task.preflight === null ? null : <PreflightView preflight={task.preflight} compact />}
      {task.failure !== null ? <div className="task-failure" role="alert"><strong>{task.failure.category}: {task.failure.code}</strong><span>{task.failure.message}</span><span>Recovery: {recoveryActionLabel(task.failure.recoveryAction)}{task.failure.retryable ? ' Retry is available after the condition is corrected.' : ' Automatic retry is not safe.'}</span><details><summary>Technical detail</summary><code>{task.failure.technicalMessage}</code></details></div> : null}
      {task.result !== null ? <TaskResultView task={task} /> : null}
      <div className="task-actions">
        {task.status === 'queued' ? <label className="inline-select"><span>Priority</span><select aria-label={`Priority for ${task.displayName}`} value={task.priority} onChange={(event) => onPriority(task, event.target.value as TaskPriority)} disabled={busy}><option value="high">High</option><option value="normal">Normal</option><option value="low">Low</option></select></label> : null}
        {task.status === 'queued' ? <button type="button" onClick={() => onReorder(task, 'earlier')} disabled={busy || task.queuePosition === 1} aria-label={`Move ${task.displayName} earlier`}>Move earlier</button> : null}
        {task.status === 'queued' ? <button type="button" onClick={() => onReorder(task, 'later')} disabled={busy} aria-label={`Move ${task.displayName} later`}>Move later</button> : null}
        {task.status === 'running' ? <button type="button" onClick={() => onControl(task, 'pause_task')} disabled={busy}>Pause</button> : null}
        {task.status === 'paused' || task.status === 'interrupted' ? <button type="button" onClick={() => onControl(task, 'resume_task')} disabled={busy}>Resume</button> : null}
        {(task.status === 'failed' || task.status === 'permission-required') && task.recoveryEligible ? <button type="button" onClick={() => onControl(task, 'retry_task')} disabled={busy}>Retry safely</button> : null}
        {task.status === 'cancelled' ? <button type="button" onClick={() => onControl(task, 'resume_task')} disabled={busy}>Restart safely</button> : null}
        {active || task.status === 'queued' || task.status === 'interrupted' ? <button type="button" className="danger-text" onClick={() => onControl(task, 'cancel_task')} disabled={busy || task.status === 'cancelling'}>Cancel</button> : null}
        {['completed', 'failed', 'cancelled'].includes(task.status) ? <button type="button" onClick={() => onRemove(task)} disabled={busy}>Remove history</button> : null}
        {['completed', 'failed'].includes(task.status) ? <details className="receipt-options"><summary>Export receipt</summary><label><input type="checkbox" checked={receiptDetail} onChange={(event) => setReceiptDetail(event.target.checked)} /> Include more local path detail</label><span>Masked filenames are the default. Detailed paths require confirmation.</span><div><button type="button" onClick={() => onReceipt(task, 'markdown', receiptDetail)} disabled={busy}>Markdown</button><button type="button" onClick={() => onReceipt(task, 'json', receiptDetail)} disabled={busy}>JSON</button></div></details> : null}
      </div>
    </article>
  );
}

function TaskResultView({ task }: { task: TaskSnapshot }) {
  if (task.result?.type === 'split') return <div className="result"><strong>Manifest</strong><span>{task.result.manifestFilename}</span><code>{task.result.sourceSha256}</code></div>;
  if (task.result?.type === 'merge') return <div className="result"><strong>Rebuilt Cake</strong><span>{task.result.outputFilename}</span><code>{task.result.outputSha256}</code></div>;
  if (task.result?.type === 'inspection') return <div className="result"><strong>{task.result.inspection.verified ? 'Verified' : 'Inspected'}</strong><span>{task.result.inspection.originalFilename}</span><code>{task.result.inspection.originalSha256}</code></div>;
  return null;
}

function SettingsWorkspace({ settings, busy, onSave }: { settings: DesktopSettings; busy: boolean; onSave: (settings: DesktopSettings) => void }) {
  const [draft, setDraft] = useState(settings);
  useEffect(() => setDraft(settings), [settings]);
  return (
    <>
      <WorkspaceHeading eyebrow="Local preferences" title="Settings">These preferences affect the local desktop interface only. No settings or task metadata leave this computer.</WorkspaceHeading>
      <section className="panel settings-panel">
        <label className="field"><span>Default Slice size (bytes)</span><input type="number" min="1" max={Number.MAX_SAFE_INTEGER} value={draft.defaultSliceSize} onChange={(event) => setDraft({ ...draft, defaultSliceSize: Number(event.target.value) })} /><small>{formatBytes(draft.defaultSliceSize)}</small></label>
        <label className="field"><span>Maximum terminal task history</span><input type="number" min="1" max="500" value={draft.maximumTerminalHistory} onChange={(event) => setDraft({ ...draft, maximumTerminalHistory: Number(event.target.value) })} /><small>Completed, failed, and cancelled records are pruned by this count limit. Nonterminal tasks are preserved.</small></label>
        <label className="field"><span>Terminal history retention (days)</span><input type="number" min="1" max="3650" value={draft.terminalHistoryDays} onChange={(event) => setDraft({ ...draft, terminalHistoryDays: Number(event.target.value) })} /><small>Age and count limits are both enforced locally.</small></label>
        <label className="check-field"><input type="checkbox" checked={draft.confirmDestructiveActions} onChange={(event) => setDraft({ ...draft, confirmDestructiveActions: event.target.checked })} /><span><strong>Confirm destructive actions</strong><small>Ask before clearing task history.</small></span></label>
        <label className="check-field"><input type="checkbox" checked={draft.reduceMotion} onChange={(event) => setDraft({ ...draft, reduceMotion: event.target.checked })} /><span><strong>Reduce motion</strong><small>Disable nonessential progress transitions.</small></span></label>
        <button className="button button--primary" type="button" onClick={() => onSave(draft)} disabled={busy || draft.defaultSliceSize < 1 || draft.maximumTerminalHistory < 1 || draft.maximumTerminalHistory > 500 || draft.terminalHistoryDays < 1 || draft.terminalHistoryDays > 3650}>Save settings</button>
      </section>
    </>
  );
}

function AboutWorkspace({ runtime }: { runtime: RuntimeInfo | null }) {
  return (
    <>
      <WorkspaceHeading eyebrow="Early native prototype" title="About CakeSplitter Desktop">Native Windows workflows for local, streamed Cake Package Split, Merge, Inspect, and Verify.</WorkspaceHeading>
      <div className="about-grid">
        <section className="panel"><h2>Runtime boundary</h2><dl className="about-list"><div><dt>Application</dt><dd>{runtime?.applicationVersion ?? '0.6.0'}</dd></div><div><dt>Cake Package</dt><dd>{runtime?.formatVersion ?? '1.0'}</dd></div><div><dt>Platform</dt><dd>{runtime?.platform ?? 'windows-x64'}</dd></div><div><dt>Signing</dt><dd>{runtime?.signedBuild === true ? 'Signed' : 'Unsigned preview'}</dd></div></dl></section>
        <section className="panel"><h2>Privacy promise</h2><ul className="check-list"><li>Files, paths, hashes, and task data remain local.</li><li>No telemetry, analytics, crash upload, or remote logging.</li><li>No automatic update checks or background service.</li><li>Bundled static interface; no remote application content.</li></ul></section>
        <section className="panel panel--wide"><h2>Known boundary</h2><p>The v0.6.0 release remains Windows 10 and Windows 11 x64 only. The unsigned installer is a private local preview. Resume is at verified Slice boundaries, not arbitrary bytes.</p></section>
      </div>
    </>
  );
}

function InspectionCompact({ inspection }: { inspection: InspectionSummary }) {
  return <dl className="metrics metrics--compact"><div><dt>File</dt><dd>{inspection.originalFilename}</dd></div><div><dt>Size</dt><dd>{formatBytes(inspection.originalSize)}</dd></div><div><dt>Slices</dt><dd>{inspection.foundSliceCount} / {inspection.expectedSliceCount}</dd></div><div><dt>Issues</dt><dd>{inspection.missing.length + inspection.corrupted.length + inspection.unexpected.length}</dd></div></dl>;
}

function InspectionDetails({ inspection }: { inspection: InspectionSummary }) {
  const structuralReady = inspection.missing.length === 0 && inspection.corrupted.length === 0 && inspection.unexpected.length === 0;
  return (
    <section className={structuralReady ? 'inspection inspection--ready' : 'inspection inspection--failed'} aria-labelledby="inspection-verdict">
      <div className="inspection__verdict"><span className="eyebrow">Readiness verdict</span><h2 id="inspection-verdict">{inspection.verified ? 'Package verified' : structuralReady ? 'Structure ready; hashes not yet verified' : 'Package is not ready'}</h2><p>{inspection.originalFilename} · {formatBytes(inspection.originalSize)} · format {inspection.formatVersion}</p></div>
      <InspectionCompact inspection={inspection} />
      <div className="issue-grid"><IssueList title="Missing" items={inspection.missing} /><IssueList title="Corrupted" items={inspection.corrupted} /><IssueList title="Unexpected" items={inspection.unexpected} /></div>
      <details><summary>Technical details</summary><dl className="technical"><div><dt>Package ID</dt><dd><code>{inspection.packageId}</code></dd></div><div><dt>Original SHA-256</dt><dd><code>{inspection.originalSha256}</code></dd></div></dl></details>
    </section>
  );
}

function IssueList({ title, items }: { title: string; items: string[] }) {
  return <div className="issue-list"><strong>{title} <span>{items.length}</span></strong>{items.length === 0 ? <p>None</p> : <ul>{items.slice(0, MAX_RENDERED_PACKAGE_DIAGNOSTIC_ROWS).map((item) => <li key={item}>{item}</li>)}</ul>}</div>;
}

function EmptyState({ title, children }: { title: string; children: ReactNode }) {
  return <section className="empty-state"><span aria-hidden="true">◇</span><h2>{title}</h2><p>{children}</p></section>;
}

function taskTone(status: TaskStatus): 'neutral' | 'working' | 'success' | 'danger' {
  if (status === 'completed') return 'success';
  if (['failed', 'permission-required', 'cancelled'].includes(status)) return 'danger';
  if (['running', 'pausing', 'paused', 'resuming', 'cancelling'].includes(status)) return 'working';
  return 'neutral';
}

function countTaskStatuses(tasks: TaskSnapshot[]): {
  active: number;
  queued: number;
  attention: number;
  terminal: number;
} {
  return tasks.reduce((counts, task) => {
    if (['running', 'pausing', 'paused', 'resuming', 'cancelling'].includes(task.status)) counts.active += 1;
    if (task.status === 'queued') counts.queued += 1;
    if (['failed', 'permission-required', 'interrupted'].includes(task.status)) counts.attention += 1;
    if (['completed', 'failed', 'cancelled'].includes(task.status)) counts.terminal += 1;
    return counts;
  }, { active: 0, queued: 0, attention: 0, terminal: 0 });
}

function recoveryActionLabel(action: RecoveryAction): string {
  return action.split('-').map((part) => `${part.slice(0, 1).toUpperCase()}${part.slice(1)}`).join(' ');
}

function formatDuration(milliseconds: number): string {
  if (!Number.isFinite(milliseconds) || milliseconds <= 0) return '—';
  const seconds = Math.max(1, Math.round(milliseconds / 1000));
  const hours = Math.floor(seconds / 3600);
  const minutes = Math.floor((seconds % 3600) / 60);
  const remainder = seconds % 60;
  if (hours > 0) return `${hours}h ${minutes}m`;
  if (minutes > 0) return `${minutes}m ${remainder}s`;
  return `${remainder}s`;
}

function formatBytes(value: number): string {
  if (!Number.isFinite(value) || value < 0) return '—';
  if (value < 1024) return `${value.toLocaleString()} B`;
  const units = ['KiB', 'MiB', 'GiB', 'TiB'];
  let amount = value / 1024;
  let index = 0;
  while (amount >= 1024 && index < units.length - 1) {
    amount /= 1024;
    index += 1;
  }
  return `${amount >= 10 ? amount.toFixed(0) : amount.toFixed(1)} ${units[index]}`;
}
