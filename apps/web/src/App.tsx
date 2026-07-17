import {
  useEffect,
  useMemo,
  useRef,
  useState,
  type ChangeEvent,
  type DragEvent,
} from 'react';

import {
  expectedSliceCount,
  MAX_BROWSER_FALLBACK_BYTES,
  MAX_BROWSER_FALLBACK_DOWNLOADS,
  MAX_BROWSER_SELECTED_FILES,
  MAX_MANIFEST_BYTES,
  parseManifest,
  type CakeManifest,
} from '@cakesplitter/shared-types';
import { FileInput, ProgressMeter, StatusBadge } from '@cakesplitter/ui';
import { getDirectFolderCapabilities } from '@cakesplitter/web-file-io';

import { observePwa, type PwaController, type PwaSnapshot } from './pwa';
import {
  parseWorkerResponse,
  type InspectionResult,
  type OutputMode,
  type TaskStatus,
  type WorkerOperation,
  type WorkerRequest,
  type WorkerResponse,
  type Workspace,
} from './protocol';
import { TaskStore, type PersistedTask } from './task-store';

interface ProgressState {
  bytesProcessed: number;
  totalBytes: number;
  currentSlice: number;
  sliceCount: number;
  speedBytesPerSecond: number;
  message: string;
}

type StartRequest = Extract<WorkerRequest, { type: 'start' }>;
type WithoutIdentity<T> = T extends unknown ? Omit<T, 'requestId' | 'taskId'> : never;
type StartRequestInput = WithoutIdentity<StartRequest>;

interface ActiveContext {
  requestId: string;
  taskId: string;
  operation: WorkerOperation;
  metadata: PersistedTask;
  lastPersistedSlice: number;
  persistenceGeneration: number;
  clearing: boolean;
  acknowledgeClear?: () => void;
}

interface PackageReadiness {
  missing: string[];
  duplicate: string[];
  sizeMismatch: string[];
  unexpected: string[];
}

const SLICE_PRESETS = [
  { label: '10 MiB', value: 10 * 1024 * 1024 },
  { label: '100 MiB', value: 100 * 1024 * 1024 },
  { label: '650 MiB', value: 650 * 1024 * 1024 },
];
const TASK_STORE = new TaskStore();
const INITIAL_PWA: PwaSnapshot = {
  online: typeof navigator === 'undefined' ? true : navigator.onLine,
  serviceWorkerSupported: false,
  installed: false,
  updateAvailable: false,
};

