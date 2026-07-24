"use client";

import { useEffect, useMemo, useRef, useState } from "react";
import type { ChangeEvent, DragEvent } from "react";
import Link from "next/link";

const sliceSizes = [1, 2, 3, 4, 6] as const;
const sliceBytesFor = (size: number) => size * 1024 ** 3;

type SliceRecord = { index: number; name: string; size: number; sha256: string };
type CakeManifest = {
  formatVersion: "1.0";
  originalName: string;
  originalSize: number;
  sliceSizeBytes: number;
  originalSha256: string;
  slices: SliceRecord[];
};
type PackageOutput = { file: File; manifest: CakeManifest };
type ProgressMessage = { type: "progress"; phase: string; processedBytes: number; totalBytes: number; currentIndex: number; totalSlices: number };
type WorkerMessage = ProgressMessage | { type: "slice"; record: SliceRecord } | { type: "complete"; manifest?: CakeManifest; rebuiltHash?: string; size?: number } | { type: "error"; message: string };

function formatBytes(bytes: number) {
  if (!bytes) return "0 B";
  const units = ["B", "KB", "MB", "GB", "TB"];
  const index = Math.min(Math.floor(Math.log(bytes) / Math.log(1024)), units.length - 1);
  return `${(bytes / 1024 ** index).toFixed(index ? 2 : 0)} ${units[index]}`;
}

function downloadBlob(blob: Blob, name: string) {
  const url = URL.createObjectURL(blob);
  const anchor = document.createElement("a");
  anchor.href = url;
  anchor.download = name;
  anchor.click();
  window.setTimeout(() => URL.revokeObjectURL(url), 1000);
}

function manifestBlob(manifest: CakeManifest) {
  return new Blob([JSON.stringify(manifest, null, 2)], { type: "application/json" });
}

function isManifestFile(file: File) {
  return file.name.endsWith(".cake.json") || file.name.endsWith(".json");
}

