import { useMemo, useRef, useState, type ChangeEvent, type DragEvent } from 'react';

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
import { supportsDirectFolderMode } from '@cakesplitter/web-file-io';

import {
  parseWorkerResponse,
  type InspectionResult,
  type WorkerRequest,
  type WorkerResponse,
  type Workspace,
} from './protocol';

type TaskState = 'idle' | 'working' | 'completed' | 'failed' | 'cancelled';

interface ProgressState {
  bytesProcessed: number;
  totalBytes: number;
  currentSlice: number;
  sliceCount: number;
  message: string;
}

const SLICE_PRESETS = [
  { label: '10 MiB', value: 10 * 1024 * 1024 },
  { label: '100 MiB', value: 100 * 1024 * 1024 },
  { label: '650 MiB', value: 650 * 1024 * 1024 },
];

export function App() {
  const [workspace, setWorkspace] = useState<Workspace>('split');
  const [splitFile, setSplitFile] = useState<File>();
  const [sliceSize, setSliceSize] = useState(100 * 1024 * 1024);
  const [manifestFile, setManifestFile] = useState<File>();
  const [manifestText, setManifestText] = useState('');
  const [parsedManifest, setParsedManifest] = useState<CakeManifest>();
  const [sliceFiles, setSliceFiles] = useState<File[]>([]);
  const [directory, setDirectory] = useState<FileSystemDirectoryHandle>();
  const [taskState, setTaskState] = useState<TaskState>('idle');
  const [progress, setProgress] = useState<ProgressState>();
  const [inspection, setInspection] = useState<InspectionResult>();
  const [resultMessage, setResultMessage] = useState('');
  const [resultTone, setResultTone] = useState<'success' | 'danger'>('success');
  const [errorMessage, setErrorMessage] = useState('');
  const workerRef = useRef<Worker | undefined>(undefined);
  const directFolderSupported = useMemo(() => supportsDirectFolderMode(), []);

  const estimatedSlices = useMemo(() => {
    if (!splitFile || !Number.isSafeInteger(sliceSize) || sliceSize < 1) {
      return 0;
    }
    return expectedSliceCount(splitFile.size, sliceSize);
  }, [sliceSize, splitFile]);

  const selectDirectory = async () => {
    try {
      const handle = await window.showDirectoryPicker({
        id: 'splitthecake-output',
        mode: 'readwrite',
      });
      setDirectory(handle);
    } catch (error) {
      if (!(error instanceof DOMException && error.name === 'AbortError')) {
        setErrorMessage(error instanceof Error ? error.message : 'Could not select the folder.');
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
    if (!file) {
      return;
    }
    try {
      if (file.size > MAX_MANIFEST_BYTES) {
        throw new Error(`Cake Manifest exceeds the ${formatBytes(MAX_MANIFEST_BYTES)} limit.`);
      }
      const text = await file.text();
      const manifest = parseManifest(text);
      setManifestText(text);
      setParsedManifest(manifest);
    } catch (error) {
      setErrorMessage(error instanceof Error ? error.message : 'The Cake Manifest is invalid.');
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

  const startWorker = (request: Exclude<WorkerRequest, { type: 'cancel' }>) => {
    workerRef.current?.terminate();
    const activeWorker = new Worker(new URL('./worker.ts', import.meta.url), { type: 'module' });
    workerRef.current = activeWorker;
    setTaskState('working');
    setProgress(undefined);
    setInspection(undefined);
    setResultMessage('');
    setResultTone('success');
    setErrorMessage('');
    activeWorker.addEventListener('message', (event: MessageEvent<unknown>) => {
      let message: WorkerResponse;
      try {
        message = parseWorkerResponse(event.data);
      } catch (error) {
        setTaskState('failed');
        setErrorMessage(error instanceof Error ? error.message : 'The Worker sent an invalid response.');
        activeWorker.terminate();
        workerRef.current = undefined;
        return;
      }
      if (message.type === 'progress') {
        setProgress(message);
        return;
      }
      if (message.type === 'download') {
        downloadBlob(message.filename, message.blob);
        return;
      }
      if (message.type === 'result') {
        const inspectionFailed = message.inspection?.verified === false;
        setTaskState(inspectionFailed ? 'failed' : 'completed');
        setResultTone(inspectionFailed ? 'danger' : 'success');
        setResultMessage(message.message);
        if (message.inspection) {
          setInspection(message.inspection);
        }
        activeWorker.terminate();
        workerRef.current = undefined;
        return;
      }
      setTaskState(message.state);
      setErrorMessage(message.message);
      activeWorker.terminate();
      workerRef.current = undefined;
    });
    activeWorker.addEventListener('error', (event) => {
      setTaskState('failed');
      setErrorMessage(event.message || 'The processing worker stopped unexpectedly.');
      activeWorker.terminate();
      workerRef.current = undefined;
    });
    activeWorker.postMessage(request);
  };

  const cancel = () => {
    workerRef.current?.postMessage({ type: 'cancel' } satisfies WorkerRequest);
  };

  const runSplit = () => {
    if (!splitFile || !Number.isSafeInteger(sliceSize) || sliceSize < 1) {
      setErrorMessage('Choose one Cake and enter a whole-number Slice size greater than zero.');
      return;
    }
    if (splitFile.size > MAX_BROWSER_FALLBACK_BYTES) {
      setErrorMessage(
        `Compatibility Split is limited to ${formatBytes(MAX_BROWSER_FALLBACK_BYTES)} to keep browser memory bounded.`,
      );
      return;
    }
    if (estimatedSlices > MAX_BROWSER_FALLBACK_DOWNLOADS) {
      setErrorMessage(
        `Compatibility Split supports at most ${MAX_BROWSER_FALLBACK_DOWNLOADS} downloaded files per operation.`,
      );
      return;
    }
    startWorker({
      type: 'split',
      file: splitFile,
      sliceSize,
      ...(directory ? { directory } : {}),
    });
  };

  const runInspect = () => {
    if (!manifestText) {
      setErrorMessage('Choose a valid .cake.json manifest first.');
      return;
    }
    startWorker({ type: 'inspect', manifestText, files: sliceFiles });
  };

  const runMerge = () => {
    if (!manifestText) {
      setErrorMessage('Choose a valid .cake.json manifest first.');
      return;
    }
    if ((parsedManifest?.original.size ?? 0) > MAX_BROWSER_FALLBACK_BYTES) {
      setErrorMessage(
        `Compatibility Merge is limited to ${formatBytes(MAX_BROWSER_FALLBACK_BYTES)} because it buffers the rebuilt Cake in memory.`,
      );
      return;
    }
    startWorker({
      type: 'merge',
      manifestText,
      files: sliceFiles,
      ...(directory ? { directory } : {}),
    });
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

  return (
    <div className="app-shell">
      <header className="site-header">
        <a className="brand" href="#workspace" aria-label="SplitTheCake home">
          <CakeMark />
          <span>
            <strong>SplitTheCake</strong>
            <small>CakeSplitter v0.3.0-dev</small>
          </span>
        </a>
        <div className="header-status" aria-label="Application mode">
          <StatusBadge tone="neutral">Local-only prototype</StatusBadge>
          <span className="version-chip">Format 1.0</span>
        </div>
      </header>

      <main id="workspace" className="main-layout" tabIndex={-1}>
        <section className="intro" aria-labelledby="page-title">
          <div className="eyebrow">Large files, cut with proof</div>
          <h1 id="page-title">Cut a Cake into verified Slices. Layer it back exactly.</h1>
          <p>
            A local processing workbench for splitting one file, checking every piece, and
            rebuilding the original byte-for-byte.
          </p>
          <div className="trust-row">
            <TrustItem icon={<DeviceIcon />} title="Browser worker" detail="Heavy work stays off the interface thread." />
            <TrustItem icon={<HashIcon />} title="SHA-256 proof" detail="Each Slice and rebuilt Cake is verified." />
            <TrustItem icon={<NoCloudIcon />} title="No cloud fallback" detail="This prototype has no upload path or account." />
          </div>
          <p className="privacy-proof">
            <CheckIcon />
            <strong>Processed locally in your browser. Your files never leave your device.</strong>
          </p>
        </section>

        <section className="workbench" aria-label="Cake processing workbench">
          <nav className="workspace-tabs" aria-label="Workspaces">
            {(['split', 'merge', 'inspect'] as const).map((item) => (
              <button
                key={item}
                className={workspace === item ? 'workspace-tab workspace-tab--active' : 'workspace-tab'}
                type="button"
                aria-current={workspace === item ? 'page' : undefined}
                onClick={() => {
                  setWorkspace(item);
                  setTaskState('idle');
                  setErrorMessage('');
                  setResultMessage('');
                  setInspection(undefined);
                }}
              >
                {item === 'split' ? <CutIcon /> : item === 'merge' ? <LayersIcon /> : <InspectIcon />}
                <span>{capitalize(item)}</span>
              </button>
            ))}
          </nav>

          <div className="workspace-panel">
            <div className="panel-heading">
              <div>
                <span className="step-label">{workspaceCopy[workspace].step}</span>
                <h2>{workspaceCopy[workspace].title}</h2>
                <p>{workspaceCopy[workspace].description}</p>
              </div>
              <StatusBadge tone={statusTone(taskState)}>{statusLabel(taskState)}</StatusBadge>
            </div>

            {workspace === 'split' ? (
              <SplitWorkspace
                file={splitFile}
                sliceSize={sliceSize}
                estimatedSlices={estimatedSlices}
                onFile={setSplitFile}
                onDrop={onDrop}
                onSliceSize={setSliceSize}
              />
            ) : (
              <PackageInputs
                manifestFile={manifestFile}
                parsedManifest={parsedManifest}
                sliceFiles={sliceFiles}
                onManifest={handleManifest}
                onSlices={handleSlices}
              />
            )}

            <OutputMode
              supported={directFolderSupported}
              directory={directory}
              onSelect={selectDirectory}
              readOnly={workspace === 'inspect'}
            />

            {progress ? (
              <section className="progress-card" aria-live="polite">
                <div className="progress-card__heading">
                  <div>
                    <strong>{progress.message}</strong>
                    <span>
                      {formatBytes(progress.bytesProcessed)} of {formatBytes(progress.totalBytes)}
                    </span>
                  </div>
                  <span className="slice-counter">
                    Slice {progress.currentSlice} / {progress.sliceCount}
                  </span>
                </div>
                <ProgressMeter value={progressPercent} label={`${Math.round(progressPercent)}% complete`} />
              </section>
            ) : null}

            {errorMessage ? (
              <div className="notice notice--danger" role="alert">
                <AlertIcon />
                <div>
                  <strong>{taskState === 'cancelled' ? 'Operation cancelled' : 'Needs attention'}</strong>
                  <p>{errorMessage}</p>
                </div>
              </div>
            ) : null}
            {resultMessage ? (
              <div
                className={`notice notice--${resultTone === 'success' ? 'success' : 'danger'}`}
                role={resultTone === 'success' ? 'status' : 'alert'}
              >
                {resultTone === 'success' ? <CheckIcon /> : <AlertIcon />}
                <div>
                  <strong>{resultTone === 'success' ? 'Verified result' : 'Inspection failed'}</strong>
                  <p>{resultMessage}</p>
                </div>
              </div>
            ) : null}

            {inspection ? <InspectionLedger inspection={inspection} /> : null}

            <div className="action-row">
              <div className="action-copy">
                {workspace === 'split'
                  ? splitFile
                    ? `${splitFile.name} · ${formatBytes(splitFile.size)} · ${estimatedSlices} planned ${estimatedSlices === 1 ? 'Slice' : 'Slices'}`
                    : 'Choose one Cake to calculate a Slice plan.'
                  : parsedManifest
                    ? `${parsedManifest.original.filename} · ${parsedManifest.sliceCount} expected Slices`
                    : 'Choose a Cake Manifest and the expected Slices.'}
              </div>
              {taskState === 'working' ? (
                <button className="button button--danger" type="button" onClick={cancel}>
                  Cancel safely
                </button>
              ) : (
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
        </section>

        <section className="capability-strip" aria-labelledby="capability-title">
          <div>
            <span className="step-label">Browser capability</span>
            <h2 id="capability-title">
              {directFolderSupported ? 'Direct folder mode is available' : 'Compatibility download mode is active'}
            </h2>
          </div>
          <p>
            {directFolderSupported
              ? 'This Chromium browser exposes folder writing and file rename. Outputs use .partial names until validation succeeds.'
              : `Direct folder output is disabled until browsers can guarantee exclusive creation and no-replace publication. Split and Merge downloads are limited to ${formatBytes(MAX_BROWSER_FALLBACK_BYTES)}; Merge buffers the rebuilt Cake in memory.`}
          </p>
        </section>
      </main>

      <footer>
        <span>Early technical prototype · MIT licensed</span>
        <span>No accounts · No analytics · No telemetry</span>
      </footer>
    </div>
  );
}

function SplitWorkspace({
  file,
  sliceSize,
  estimatedSlices,
  onFile,
  onDrop,
  onSliceSize,
}: {
  file: File | undefined;
  sliceSize: number;
  estimatedSlices: number;
  onFile: (file?: File) => void;
  onDrop: (event: DragEvent<HTMLLabelElement>) => void;
  onSliceSize: (value: number) => void;
}) {
  return (
    <div className="split-grid">
      <label
        className="drop-zone"
        onDrop={onDrop}
        onDragOver={(event) => event.preventDefault()}
      >
        <input
          type="file"
          data-testid="split-file"
          onChange={(event) => onFile(event.target.files?.[0])}
        />
        <span className="drop-zone__icon"><FileIcon /></span>
        <strong>{file ? file.name : 'Drop one Cake here'}</strong>
        <span>{file ? formatBytes(file.size) : 'or choose a local file'}</span>
        <small>The original stays untouched.</small>
      </label>
      <div className="plan-card">
        <label className="field-label" htmlFor="slice-size">Target Slice size in bytes</label>
        <input
          id="slice-size"
          className="text-input numeric-input"
          type="number"
          min="1"
          step="1"
          value={sliceSize}
          onChange={(event) => onSliceSize(Number(event.target.value))}
        />
        <span className="field-help">Whole bytes. The final Slice may be smaller.</span>
        <div className="preset-row" aria-label="Common Slice sizes">
          {SLICE_PRESETS.map((preset) => (
            <button
              key={preset.value}
              className={sliceSize === preset.value ? 'preset preset--active' : 'preset'}
              type="button"
              onClick={() => onSliceSize(preset.value)}
            >
              {preset.label}
            </button>
          ))}
        </div>
        <div className="plan-summary">
          <span>Estimated plan</span>
          <strong>{file ? estimatedSlices : '—'} {file ? (estimatedSlices === 1 ? 'Slice' : 'Slices') : ''}</strong>
        </div>
      </div>
    </div>
  );
}

function PackageInputs({
  manifestFile,
  parsedManifest,
  sliceFiles,
  onManifest,
  onSlices,
}: {
  manifestFile: File | undefined;
  parsedManifest: CakeManifest | undefined;
  sliceFiles: File[];
  onManifest: (event: ChangeEvent<HTMLInputElement>) => void;
  onSlices: (event: ChangeEvent<HTMLInputElement>) => void;
}) {
  return (
    <div className="package-grid">
      <FileInput
        label="Cake Manifest"
        helper={manifestFile ? manifestFile.name : 'Choose one .cake.json file'}
        accept=".json,.cake.json,application/json"
        data-testid="manifest-file"
        onChange={onManifest}
      />
      <FileInput
        label="Expected Slices"
        helper={`${sliceFiles.length} ${sliceFiles.length === 1 ? 'file' : 'files'} selected`}
        accept=".slice,application/octet-stream"
        multiple
        data-testid="slice-files"
        onChange={onSlices}
      />
      {parsedManifest ? (
        <dl className="manifest-summary">
          <div><dt>Original Cake</dt><dd>{parsedManifest.original.filename}</dd></div>
          <div><dt>Size</dt><dd>{formatBytes(parsedManifest.original.size)}</dd></div>
          <div><dt>Package version</dt><dd>{parsedManifest.version}</dd></div>
          <div><dt>Expected Slices</dt><dd>{parsedManifest.sliceCount}</dd></div>
        </dl>
      ) : null}
    </div>
  );
}

function OutputMode({
  supported,
  directory,
  onSelect,
  readOnly,
}: {
  supported: boolean;
  directory: FileSystemDirectoryHandle | undefined;
  onSelect: () => Promise<void>;
  readOnly: boolean;
}) {
  if (readOnly) {
    return (
      <div className="mode-card mode-card--read-only">
        <InspectIcon />
        <div><strong>Read-only inspection</strong><span>No rebuilt file is written.</span></div>
      </div>
    );
  }
  return (
    <div className="mode-card">
      <FolderIcon />
      <div>
        <strong>{directory ? `Direct folder: ${directory.name}` : supported ? 'Choose a direct output folder' : 'Compatibility downloads'}</strong>
        <span>
          {directory
            ? 'Atomic .partial workflow is enabled.'
            : supported
              ? 'Without a folder, outputs use the compatibility download fallback.'
              : 'Direct writes are intentionally disabled; bounded compatibility downloads are used.'}
        </span>
      </div>
      {supported ? (
        <button className="button button--secondary" type="button" onClick={onSelect}>
          {directory ? 'Change folder' : 'Select folder'}
        </button>
      ) : null}
    </div>
  );
}

function InspectionLedger({ inspection }: { inspection: InspectionResult }) {
  return (
    <section className="inspection-ledger" aria-labelledby="ledger-title">
      <div className="ledger-heading">
        <div>
          <span className="step-label">Slice ledger</span>
          <h3 id="ledger-title">{inspection.verified ? 'Package verified' : 'Package needs attention'}</h3>
        </div>
        <StatusBadge tone={inspection.verified ? 'success' : 'danger'}>
          {inspection.foundSliceCount} found / {inspection.manifest.sliceCount} expected
        </StatusBadge>
      </div>
      <div className="ledger-stats">
        <span><strong>{inspection.missing.length}</strong> missing</span>
        <span><strong>{inspection.corrupted.length}</strong> damaged</span>
        <span><strong>{inspection.duplicates.length}</strong> duplicate</span>
        <span><strong>{inspection.unexpected.length}</strong> unexpected</span>
      </div>
      <div className="table-scroll">
        <table>
          <thead><tr><th>Slice</th><th>Filename</th><th>State</th><th>Evidence</th></tr></thead>
          <tbody>
            {inspection.slices.map((slice) => (
              <tr key={`${slice.index}-${slice.filename}`}>
                <td className="tabular">{String(slice.index).padStart(3, '0')}</td>
                <td>{slice.filename}</td>
                <td><StatusBadge tone={slice.state === 'verified' ? 'success' : 'danger'}>{slice.state}</StatusBadge></td>
                <td>{slice.detail}</td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>
      {inspection.unexpected.length > 0 ? (
        <p className="ledger-note"><strong>Unexpected:</strong> {inspection.unexpected.join(', ')}</p>
      ) : null}
    </section>
  );
}

function TrustItem({ icon, title, detail }: { icon: React.ReactNode; title: string; detail: string }) {
  return <div className="trust-item"><span>{icon}</span><div><strong>{title}</strong><small>{detail}</small></div></div>;
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

function capitalize(value: string) {
  return value.charAt(0).toUpperCase() + value.slice(1);
}

function statusTone(state: TaskState): 'neutral' | 'working' | 'success' | 'danger' {
  if (state === 'working') return 'working';
  if (state === 'completed') return 'success';
  if (state === 'failed' || state === 'cancelled') return 'danger';
  return 'neutral';
}

function statusLabel(state: TaskState) {
  return ({ idle: 'Ready', working: 'Processing', completed: 'Completed', failed: 'Failed', cancelled: 'Cancelled' } as const)[state];
}

const workspaceCopy = {
  split: {
    step: 'Workspace 01',
    title: 'Cut one Cake',
    description: 'Choose a target size, preview the plan, then stream verified Slices and a Cake Manifest.',
  },
  merge: {
    step: 'Workspace 02',
    title: 'Layer the Cake back',
    description: 'Match Slices to a manifest, verify each one, and rebuild only when the package is complete.',
  },
  inspect: {
    step: 'Workspace 03',
    title: 'Inspect the package',
    description: 'Read package facts and verify every selected Slice without rebuilding the original file.',
  },
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