export function App() {
  const [workspace, setWorkspace] = useState<Workspace>('split');
  const [splitFile, setSplitFile] = useState<File>();
  const [sliceSize, setSliceSize] = useState(100 * 1024 * 1024);
  const [planningMode, setPlanningMode] = useState<'size' | 'count'>('size');
  const [requestedSliceCount, setRequestedSliceCount] = useState(10);
  const [manifestFile, setManifestFile] = useState<File>();
  const [manifestText, setManifestText] = useState('');
  const [parsedManifest, setParsedManifest] = useState<CakeManifest>();
  const [sliceFiles, setSliceFiles] = useState<File[]>([]);
  const [directory, setDirectory] = useState<FileSystemDirectoryHandle>();
  const [outputMode, setOutputMode] = useState<'direct' | 'fallback'>('fallback');
  const [taskState, setTaskState] = useState<TaskStatus>('planned');
  const [activeOperation, setActiveOperation] = useState<WorkerOperation>();
  const [progress, setProgress] = useState<ProgressState>();
  const [inspection, setInspection] = useState<InspectionResult>();
  const [resultMessage, setResultMessage] = useState('');
  const [resultTone, setResultTone] = useState<'success' | 'danger'>('success');
  const [errorMessage, setErrorMessage] = useState('');
  const [tasks, setTasks] = useState<PersistedTask[]>([]);
  const [taskStorageMessage, setTaskStorageMessage] = useState('Loading browser-local task metadata…');
  const [clearingTasks, setClearingTasks] = useState(false);
  const [storageUsage, setStorageUsage] = useState<{ usage: number; quota?: number }>({ usage: 0 });
  const [recoveryCandidate, setRecoveryCandidate] = useState<PersistedTask>();
  const [pwa, setPwa] = useState<PwaSnapshot>(INITIAL_PWA);
  const [pwaMessage, setPwaMessage] = useState('');
  const workerRef = useRef<Worker | undefined>(undefined);
  const activeRef = useRef<ActiveContext | undefined>(undefined);
  const pwaControllerRef = useRef<PwaController | undefined>(undefined);
  const persistenceRef = useRef<Promise<void>>(Promise.resolve());
  const clearTasksRef = useRef<Promise<void> | undefined>(undefined);
  const taskStorageBarrierFailedRef = useRef(false);
  const capabilities = useMemo(() => getDirectFolderCapabilities(window), []);

  const effectiveSliceSize = useMemo(() => {
    if (planningMode === 'size') return sliceSize;
    if (!splitFile || !Number.isSafeInteger(requestedSliceCount) || requestedSliceCount < 1) return 0;
    return Math.max(1, Math.ceil(splitFile.size / requestedSliceCount));
  }, [planningMode, requestedSliceCount, sliceSize, splitFile]);

  const estimatedSlices = useMemo(() => {
    if (!splitFile || !Number.isSafeInteger(effectiveSliceSize) || effectiveSliceSize < 1) return 0;
    return expectedSliceCount(splitFile.size, effectiveSliceSize);
  }, [effectiveSliceSize, splitFile]);

  const packageReadiness = useMemo(
    () => analyzePackageReadiness(parsedManifest, sliceFiles),
    [parsedManifest, sliceFiles],
  );

  useEffect(() => {
    let disposed = false;
    const persistenceGeneration = TASK_STORE.captureGeneration();
    void TASK_STORE.markInterrupted(persistenceGeneration)
      .then(async (stored) => {
        if (disposed) return;
        setTasks(stored);
        const estimate = await TASK_STORE.storageEstimate();
        if (!disposed) {
          setStorageUsage({ usage: estimate.usage, ...(estimate.quota !== undefined ? { quota: estimate.quota } : {}) });
          setTaskStorageMessage(
            stored.length
              ? `${stored.length} bounded task ${stored.length === 1 ? 'record' : 'records'} stored locally.`
              : 'No recoverable task metadata is stored.',
          );
        }
      })
      .catch((error: unknown) => {
        if (!disposed) setTaskStorageMessage(errorMessageOf(error));
      });
    void observePwa(setPwa)
      .then((controller) => {
        if (disposed) controller.dispose();
        else pwaControllerRef.current = controller;
      })
      .catch((error: unknown) => {
        if (!disposed) setPwaMessage(`Offline installation is unavailable: ${errorMessageOf(error)}`);
      });
    return () => {
      disposed = true;
      pwaControllerRef.current?.dispose();
      workerRef.current?.terminate();
    };
  }, []);

  const persistTask = (task: PersistedTask, generation: number): Promise<boolean> => {
    const operation = persistenceRef.current.catch(() => undefined).then(async () => {
      if (!TASK_STORE.isCurrentGeneration(generation)) return false;
      try {
        if (!(await TASK_STORE.save(task, generation))) return false;
        setTasks((current) => [task, ...current.filter((entry) => entry.taskId !== task.taskId)]);
        const estimate = await TASK_STORE.storageEstimate();
        if (!TASK_STORE.isCurrentGeneration(generation)) return false;
        setStorageUsage({ usage: estimate.usage, ...(estimate.quota !== undefined ? { quota: estimate.quota } : {}) });
        setTaskStorageMessage('Recovery metadata saved locally. File contents were not stored.');
        return true;
      } catch (error) {
        if (!TASK_STORE.isCurrentGeneration(generation)) return false;
        setTaskStorageMessage(`Task metadata was not persisted: ${errorMessageOf(error)}`);
        return true;
      }
    });
    persistenceRef.current = operation.then(() => undefined);
    return operation;
  };

  const updateActiveTask = (patch: Partial<PersistedTask>) => {
    const active = activeRef.current;
    if (!active || !TASK_STORE.isCurrentGeneration(active.persistenceGeneration)) return;
    const updated = {
      ...active.metadata,
      ...patch,
      updatedAt: new Date().toISOString(),
    } as PersistedTask;
    active.metadata = updated;
    void persistTask(updated, active.persistenceGeneration);
  };

  const selectDirectory = async () => {
    if (!capabilities.supported) {
      setErrorMessage(capabilities.reason);
      return;
    }
    try {
      const handle = await window.showDirectoryPicker({ id: 'splitthecake-output', mode: 'readwrite' });
      setDirectory(handle);
      setOutputMode('direct');
    } catch (error) {
      if (!(error instanceof DOMException && error.name === 'AbortError')) {
        setErrorMessage(errorMessageOf(error));
      }
    }
  };

  const handleManifest = async (event: ChangeEvent<HTMLInputElement>) => {
    const file = event.target.files?.[0];
    setManifestFile(file);
    setParsedManifest(undefined);
    setManifestText('');
    setInspection(undefined);
    setErrorMessage('');
    if (!file) return;
    try {
      if (file.size > MAX_MANIFEST_BYTES) {
        throw new Error(`Cake Manifest exceeds the ${formatBytes(MAX_MANIFEST_BYTES)} limit.`);
      }
      const text = await file.text();
      const manifest = parseManifest(text);
      setManifestText(text);
      setParsedManifest(manifest);
    } catch (error) {
      setErrorMessage(errorMessageOf(error));
    }
  };

  const handleSlices = (event: ChangeEvent<HTMLInputElement>) => {
    const files = Array.from(event.target.files ?? []);
    if (files.length > MAX_BROWSER_SELECTED_FILES) {
      setSliceFiles([]);
      setErrorMessage(
        `Choose no more than ${new Intl.NumberFormat().format(MAX_BROWSER_SELECTED_FILES)} Slice files.`,
      );
      return;
    }
    setSliceFiles(files);
    setInspection(undefined);
    setErrorMessage('');
  };

  const startWorker = async (request: StartRequestInput, metadata: Omit<PersistedTask, 'taskId' | 'status' | 'createdAt' | 'updatedAt'>) => {
    if (taskStorageBarrierFailedRef.current) {
      setErrorMessage('Browser-local cleanup previously failed closed. Reload before starting another task.');
      return;
    }
    if (clearTasksRef.current) {
      setErrorMessage('Clear All is still completing. Start a new task after browser-local cleanup finishes.');
      return;
    }
    if (workerRef.current || activeRef.current) {
      setErrorMessage('Another task is active. Finish or cancel it before starting a new task.');
      return;
    }
    const taskId = crypto.randomUUID();
    const requestId = crypto.randomUUID();
    const persistenceGeneration = TASK_STORE.captureGeneration();
    const now = new Date().toISOString();
    const task: PersistedTask = {
      ...metadata,
      taskId,
      status: 'running',
      createdAt: now,
      updatedAt: now,
      completedSliceIndexes: [],
    };
    if (!(await persistTask(task, persistenceGeneration))) {
      setErrorMessage('The task start was invalidated by Clear All before processing began.');
      return;
    }
    setRecoveryCandidate(undefined);

    const activeWorker = new Worker(new URL('./worker.ts', import.meta.url), { type: 'module' });
    const context: ActiveContext = {
      requestId,
      taskId,
      operation: request.operation,
      metadata: task,
      lastPersistedSlice: 0,
      persistenceGeneration,
      clearing: false,
    };
    workerRef.current = activeWorker;
    activeRef.current = context;
    setActiveOperation(request.operation);
    setTaskState('running');
    setProgress(undefined);
    setInspection(undefined);
    setResultMessage('');
    setResultTone('success');
    setErrorMessage('');

    const acknowledgeClear = () => {
      const acknowledge = context.acknowledgeClear;
      delete context.acknowledgeClear;
      acknowledge?.();
    };

    const finish = () => {
      acknowledgeClear();
      activeWorker.terminate();
      if (workerRef.current === activeWorker) workerRef.current = undefined;
      if (activeRef.current === context) activeRef.current = undefined;
      setActiveOperation(undefined);
    };

    activeWorker.addEventListener('message', (event: MessageEvent<unknown>) => {
      let message: WorkerResponse;
      try {
        message = parseWorkerResponse(event.data);
      } catch (error) {
        setTaskState('failed');
        setErrorMessage(errorMessageOf(error));
        updateActiveTask({ status: 'failed' });
        finish();
        return;
      }
      if (message.taskId !== context.taskId || message.requestId !== context.requestId || message.operation !== context.operation) {
        setTaskState('failed');
        setErrorMessage('Rejected a stale or mismatched Worker message. The task was stopped safely.');
        updateActiveTask({ status: 'failed' });
        finish();
        return;
      }
      if (context.clearing || !TASK_STORE.isCurrentGeneration(context.persistenceGeneration)) {
        if (
          (message.type === 'state' && message.status === 'cancelled') ||
          message.type === 'result' ||
          message.type === 'error'
        ) {
          acknowledgeClear();
        }
        return;
      }
      if (message.type === 'state') {
        setTaskState(message.status);
        updateActiveTask({ status: message.status });
        return;
      }
      if (message.type === 'progress') {
        setProgress(message);
        const completed = Math.max(0, message.currentSlice - 1);
        if (completed > context.lastPersistedSlice) {
          context.lastPersistedSlice = completed;
          updateActiveTask({
            completedSliceIndexes: Array.from({ length: completed }, (_, index) => index + 1),
          });
        }
        return;
      }
      if (message.type === 'download') {
        downloadBlob(message.filename, message.blob);
        return;
      }
      if (message.type === 'result') {
        const incomplete = message.status === 'incomplete';
        setTaskState(message.status);
        setResultTone(incomplete ? 'danger' : 'success');
        setResultMessage(message.message);
        if (message.inspection) setInspection(message.inspection);
        updateActiveTask({
          status: message.status,
          ...(message.manifest
            ? {
                packageId: message.manifest.packageId,
                expectedSha256: message.manifest.original.sha256,
                sliceCount: message.manifest.sliceCount,
                completedSliceIndexes: Array.from({ length: message.manifest.sliceCount }, (_, index) => index + 1),
              }
            : {}),
          ...(message.outputSha256 ? { expectedSha256: message.outputSha256 } : {}),
        });
        finish();
        return;
      }
      setTaskState(message.status);
      setErrorMessage(message.message);
      updateActiveTask({ status: message.status });
      finish();
    });
    activeWorker.addEventListener('error', (event) => {
      if (context.clearing || !TASK_STORE.isCurrentGeneration(context.persistenceGeneration)) {
        finish();
        return;
      }
      setTaskState('failed');
      setErrorMessage(event.message || 'The processing Worker stopped unexpectedly.');
      updateActiveTask({ status: 'failed' });
      finish();
    });
    const completeRequest = { ...request, requestId, taskId } as WorkerRequest;
    activeWorker.postMessage(completeRequest);
  };

  const control = (command: 'pause' | 'resume' | 'cancel') => {
    const active = activeRef.current;
    if (!active || !workerRef.current) return;
    workerRef.current.postMessage({
      type: 'control',
      requestId: crypto.randomUUID(),
      taskId: active.taskId,
      command,
    } satisfies WorkerRequest);
  };

  const validateRecovery = (operation: WorkerOperation): boolean => {
    const candidate = recoveryCandidate;
    if (!candidate) return true;
    if (candidate.operation !== operation) {
      setErrorMessage('The selected recovery task belongs to a different operation.');
      return false;
    }
    if (operation === 'split') {
      if (!splitFile || splitFile.name !== candidate.originalFilename || splitFile.size !== candidate.expectedSize) {
        setErrorMessage('Reselect the original source with the same portable filename and byte size before resuming.');
        return false;
      }
      return true;
    }
    if (
      !parsedManifest ||
      parsedManifest.original.filename !== candidate.originalFilename ||
      parsedManifest.original.size !== candidate.expectedSize ||
      (candidate.packageId !== undefined && parsedManifest.packageId !== candidate.packageId)
    ) {
      setErrorMessage('Reselect the matching manifest. Filename, size, and package ID must match recovery metadata.');
      return false;
    }
    return true;
  };

  const runSplit = () => {
    if (!splitFile || !Number.isSafeInteger(effectiveSliceSize) || effectiveSliceSize < 1) {
      setErrorMessage('Choose one Cake and enter a valid whole-number Slice plan.');
      return;
    }
    if (!validateRecovery('split')) return;
    if (outputMode === 'fallback' && splitFile.size > MAX_BROWSER_FALLBACK_BYTES) {
      setErrorMessage(`Compatibility Split is limited to ${formatBytes(MAX_BROWSER_FALLBACK_BYTES)}. Direct Folder Mode is required for larger files, but ${capabilities.reason.toLocaleLowerCase()}`);
      return;
    }
    if (outputMode === 'fallback' && estimatedSlices > MAX_BROWSER_FALLBACK_DOWNLOADS) {
      setErrorMessage(`Compatibility Split supports at most ${MAX_BROWSER_FALLBACK_DOWNLOADS} downloads. Increase the Slice size or use a securely supported direct-folder browser.`);
      return;
    }
    void startWorker(
      {
        type: 'start',
        operation: 'split',
        file: splitFile,
        sliceSize: effectiveSliceSize,
        outputMode,
        ...(outputMode === 'direct' && directory ? { directory } : {}),
      },
      {
        schemaVersion: 1,
        operation: 'split',
        originalFilename: splitFile.name,
        expectedSize: splitFile.size,
        sliceSize: effectiveSliceSize,
        sliceCount: estimatedSlices,
        completedSliceIndexes: [],
        outputMode,
        capability: { directFolder: capabilities.supported, reason: capabilities.reason },
        recovery: { sourceFile: true, manifest: false, slices: false, outputDirectory: outputMode === 'direct' },
      },
    );
  };

  const runInspect = () => {
    if (!manifestText || !parsedManifest) {
      setErrorMessage('Choose a valid .cake.json manifest first.');
      return;
    }
    if (!validateRecovery('inspect')) return;
    void startWorker(
      { type: 'start', operation: 'inspect', manifestText, files: sliceFiles },
      packageTaskMetadata('inspect', 'read-only', parsedManifest, capabilities.reason),
    );
  };

  const runMerge = () => {
    if (!manifestText || !parsedManifest) {
      setErrorMessage('Choose a valid .cake.json manifest first.');
      return;
    }
    if (!validateRecovery('merge')) return;
    if (packageReadiness.missing.length || packageReadiness.duplicate.length || packageReadiness.sizeMismatch.length || packageReadiness.unexpected.length) {
      setErrorMessage('Resolve missing, duplicate, unexpected, and size-mismatched Slices before Merge.');
      return;
    }
    if (outputMode === 'fallback' && parsedManifest.original.size > MAX_BROWSER_FALLBACK_BYTES) {
      setErrorMessage(`Compatibility Merge is limited to ${formatBytes(MAX_BROWSER_FALLBACK_BYTES)} because the rebuilt output is buffered. ${capabilities.reason}`);
      return;
    }
    void startWorker(
      {
        type: 'start',
        operation: 'merge',
        manifestText,
        files: sliceFiles,
        outputMode,
        ...(outputMode === 'direct' && directory ? { directory } : {}),
      },
      packageTaskMetadata('merge', outputMode, parsedManifest, capabilities.reason),
    );
  };

  const resumeStoredTask = (task: PersistedTask) => {
    setRecoveryCandidate(task);
    setWorkspace(task.operation);
    setErrorMessage('');
    setResultMessage('');
    setTaskState('planned');
  };

  const discardTask = async (task: PersistedTask) => {
    if (!window.confirm(`Discard recovery metadata for ${task.originalFilename}? User files are not deleted.`)) return;
    await TASK_STORE.discard(task.taskId);
    setTasks((current) => current.filter((entry) => entry.taskId !== task.taskId));
    if (recoveryCandidate?.taskId === task.taskId) setRecoveryCandidate(undefined);
  };

  const clearTasks = (): Promise<void> => {
    if (clearTasksRef.current) return clearTasksRef.current;
    if (!window.confirm('Clear all CakeSplitter task metadata and controlled temporary state from this browser?')) {
      return Promise.resolve();
    }
    const operation = (async () => {
      setClearingTasks(true);
      setTaskStorageMessage('Stopping active processing and clearing browser-local task metadata…');
      const clearOperation = TASK_STORE.clear();
      const active = activeRef.current;
      const activeWorker = workerRef.current;
      let workerAcknowledgement = Promise.resolve();
      try {
        if (active && activeWorker) {
          active.clearing = true;
          workerAcknowledgement = new Promise<void>((resolve) => {
            active.acknowledgeClear = resolve;
          });
          activeWorker.postMessage({
            type: 'control',
            requestId: crypto.randomUUID(),
            taskId: active.taskId,
            command: 'cancel',
          } satisfies WorkerRequest);
        }
        await Promise.all([
          clearOperation,
          workerAcknowledgement,
          persistenceRef.current.catch(() => undefined),
        ]);
        if (active && activeWorker) {
          activeWorker.terminate();
          if (workerRef.current === activeWorker) workerRef.current = undefined;
          if (activeRef.current === active) activeRef.current = undefined;
          setTaskState('cancelled');
          setActiveOperation(undefined);
          setProgress(undefined);
        }
        setTasks([]);
        setRecoveryCandidate(undefined);
        setStorageUsage({ usage: 0 });
        taskStorageBarrierFailedRef.current = false;
        setResultMessage('');
        setErrorMessage('');
        setTaskStorageMessage('All browser-local CakeSplitter task metadata was cleared.');
      } catch (error) {
        if (active && activeWorker) {
          activeWorker.terminate();
          if (workerRef.current === activeWorker) workerRef.current = undefined;
          if (activeRef.current === active) activeRef.current = undefined;
          setActiveOperation(undefined);
          setProgress(undefined);
        }
        await clearOperation.catch(() => undefined);
        await persistenceRef.current.catch(() => undefined);
        taskStorageBarrierFailedRef.current = true;
        setTaskStorageMessage(`Clear All did not complete: ${errorMessageOf(error)}`);
        setErrorMessage('Browser-local cleanup failed closed. Reload before starting another task.');
      } finally {
        setClearingTasks(false);
      }
    })();
    clearTasksRef.current = operation;
    void operation.finally(() => {
      if (clearTasksRef.current === operation) clearTasksRef.current = undefined;
    });
    return operation;
  };

  const applyUpdate = async () => {
    try {
      await pwaControllerRef.current?.activateUpdate(taskState === 'running' || taskState === 'paused');
      setPwaMessage('The application update is activating.');
    } catch (error) {
      setPwaMessage(errorMessageOf(error));
    }
  };

  const onDrop = (event: DragEvent<HTMLLabelElement>) => {
    event.preventDefault();
    const [file] = Array.from(event.dataTransfer.files);
    if (file) {
      setSplitFile(file);
      setErrorMessage('');
    }
  };

  const progressPercent = progress
    ? progress.totalBytes === 0
      ? 100
      : (progress.bytesProcessed / progress.totalBytes) * 100
    : 0;
  const active = taskState === 'running' || taskState === 'paused';

  return (
    <div className="app-shell">
      <header className="site-header">
        <a className="brand" href="#workspace" aria-label="SplitTheCake home">
          <CakeMark />
          <span><strong>SplitTheCake</strong><small>CakeSplitter v0.3.0</small></span>
        </a>
        <div className="header-status" aria-label="Application status">
          <StatusBadge tone={pwa.online ? 'neutral' : 'working'}>{pwa.online ? 'Online · local-only' : 'Offline · local-only'}</StatusBadge>
          <span className="version-chip">Format 1.0</span>
        </div>
      </header>

      <main id="workspace" className="main-layout" tabIndex={-1}>
        <section className="intro" aria-labelledby="page-title">
          <div className="eyebrow">Local files, cut with proof</div>
          <h1 id="page-title">Cut a Cake into verified Slices. Layer it back exactly.</h1>
          <p>A restart-aware local processing workbench with bounded memory, explicit capability gates, and byte-for-byte SHA-256 evidence.</p>
          <div className="trust-row">
            <TrustItem icon={<DeviceIcon />} title="Worker processing" detail="Long-running reads and hashes stay off the interface thread." />
            <TrustItem icon={<HashIcon />} title="SHA-256 evidence" detail="Every selected Slice and successful rebuild is verified." />
            <TrustItem icon={<NoCloudIcon />} title="No cloud fallback" detail="No upload, account, telemetry, or remote error path exists." />
          </div>
          <p className="privacy-proof"><CheckIcon /><strong>Processed locally in your browser. Your files never leave your device.</strong></p>
        </section>

        <section className="workbench" aria-label="Cake processing application">
          <nav className="workspace-tabs workspace-tabs--five" aria-label="Primary navigation">
            {NAVIGATION.map((item) => (
              <button
                key={item.id}
                className={workspace === item.id ? 'workspace-tab workspace-tab--active' : 'workspace-tab'}
                type="button"
                aria-current={workspace === item.id ? 'page' : undefined}
                onClick={() => {
                  setWorkspace(item.id);
                  setErrorMessage('');
                  setResultMessage('');
                }}
              >
                {item.icon}
                <span>{item.label}</span>
              </button>
            ))}
          </nav>

          <div className="workspace-panel">
            <div className="panel-heading">
              <div>
                <span className="step-label">{WORKSPACE_COPY[workspace].step}</span>
                <h2>{WORKSPACE_COPY[workspace].title}</h2>
                <p>{WORKSPACE_COPY[workspace].description}</p>
              </div>
              {workspace !== 'about' ? <StatusBadge tone={statusTone(taskState)}>{statusLabel(taskState)}</StatusBadge> : null}
            </div>

            {recoveryCandidate && workspace === recoveryCandidate.operation ? (
              <div className="notice notice--warning" role="status">
                <HistoryIcon />
                <div>
                  <strong>Recovery requires reselection</strong>
                  <p>Reselect the required local inputs. Compatibility tasks restart from byte zero under a new task ID; stale progress and partial output are never reused. Package metadata and every selected Slice are revalidated before success.</p>
                </div>
              </div>
            ) : null}

            {workspace === 'split' ? (
              <>
                <SplitWorkspace
                  file={splitFile}
                  planningMode={planningMode}
                  sliceSize={sliceSize}
                  requestedSliceCount={requestedSliceCount}
                  effectiveSliceSize={effectiveSliceSize}
                  estimatedSlices={estimatedSlices}
                  onFile={setSplitFile}
                  onDrop={onDrop}
                  onPlanningMode={setPlanningMode}
                  onSliceSize={setSliceSize}
                  onSliceCount={setRequestedSliceCount}
                />
                <OutputModeCard capabilities={capabilities} directory={directory} mode={outputMode} onMode={setOutputMode} onSelect={selectDirectory} />
                <FallbackWarning operation="split" size={splitFile?.size ?? 0} activeSliceSize={effectiveSliceSize} downloadCount={estimatedSlices + 1} />
              </>
            ) : workspace === 'merge' || workspace === 'inspect' ? (
              <>
                <PackageInputs manifestFile={manifestFile} parsedManifest={parsedManifest} sliceFiles={sliceFiles} readiness={packageReadiness} onManifest={handleManifest} onSlices={handleSlices} />
                {workspace === 'merge' ? (
                  <>
                    <OutputModeCard capabilities={capabilities} directory={directory} mode={outputMode} onMode={setOutputMode} onSelect={selectDirectory} />
                    <FallbackWarning operation="merge" size={parsedManifest?.original.size ?? 0} activeSliceSize={0} downloadCount={1} />
                  </>
                ) : (
                  <div className="mode-card mode-card--read-only"><InspectIcon /><div><strong>Read-only inspection</strong><span>No output file is written.</span></div></div>
                )}
              </>
            ) : workspace === 'tasks' ? (
              <TasksWorkspace tasks={tasks} message={taskStorageMessage} storage={storageUsage} activeTaskId={activeRef.current?.taskId} clearing={clearingTasks} onResume={resumeStoredTask} onDiscard={(task) => void discardTask(task)} onClear={() => void clearTasks()} />
            ) : (
              <AboutWorkspace capabilities={capabilities} pwa={pwa} pwaMessage={pwaMessage} onApplyUpdate={() => void applyUpdate()} active={active} />
            )}

            {progress && workspace === activeOperation ? (
              <section className="progress-card" aria-live="polite">
                <div className="progress-card__heading">
                  <div><strong>{progress.message}</strong><span>{formatBytes(progress.bytesProcessed)} of {formatBytes(progress.totalBytes)} · {formatSpeed(progress.speedBytesPerSecond)}</span></div>
                  <span className="slice-counter">Slice {progress.currentSlice} / {progress.sliceCount}</span>
                </div>
                <ProgressMeter value={progressPercent} label={`${Math.round(progressPercent)}% complete`} />
              </section>
            ) : null}

            {errorMessage ? <Notice tone="danger" title={taskState === 'cancelled' ? 'Operation cancelled' : 'Needs attention'} message={errorMessage} /> : null}
            {resultMessage ? <Notice tone={resultTone} title={resultTone === 'success' ? 'Verified result' : 'Incomplete result'} message={resultMessage} /> : null}
            {inspection && (workspace === 'inspect' || workspace === 'merge') ? <InspectionLedger inspection={inspection} /> : null}

            {workspace === 'split' || workspace === 'merge' || workspace === 'inspect' ? (
              <div className="action-row">
                <div className="action-copy">
                  {workspace === 'split'
                    ? splitFile
                      ? `${splitFile.name} · ${formatBytes(splitFile.size)} · ${estimatedSlices} planned ${estimatedSlices === 1 ? 'Slice' : 'Slices'}`
                      : 'Choose one Cake to calculate a bounded Slice plan.'
                    : parsedManifest
                      ? `${parsedManifest.original.filename} · ${parsedManifest.sliceCount} expected Slices`
                      : 'Choose a Cake Manifest and its expected Slices.'}
                </div>
                <div className="task-controls">
                  {taskState === 'running' ? <button className="button button--secondary" type="button" onClick={() => control('pause')}><PauseIcon />Pause</button> : null}
                  {taskState === 'paused' ? <button className="button button--secondary" type="button" onClick={() => control('resume')}><PlayIcon />Resume</button> : null}
                  {active ? <button className="button button--danger" type="button" onClick={() => control('cancel')}>Cancel safely</button> : (
                    <button
                      className="button button--primary"
                      type="button"
                      onClick={workspace === 'split' ? runSplit : workspace === 'merge' ? runMerge : runInspect}
                      disabled={workspace === 'split' ? !splitFile : !parsedManifest}
                    >
                      {workspace === 'split' ? 'Cut the Cake' : workspace === 'merge' ? 'Layer the Cake' : 'Inspect Package'}
                      <ArrowIcon />
                    </button>
                  )}
                </div>
              </div>
            ) : null}
          </div>
        </section>

        <section className="capability-strip" aria-labelledby="capability-title">
          <div><span className="step-label">Browser capability</span><h2 id="capability-title">{capabilities.supported ? 'Secure Direct Folder Mode available' : 'Compatibility Download Mode active'}</h2></div>
          <p>{capabilities.supported ? 'Large-file output can use bounded direct streaming with verified no-replace finalization.' : `${capabilities.reason} Compatibility limits are enforced before allocation; this app does not claim unlimited browser processing.`}</p>
        </section>
      </main>

      <footer><span>Early technical source release · MIT licensed</span><span>No accounts · No analytics · No telemetry</span></footer>
    </div>
  );
}