export default function BrowserApp() {
  const [file, setFile] = useState<File | null>(null);
  const [sliceSizeIndex, setSliceSizeIndex] = useState(1);
  const [phase, setPhase] = useState<"idle" | "splitting" | "ready" | "error">("idle");
  const [progress, setProgress] = useState(0);
  const [message, setMessage] = useState("Select a local file to begin.");
  const [packageOutput, setPackageOutput] = useState<PackageOutput | null>(null);
  const [sliceRecords, setSliceRecords] = useState<SliceRecord[]>([]);
  const [dropActive, setDropActive] = useState(false);
  const [rebuildFiles, setRebuildFiles] = useState<File[]>([]);
  const [rebuildPhase, setRebuildPhase] = useState<"idle" | "rebuilding" | "complete" | "error">("idle");
  const [rebuildProgress, setRebuildProgress] = useState(0);
  const [rebuildMessage, setRebuildMessage] = useState("Load .slice files and one .cake.json manifest to rebuild.");
  const [rebuiltOutput, setRebuiltOutput] = useState<{ blob: Blob; name: string } | null>(null);
  const workerRef = useRef<Worker | null>(null);
  const operationRef = useRef(0);
  const fileInputRef = useRef<HTMLInputElement | null>(null);
  const rebuildInputRef = useRef<HTMLInputElement | null>(null);
  const sliceSize = sliceSizes[sliceSizeIndex];
  const slices = useMemo(() => file ? Math.max(1, Math.ceil(file.size / sliceBytesFor(sliceSize))) : 0, [file, sliceSize]);

  useEffect(() => () => workerRef.current?.terminate(), []);

  const stopWorker = () => {
    operationRef.current += 1;
    workerRef.current?.terminate();
    workerRef.current = null;
  };

  const selectFile = (next: File | null) => {
    stopWorker();
    setFile(next);
    setPackageOutput(null);
    setSliceRecords([]);
    setProgress(0);
    setPhase("idle");
    setMessage(next ? "Ready to split and verify locally." : "Select a local file to begin.");
  };

  const onFileChange = (event: ChangeEvent<HTMLInputElement>) => {
    selectFile(event.target.files?.[0] ?? null);
    event.currentTarget.value = "";
  };

  const onDrop = (event: DragEvent<HTMLLabelElement>) => {
    event.preventDefault();
    setDropActive(false);
    selectFile(event.dataTransfer.files?.[0] ?? null);
  };

  const handleSplit = () => {
    if (!file || phase === "splitting") return;
    stopWorker();
    const operation = operationRef.current;
    const worker = new Worker(new URL("./split-worker.ts", import.meta.url), { type: "module" });
    workerRef.current = worker;
    setPhase("splitting");
    setProgress(0);
    setSliceRecords([]);
    setPackageOutput(null);
    setMessage("Reading the file locally and calculating SHA-256…");
    worker.onmessage = (event: MessageEvent<WorkerMessage>) => {
      if (operation !== operationRef.current) return;
      const data = event.data;
      if (data.type === "progress") {
        setProgress(data.totalBytes ? Math.min(100, Math.round((data.processedBytes / data.totalBytes) * 100)) : 0);
        setMessage(`Verifying Slice ${data.currentIndex + 1} of ${data.totalSlices}…`);
      } else if (data.type === "slice") {
        setSliceRecords((records) => [...records, data.record]);
      } else if (data.type === "complete" && data.manifest) {
        setPackageOutput({ file, manifest: data.manifest });
        setSliceRecords(data.manifest.slices);
        setProgress(100);
        setPhase("ready");
        setMessage(`${data.manifest.slices.length} ${data.manifest.slices.length === 1 ? "Slice" : "Slices"} verified. Package is ready to download.`);
        worker.terminate();
        workerRef.current = null;
      } else if (data.type === "error") {
        setPhase("error");
        setMessage(data.message);
        worker.terminate();
        workerRef.current = null;
      }
    };
    worker.onerror = () => {
      if (operation !== operationRef.current) return;
      setPhase("error");
      setMessage("The browser could not finish this local operation.");
      worker.terminate();
      workerRef.current = null;
    };
    worker.postMessage({ type: "split", file, sliceSizeBytes: sliceBytesFor(sliceSize) });
  };

  const handleCancel = () => {
    stopWorker();
    setPhase("idle");
    setProgress(0);
    setMessage("Operation cancelled. Your file remains on this device.");
  };

  const downloadSlice = (record: SliceRecord) => {
    if (!packageOutput) return;
    const start = record.index * packageOutput.manifest.sliceSizeBytes;
    downloadBlob(packageOutput.file.slice(start, start + record.size), record.name);
  };

  const downloadAll = () => {
    if (!packageOutput) return;
    packageOutput.manifest.slices.forEach((record, index) => window.setTimeout(() => downloadSlice(record), index * 100));
    window.setTimeout(() => downloadBlob(manifestBlob(packageOutput.manifest), `${packageOutput.file.name}.cake.json`), packageOutput.manifest.slices.length * 100);
  };

  const onRebuildFilesChange = (event: ChangeEvent<HTMLInputElement>) => {
    setRebuildFiles(Array.from(event.target.files ?? []));
    setRebuiltOutput(null);
    setRebuildPhase("idle");
    setRebuildMessage("Files loaded. Choose Rebuild and verify when ready.");
    event.currentTarget.value = "";
  };

  const handleRebuild = async () => {
    if (rebuildPhase === "rebuilding") return;
    try {
      const manifestFile = rebuildFiles.find(isManifestFile);
      const sliceFiles = rebuildFiles.filter((candidate) => candidate.name.endsWith(".slice"));
      if (!manifestFile || sliceFiles.length === 0) throw new Error("Select one .cake.json manifest and at least one .slice file.");
      const manifest = JSON.parse(await manifestFile.text()) as CakeManifest;
      if (manifest.formatVersion !== "1.0" || !Array.isArray(manifest.slices) || !manifest.originalSha256) throw new Error("This is not a valid Cake Package manifest.");
      const filesByName = new Map(sliceFiles.map((candidate) => [candidate.name, candidate]));
      const orderedSlices = manifest.slices.map((record) => {
        const candidate = filesByName.get(record.name);
        if (!candidate) throw new Error(`Missing ${record.name}.`);
        return { record, blob: candidate };
      });
      stopWorker();
      const operation = operationRef.current;
      const worker = new Worker(new URL("./split-worker.ts", import.meta.url), { type: "module" });
      workerRef.current = worker;
      setRebuildPhase("rebuilding");
      setRebuildProgress(0);
      setRebuiltOutput(null);
      setRebuildMessage("Verifying every Slice before rebuilding…");
      worker.onmessage = (event: MessageEvent<WorkerMessage>) => {
        if (operation !== operationRef.current) return;
        const data = event.data;
        if (data.type === "progress") {
          setRebuildProgress(data.totalBytes ? Math.min(100, Math.round((data.processedBytes / data.totalBytes) * 100)) : 0);
          setRebuildMessage(data.phase === "rebuilding" ? "Rebuilding and checking the original SHA-256…" : `Verifying Slice ${data.currentIndex + 1} of ${data.totalSlices}…`);
        } else if (data.type === "complete" && data.rebuiltHash) {
          const rebuiltBlob = new Blob(orderedSlices.map(({ blob }) => blob), { type: "application/octet-stream" });
          setRebuiltOutput({ blob: rebuiltBlob, name: manifest.originalName });
          setRebuildProgress(100);
          setRebuildPhase("complete");
          setRebuildMessage(`EXACT MATCH · ${formatBytes(rebuiltBlob.size)} rebuilt and SHA-256 verified.`);
          worker.terminate();
          workerRef.current = null;
        } else if (data.type === "error") {
          setRebuildPhase("error");
          setRebuildMessage(data.message);
          worker.terminate();
          workerRef.current = null;
        }
      };
      worker.onerror = () => {
        if (operation !== operationRef.current) return;
        setRebuildPhase("error");
        setRebuildMessage("The browser could not finish the rebuild operation.");
        worker.terminate();
        workerRef.current = null;
      };
      worker.postMessage({ type: "rebuild", manifest, slices: orderedSlices });
    } catch (error) {
      setRebuildPhase("error");
      setRebuildMessage(error instanceof Error ? error.message : "The selected package could not be rebuilt.");
    }
  };

  return (
    <main className="browser-app">
      <header className="site-header"><Link className="brand" href="/"><span className="brand-mark">◒</span><span>SplitTheCake</span></Link><div className="header-actions"><span className="preview-pill"><i /> Local-only processing</span><Link href="/" className="text-link">Back to overview <span>↗</span></Link></div></header>
      <div className="app-wrap">
        <div className="app-heading"><span className="section-number mono">CAKESPLITTER / WEB APP</span><h1>Split, verify, rebuild.</h1><p>Choose one file, create verified .slice parts and a Cake manifest, then rebuild the original locally. Nothing leaves this device.</p></div>
        <section className="app-grid">
          <div className="app-card panel">
            <label className={`drop-zone ${dropActive ? "is-dragging" : ""}`} htmlFor="file-input" onDragOver={(event) => { event.preventDefault(); setDropActive(true); }} onDragLeave={() => setDropActive(false)} onDrop={onDrop}><span className="drop-icon">＋</span><strong>{file ? file.name : "Drop a file here or choose one"}</strong><small>{file ? formatBytes(file.size) : "Local-only · no account · no server"}</small><input ref={fileInputRef} id="file-input" type="file" aria-describedby="file-status" onChange={onFileChange} /></label>
            <div className="app-control"><label htmlFor="app-slice-size">Slice size <strong>{sliceSize} GB</strong></label><input id="app-slice-size" type="range" min="0" max={sliceSizes.length - 1} step="1" value={sliceSizeIndex} aria-valuetext={`${sliceSize} GB`} onInput={(event) => setSliceSizeIndex(Number(event.currentTarget.value))} onChange={(event) => setSliceSizeIndex(Number(event.target.value))} /><div className="range-labels mono">{sliceSizes.map((size) => <span key={size}>{size} GB</span>)}</div></div>
            <div className="app-actions"><button className="button button--primary app-action" type="button" disabled={!file || phase === "splitting"} onClick={handleSplit}>{phase === "splitting" ? "Processing locally…" : "Split and verify"}<span>→</span></button>{phase === "splitting" && <button className="button button--secondary app-action" type="button" onClick={handleCancel}>Cancel</button>}{file && phase !== "splitting" && <button className="text-button" type="button" onClick={() => selectFile(null)}>Clear file</button>}</div>
            {phase === "splitting" && <div className="app-progress" role="progressbar" aria-label="Local split progress" aria-valuemin={0} aria-valuemax={100} aria-valuenow={progress}><span style={{ width: `${progress}%` }} /></div>}
            <p className="app-message" id="file-status" aria-live="polite"><span className={`pulse-ring ${phase === "error" ? "pulse-ring--error" : ""}`} /> {message}</p>
          </div>
          <div className="app-card panel app-output"><div className="package-card-head"><span className="eyebrow">CAKE PACKAGE PREVIEW</span><span className={phase === "ready" ? "verified-text" : "mono"}>{phase === "ready" ? "VERIFIED" : file ? "READY TO PROCESS" : "WAITING"}</span></div>{file ? <><div className="app-metrics"><div><span>ORIGINAL</span><strong>{formatBytes(file.size)}</strong></div><div><span>ESTIMATED SLICES</span><strong>{packageOutput?.manifest.slices.length ?? slices}</strong></div><div><span>SLICE SIZE</span><strong>≤ {sliceSize} GB</strong></div><div><span>PROCESSING</span><strong>LOCAL</strong></div></div><div className="app-files">{sliceRecords.length > 0 ? sliceRecords.map((record) => <div className="package-row is-linked" key={record.name}><span className="file-glyph">▦</span><span className="file-name">{record.name}</span><span className="file-size mono">{formatBytes(record.size)}</span><span className="file-status">{phase === "ready" ? "SHA-256 verified" : "hashing…"}</span>{phase === "ready" && <button className="file-download" type="button" onClick={() => downloadSlice(record)}>Download</button>}</div>) : <div className="app-more mono">{slices} Slice{slices === 1 ? "" : "s"} planned · click Split and verify to create files</div>}{packageOutput && <div className="package-row is-manifest"><span className="file-glyph">{}</span><span className="file-name">{file.name}.cake.json</span><span className="file-status">Manifest · verified</span><button className="file-download" type="button" onClick={() => downloadBlob(manifestBlob(packageOutput.manifest), `${file.name}.cake.json`)}>Download</button></div>}</div>{packageOutput && <button className="button button--secondary app-action" type="button" onClick={downloadAll}>Download package files <span>↓</span></button>}</> : <div className="empty-output"><div className="empty-cake">◒</div><p>Your verified Slice parts and manifest will appear here.</p></div>}</div>
        </section>
        <section className="rebuild-card panel"><div className="section-number mono">REBUILD A CAKE PACKAGE</div><h2>Load Slices. Match bytes.</h2><p>Choose the `.slice` files and their `.cake.json` manifest. The browser verifies each checksum before creating the original file.</p><label className="drop-zone rebuild-drop" htmlFor="rebuild-input"><span className="drop-icon">↺</span><strong>{rebuildFiles.length ? `${rebuildFiles.length} package files selected` : "Choose Slices and a manifest"}</strong><small>Multiple files · .slice + .cake.json</small><input ref={rebuildInputRef} id="rebuild-input" type="file" multiple accept=".slice,.json,.cake.json" onChange={onRebuildFilesChange} /></label><div className="rebuild-file-list">{rebuildFiles.map((candidate) => <span key={`${candidate.name}-${candidate.size}`} className="mono">{candidate.name}</span>)}</div><div className="rebuild-actions"><button className="button button--primary" type="button" disabled={!rebuildFiles.length || rebuildPhase === "rebuilding"} onClick={handleRebuild}>{rebuildPhase === "rebuilding" ? "Rebuilding…" : "Rebuild and verify"}<span>→</span></button>{rebuiltOutput && <button className="button button--secondary" type="button" onClick={() => downloadBlob(rebuiltOutput.blob, rebuiltOutput.name)}>Download rebuilt file <span>↓</span></button>}</div>{rebuildPhase === "rebuilding" && <div className="app-progress" role="progressbar" aria-label="Rebuild progress" aria-valuemin={0} aria-valuemax={100} aria-valuenow={rebuildProgress}><span style={{ width: `${rebuildProgress}%` }} /></div>}<p className={`app-message ${rebuildPhase === "error" ? "app-message--error" : ""}`} aria-live="polite"><span className="pulse-ring" /> {rebuildMessage}</p></section>
        <div className="app-note"><span>ⓘ</span><p>All processing stays in this browser tab. For very large files, keep this tab open while the Worker hashes each chunk. You can cancel without changing the source file.</p></div>
      </div>
    </main>
  );
}