function SplitWorkspace({
  file,
  planningMode,
  sliceSize,
  requestedSliceCount,
  effectiveSliceSize,
  estimatedSlices,
  onFile,
  onDrop,
  onPlanningMode,
  onSliceSize,
  onSliceCount,
}: {
  file: File | undefined;
  planningMode: 'size' | 'count';
  sliceSize: number;
  requestedSliceCount: number;
  effectiveSliceSize: number;
  estimatedSlices: number;
  onFile: (file?: File) => void;
  onDrop: (event: DragEvent<HTMLLabelElement>) => void;
  onPlanningMode: (mode: 'size' | 'count') => void;
  onSliceSize: (value: number) => void;
  onSliceCount: (value: number) => void;
}) {
  return (
    <div className="split-grid">
      <label className="drop-zone" onDrop={onDrop} onDragOver={(event) => event.preventDefault()}>
        <input type="file" data-testid="split-file" onChange={(event) => onFile(event.target.files?.[0])} />
        <span className="drop-zone__icon"><FileIcon /></span>
        <strong>{file ? file.name : 'Drop one Cake here'}</strong>
        <span>{file ? formatBytes(file.size) : 'or choose a local file'}</span>
        <small>The original stays untouched.</small>
      </label>
      <div className="plan-card">
        <fieldset className="segmented-fieldset">
          <legend>Slice planning mode</legend>
          <div className="segmented-control">
            <button className={planningMode === 'size' ? 'preset preset--active' : 'preset'} type="button" aria-pressed={planningMode === 'size'} onClick={() => onPlanningMode('size')}>Target size</button>
            <button className={planningMode === 'count' ? 'preset preset--active' : 'preset'} type="button" aria-pressed={planningMode === 'count'} onClick={() => onPlanningMode('count')}>Slice count</button>
          </div>
        </fieldset>
        {planningMode === 'size' ? (
          <>
            <label className="field-label" htmlFor="slice-size">Target Slice size in bytes</label>
            <input id="slice-size" className="text-input numeric-input" type="number" min="1" step="1" value={sliceSize} onChange={(event) => onSliceSize(Number(event.target.value))} />
            <span className="field-help">Whole bytes. The final Slice may be smaller.</span>
            <div className="preset-row" aria-label="Common Slice sizes">
              {SLICE_PRESETS.map((preset) => <button key={preset.value} className={sliceSize === preset.value ? 'preset preset--active' : 'preset'} type="button" onClick={() => onSliceSize(preset.value)}>{preset.label}</button>)}
            </div>
          </>
        ) : (
          <>
            <label className="field-label" htmlFor="slice-count">Requested Slice count</label>
            <input id="slice-count" className="text-input numeric-input" type="number" min="1" step="1" value={requestedSliceCount} onChange={(event) => onSliceCount(Number(event.target.value))} />
            <span className="field-help">CakeSplitter derives a whole-byte Slice size; empty files remain zero-Slice packages.</span>
          </>
        )}
        <div className="plan-summary"><span>Estimated plan</span><strong>{file ? estimatedSlices : '—'} {file ? (estimatedSlices === 1 ? 'Slice' : 'Slices') : ''}</strong><small>{file ? `${formatBytes(effectiveSliceSize)} target size` : 'Select a source file'}</small></div>
      </div>
    </div>
  );
}

function PackageInputs({ manifestFile, parsedManifest, sliceFiles, readiness, onManifest, onSlices }: {
  manifestFile: File | undefined;
  parsedManifest: CakeManifest | undefined;
  sliceFiles: File[];
  readiness: PackageReadiness;
  onManifest: (event: ChangeEvent<HTMLInputElement>) => void;
  onSlices: (event: ChangeEvent<HTMLInputElement>) => void;
}) {
  return (
    <div className="package-grid">
      <FileInput label="Cake Manifest" helper={manifestFile ? manifestFile.name : 'Choose one .cake.json file'} accept=".json,.cake.json,application/json" data-testid="manifest-file" onChange={onManifest} />
      <FileInput label="Expected Slices" helper={`${sliceFiles.length} ${sliceFiles.length === 1 ? 'file' : 'files'} selected`} accept=".slice,application/octet-stream" multiple data-testid="slice-files" onChange={onSlices} />
      {parsedManifest ? (
        <>
          <dl className="manifest-summary">
            <div><dt>Original Cake</dt><dd>{parsedManifest.original.filename}</dd></div>
            <div><dt>Size</dt><dd>{formatBytes(parsedManifest.original.size)}</dd></div>
            <div><dt>Format version</dt><dd>{parsedManifest.version}</dd></div>
            <div><dt>Expected Slices</dt><dd>{parsedManifest.sliceCount}</dd></div>
          </dl>
          <div className="readiness-grid" aria-label="Package selection readiness">
            <ReadinessStat label="Missing" count={readiness.missing.length} />
            <ReadinessStat label="Size mismatch" count={readiness.sizeMismatch.length} />
            <ReadinessStat label="Duplicate" count={readiness.duplicate.length} />
            <ReadinessStat label="Unexpected" count={readiness.unexpected.length} />
          </div>
        </>
      ) : null}
    </div>
  );
}

function OutputModeCard({ capabilities, directory, mode, onMode, onSelect }: {
  capabilities: ReturnType<typeof getDirectFolderCapabilities>;
  directory: FileSystemDirectoryHandle | undefined;
  mode: 'direct' | 'fallback';
  onMode: (mode: 'direct' | 'fallback') => void;
  onSelect: () => Promise<void>;
}) {
  return (
    <fieldset className="mode-card mode-card--selectable">
      <legend>Output mode</legend>
      <label className="mode-option">
        <input type="radio" name="output-mode" checked={mode === 'fallback'} onChange={() => onMode('fallback')} />
        <DownloadIcon /><span><strong>Compatibility Download Mode</strong><small>Bounded browser downloads; Merge buffers the rebuilt output.</small></span>
      </label>
      <label className={capabilities.supported ? 'mode-option' : 'mode-option mode-option--disabled'}>
        <input type="radio" name="output-mode" checked={mode === 'direct'} disabled={!capabilities.supported} onChange={() => onMode('direct')} />
        <FolderIcon /><span><strong>Direct Folder Mode</strong><small>{capabilities.supported ? (directory ? `Authorized folder: ${directory.name}` : 'Select an explicit output directory.') : capabilities.reason}</small></span>
      </label>
      {capabilities.supported ? <button className="button button--secondary" type="button" onClick={() => void onSelect()}>{directory ? 'Change folder' : 'Select folder'}</button> : null}
    </fieldset>
  );
}

function FallbackWarning({ operation, size, activeSliceSize, downloadCount }: { operation: 'split' | 'merge'; size: number; activeSliceSize: number; downloadCount: number }) {
  const peak = operation === 'merge' ? size + 2 * 1024 * 1024 : Math.min(size, activeSliceSize) + 2 * 1024 * 1024;
  return (
    <div className="fallback-warning" role="note">
      <AlertIcon />
      <div><strong>Compatibility Mode estimate</strong><p>Estimated peak buffered data: about {formatBytes(peak)}. Expected downloads: {new Intl.NumberFormat().format(downloadCount)}. {operation === 'merge' ? 'The rebuilt result is buffered in memory.' : 'One completed Slice is buffered per download.'} Current desktop Chromium/Edge is recommended.</p></div>
    </div>
  );
}

function TasksWorkspace({ tasks, message, storage, activeTaskId, clearing, onResume, onDiscard, onClear }: {
  tasks: PersistedTask[];
  message: string;
  storage: { usage: number; quota?: number };
  activeTaskId: string | undefined;
  clearing: boolean;
  onResume: (task: PersistedTask) => void;
  onDiscard: (task: PersistedTask) => void;
  onClear: () => void;
}) {
  return (
    <section className="tasks-workspace" aria-labelledby="tasks-title">
      <div className="storage-summary"><div><strong id="tasks-title">Browser-local task storage</strong><span>{message}</span></div><div className="storage-usage"><span>{formatBytes(storage.usage)} used</span>{storage.quota !== undefined ? <span>{formatBytes(storage.quota)} quota</span> : null}</div><button className="button button--danger button--compact" type="button" onClick={onClear} disabled={clearing}><TrashIcon />{clearing ? 'Clearing local data…' : 'Clear all local data'}</button></div>
      {tasks.length ? <div className="task-list">{tasks.map((task) => (
        <article className="task-card" key={task.taskId}>
          <div className="task-card__heading"><div><span className="step-label">{capitalize(task.operation)} · {task.outputMode}</span><h3>{task.originalFilename}</h3></div><StatusBadge tone={statusTone(task.status)}>{statusLabel(task.status)}</StatusBadge></div>
          <dl className="task-facts"><div><dt>Expected size</dt><dd>{formatBytes(task.expectedSize)}</dd></div><div><dt>Slices recorded</dt><dd>{task.completedSliceIndexes.length} / {task.sliceCount}</dd></div><div><dt>Updated</dt><dd>{new Date(task.updatedAt).toLocaleString()}</dd></div><div><dt>Task ID</dt><dd className="tabular">{task.taskId.slice(0, 12)}…</dd></div></dl>
          <p>{recoveryDescription(task)}</p>
          <div className="task-card__actions">
            {canRecover(task) && activeTaskId !== task.taskId ? <button className="button button--secondary button--compact" type="button" onClick={() => onResume(task)}><HistoryIcon />Reselect and restart safely</button> : null}
            <button className="button button--ghost button--compact" type="button" onClick={() => onDiscard(task)} disabled={activeTaskId === task.taskId}>Discard metadata</button>
          </div>
        </article>
      ))}</div> : <div className="empty-state"><TasksIcon /><h3>No stored tasks</h3><p>Interrupted and completed task metadata appears here. User file contents are never stored automatically.</p></div>}
    </section>
  );
}

function AboutWorkspace({ capabilities, pwa, pwaMessage, onApplyUpdate, active }: {
  capabilities: ReturnType<typeof getDirectFolderCapabilities>;
  pwa: PwaSnapshot;
  pwaMessage: string;
  onApplyUpdate: () => void;
  active: boolean;
}) {
  const rows = [
    ['Secure context', capabilities.secureContext],
    ['Open file picker', capabilities.openFilePicker],
    ['Directory picker', capabilities.directoryPicker],
    ['Writable streams', capabilities.writableStream],
    ['Handle identity', capabilities.handleIdentity],
    ['File move', capabilities.move],
    ['Atomic no-replace', capabilities.atomicNoReplace],
  ] as const;
  return (
    <div className="about-grid">
      <section className="about-card"><span className="step-label">Privacy model</span><h3>Local means local</h3><p>The application has no upload, account, analytics, telemetry, remote checksum, or remote error endpoint. File contents, names, manifests, hashes, task metadata, handles, and private errors are not transmitted.</p></section>
      <section className="about-card"><span className="step-label">PWA and offline</span><h3>{pwa.online ? 'Online shell' : 'Offline shell'}</h3><p>{pwa.installed ? 'The application shell is controlled by the CakeSplitter service worker.' : 'The browser can install the application shell after its first successful load.'} User-selected files are never service-worker cache entries.</p>{pwa.updateAvailable ? <button className="button button--secondary" type="button" onClick={onApplyUpdate} disabled={active}>Apply update safely</button> : null}{pwaMessage ? <p className="field-help">{pwaMessage}</p> : null}</section>
      <section className="about-card about-card--wide"><span className="step-label">Capability evidence</span><h3>Direct Folder security gate</h3><p>{capabilities.reason}</p><div className="capability-grid">{rows.map(([label, available]) => <div key={label}><span>{label}</span><strong>{available ? 'Available' : 'Unavailable'}</strong></div>)}</div></section>
      <section className="about-card about-card--wide"><span className="step-label">Prototype boundaries</span><h3>Source-only browser release</h3><p>CakeSplitter Desktop does not exist in this release. Cake Package is a project format, not an industry standard. There is no cloud, compression, encryption, marketplace, plugin execution, user account, or digital signature feature.</p></section>
    </div>
  );
}

function InspectionLedger({ inspection }: { inspection: InspectionResult }) {
  return (
    <section className="inspection-ledger" aria-labelledby="ledger-title">
      <div className="ledger-heading"><div><span className="step-label">Slice ledger</span><h3 id="ledger-title">{inspection.verified ? 'Package verified' : 'Package needs attention'}</h3></div><StatusBadge tone={inspection.verified ? 'success' : 'danger'}>{inspection.foundSliceCount} found / {inspection.manifest.sliceCount} expected</StatusBadge></div>
      <div className="ledger-stats"><span><strong>{inspection.missing.length}</strong> missing</span><span><strong>{inspection.corrupted.length}</strong> damaged</span><span><strong>{inspection.duplicates.length}</strong> duplicate</span><span><strong>{inspection.unexpected.length}</strong> unexpected</span></div>
      <div className="table-scroll"><table><thead><tr><th>Slice</th><th>Filename</th><th>State</th><th>Evidence</th></tr></thead><tbody>{inspection.slices.map((slice) => <tr key={`${slice.index}-${slice.filename}`}><td className="tabular">{String(slice.index).padStart(3, '0')}</td><td>{slice.filename}</td><td><StatusBadge tone={slice.state === 'verified' ? 'success' : 'danger'}>{slice.state}</StatusBadge></td><td>{slice.detail}</td></tr>)}</tbody></table></div>
      {inspection.unexpected.length ? <p className="ledger-note"><strong>Unexpected:</strong> {inspection.unexpected.join(', ')}</p> : null}
    </section>
  );
}

function Notice({ tone, title, message }: { tone: 'success' | 'danger'; title: string; message: string }) {
  return <div className={`notice notice--${tone}`} role={tone === 'success' ? 'status' : 'alert'}>{tone === 'success' ? <CheckIcon /> : <AlertIcon />}<div><strong>{title}</strong><p>{message}</p></div></div>;
}

function ReadinessStat({ label, count }: { label: string; count: number }) {
  return <div className={count ? 'readiness-stat readiness-stat--danger' : 'readiness-stat'}><strong>{count}</strong><span>{label}</span></div>;
}

function TrustItem({ icon, title, detail }: { icon: React.ReactNode; title: string; detail: string }) {
  return <div className="trust-item"><span>{icon}</span><div><strong>{title}</strong><small>{detail}</small></div></div>;
}

function analyzePackageReadiness(manifest: CakeManifest | undefined, files: File[]): PackageReadiness {
  if (!manifest) return { missing: [], duplicate: [], sizeMismatch: [], unexpected: [] };
  const expected = new Map(manifest.slices.map((slice) => [slice.filename, slice]));
  const selected = new Map<string, File[]>();
  for (const file of files) {
    const values = selected.get(file.name) ?? [];
    values.push(file);
    selected.set(file.name, values);
  }
  return {
    missing: manifest.slices.filter((slice) => !selected.has(slice.filename)).map((slice) => slice.filename),
    duplicate: [...selected.entries()].filter(([name, values]) => expected.has(name) && values.length > 1).map(([name]) => name),
    sizeMismatch: [...selected.entries()].filter(([name, values]) => {
      const entry = expected.get(name);
      return entry !== undefined && values.length === 1 && values[0]?.size !== entry.size;
    }).map(([name]) => name),
    unexpected: files.filter((file) => file.name.endsWith('.slice') && !expected.has(file.name)).map((file) => file.name),
  };
}

function packageTaskMetadata(operation: 'merge' | 'inspect', outputMode: OutputMode, manifest: CakeManifest, capabilityReason: string): Omit<PersistedTask, 'taskId' | 'status' | 'createdAt' | 'updatedAt'> {
  return {
    schemaVersion: 1,
    operation,
    packageId: manifest.packageId,
    originalFilename: manifest.original.filename,
    expectedSize: manifest.original.size,
    expectedSha256: manifest.original.sha256,
    sliceSize: manifest.targetSliceSize,
    sliceCount: manifest.sliceCount,
    completedSliceIndexes: [],
    outputMode,
    capability: { directFolder: outputMode === 'direct', reason: capabilityReason },
    recovery: { sourceFile: false, manifest: true, slices: true, outputDirectory: outputMode === 'direct' },
  };
}

function downloadBlob(filename: string, blob: Blob) {
  const url = URL.createObjectURL(blob);
  const anchor = document.createElement('a');
  anchor.href = url;
  anchor.download = filename;
  anchor.click();
  window.setTimeout(() => URL.revokeObjectURL(url), 1_000);
}

function formatBytes(bytes: number): string {
  if (bytes === 0) return '0 bytes';
  const units = ['bytes', 'KiB', 'MiB', 'GiB', 'TiB'];
  const exponent = Math.min(Math.floor(Math.log(bytes) / Math.log(1024)), units.length - 1);
  const value = bytes / 1024 ** exponent;
  return `${new Intl.NumberFormat(undefined, { maximumFractionDigits: exponent === 0 ? 0 : 2 }).format(value)} ${units[exponent]}`;
}

function formatSpeed(bytesPerSecond: number): string { return `${formatBytes(bytesPerSecond)}/s`; }
function capitalize(value: string) { return value.charAt(0).toUpperCase() + value.slice(1); }
function errorMessageOf(error: unknown) { return error instanceof Error ? error.message : 'Unknown browser error.'; }
function canRecover(task: PersistedTask) { return ['interrupted', 'permission-required', 'incomplete', 'failed', 'cancelled'].includes(task.status); }
function recoveryDescription(task: PersistedTask) {
  if (task.status === 'completed') return 'Verified completion metadata. Discard it when no longer needed.';
  if (task.status === 'interrupted') return 'The browser session ended before a terminal state. Reselect inputs to restart safely.';
  if (task.status === 'permission-required') return 'Output permission must be granted again before processing can continue.';
  if (task.status === 'incomplete') return 'Incomplete output was never marked verified. Reselection and revalidation are required.';
  return 'This task can restart only after its required local inputs are reselected and checked.';
}

function statusTone(state: TaskStatus): 'neutral' | 'working' | 'success' | 'danger' {
  if (state === 'running') return 'working';
  if (state === 'completed') return 'success';
  if (['failed', 'cancelled', 'incomplete'].includes(state)) return 'danger';
  return 'neutral';
}
function statusLabel(state: TaskStatus) {
  return ({ planned: 'Ready', running: 'Processing', paused: 'Paused', interrupted: 'Interrupted', 'permission-required': 'Permission required', incomplete: 'Incomplete', failed: 'Failed', completed: 'Completed', cancelled: 'Cancelled' } as const)[state];
}

const WORKSPACE_COPY = {
  split: { step: 'Workspace 01', title: 'Split a Cake', description: 'Choose a size or count plan, review memory and output mode, then process in bounded Worker chunks.' },
  merge: { step: 'Workspace 02', title: 'Merge a Cake Package', description: 'Analyze completeness first, verify each Slice, and rebuild only when every byte is trusted.' },
  inspect: { step: 'Workspace 03', title: 'Inspect a Cake Package', description: 'Review package metadata, Slice evidence, and readiness without writing output.' },
  tasks: { step: 'Workspace 04', title: 'Recover and clean up tasks', description: 'Inspect browser-local metadata, reselect required inputs, and remove controlled local state.' },
  about: { step: 'Workspace 05', title: 'Capabilities, privacy, and offline', description: 'See exactly what this browser supports and which security boundaries remain fail-closed.' },
} satisfies Record<Workspace, { step: string; title: string; description: string }>;

const iconProps = { width: 22, height: 22, viewBox: '0 0 24 24', fill: 'none', stroke: 'currentColor', strokeWidth: 1.8, strokeLinecap: 'round' as const, strokeLinejoin: 'round' as const, 'aria-hidden': true };
function CakeMark() { return <svg className="cake-mark" {...iconProps} viewBox="0 0 42 42"><path d="M8 15.5h26v16.2c0 2-1.6 3.6-3.6 3.6H11.6A3.6 3.6 0 0 1 8 31.7V15.5Z"/><path d="M8 23h26M14 15.5V11a7 7 0 0 1 14 0v4.5"/><path d="M15 27.5h3m6 0h3"/></svg>; }
function DeviceIcon() { return <svg {...iconProps}><rect x="4" y="3" width="16" height="18" rx="2"/><path d="M8 7h8m-8 4h5m-5 6h8"/></svg>; }
function HashIcon() { return <svg {...iconProps}><path d="m10 3-2 18m8-18-2 18M4 9h16M3 15h16"/></svg>; }
function NoCloudIcon() { return <svg {...iconProps}><path d="M6.5 17.5H5a4 4 0 0 1-.6-8A7 7 0 0 1 17 7.5a5 5 0 0 1 2.2 8.8M3 3l18 18"/></svg>; }
function CutIcon() { return <svg {...iconProps}><circle cx="6" cy="7" r="3"/><circle cx="6" cy="17" r="3"/><path d="m8.7 8.4 11.3 6.2M8.7 15.6 20 9.4"/></svg>; }
function LayersIcon() { return <svg {...iconProps}><path d="m12 3 9 5-9 5-9-5 9-5Z"/><path d="m3 12 9 5 9-5M3 16l9 5 9-5"/></svg>; }
function InspectIcon() { return <svg {...iconProps}><circle cx="11" cy="11" r="7"/><path d="m20 20-4-4m-8-5 2 2 4-4"/></svg>; }
function FolderIcon() { return <svg {...iconProps}><path d="M3 6.5h7l2 2h9v9.5a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V6.5Z"/></svg>; }
function FileIcon() { return <svg {...iconProps} width="30" height="30"><path d="M6 2h8l4 4v16H6V2Z"/><path d="M14 2v5h5M9 13h6m-6 4h4"/></svg>; }
function AlertIcon() { return <svg {...iconProps}><path d="M12 3 2.5 20h19L12 3Z"/><path d="M12 9v4m0 3h.01"/></svg>; }
function CheckIcon() { return <svg {...iconProps}><circle cx="12" cy="12" r="9"/><path d="m8 12 2.5 2.5L16.5 9"/></svg>; }
function ArrowIcon() { return <svg {...iconProps}><path d="M5 12h14m-5-5 5 5-5 5"/></svg>; }
function TasksIcon() { return <svg {...iconProps}><path d="M5 4h14v16H5zM8 8h8m-8 4h8m-8 4h5"/></svg>; }
function InfoIcon() { return <svg {...iconProps}><circle cx="12" cy="12" r="9"/><path d="M12 11v6m0-10h.01"/></svg>; }
function PauseIcon() { return <svg {...iconProps}><path d="M8 5v14m8-14v14"/></svg>; }
function PlayIcon() { return <svg {...iconProps}><path d="m8 5 11 7-11 7V5Z"/></svg>; }
function HistoryIcon() { return <svg {...iconProps}><path d="M4 12a8 8 0 1 0 2.3-5.7L4 8.5M4 4v4.5h4.5M12 8v5l3 2"/></svg>; }
function TrashIcon() { return <svg {...iconProps}><path d="M4 7h16M9 3h6l1 4H8l1-4Zm-3 4 1 14h10l1-14M10 11v6m4-6v6"/></svg>; }
function DownloadIcon() { return <svg {...iconProps}><path d="M12 3v12m-5-5 5 5 5-5M5 20h14"/></svg>; }

const NAVIGATION = [
  { id: 'split', label: 'Split', icon: <CutIcon /> },
  { id: 'merge', label: 'Merge', icon: <LayersIcon /> },
  { id: 'inspect', label: 'Inspect', icon: <InspectIcon /> },
  { id: 'tasks', label: 'Tasks', icon: <TasksIcon /> },
  { id: 'about', label: 'About', icon: <InfoIcon /> },
] satisfies Array<{ id: Workspace; label: string; icon: React.ReactNode }>;
