"use client";

import { useEffect, useMemo, useRef, useState } from "react";
import type { CSSProperties, ReactNode } from "react";
import Link from "next/link";

const slices = Array.from({ length: 12 }, (_, index) => index + 1);
const sliceSizes = [1, 2, 3, 4, 6] as const;
const githubUrl = "https://github.com/Ianlovesneko/CakeSplitter";

const workflow = [
  ["01", "Select", "Choose one large local file."],
  ["02", "Split", "Divide it by size or slice count."],
  ["03", "Verify", "Every slice receives SHA-256."],
  ["04", "Move", "Transfer the parts separately."],
  ["05", "Rebuild", "Match the original byte for byte."],
];

const comparisons = [
  ["Local processing", "Yes", "Yes"],
  ["Split / Merge / Inspect", "Yes", "Yes"],
  ["Installation", "No", "Windows installer"],
  ["Large-file streaming", "Limited", "Full native streaming"],
  ["Pause / Resume", "Limited", "Yes"],
  ["Restart recovery", "Limited", "Yes"],
  ["Task queue", "Limited", "Yes"],
  ["Operation receipts", "—", "Yes"],
  ["Direct Folder Mode", "Disabled", "Native support"],
];

function CapabilityValue({ value }: { value: string }) {
  if (value === "Yes") return <span className="capability-value capability-value--yes" aria-label="Yes">✓</span>;
  if (value === "No") return <span className="capability-value capability-value--no" aria-label="No">×</span>;
  const statusClass = value === "Disabled" || value === "None" || value === "—" ? "status-unavailable" : value === "Limited" ? "status-limited" : "status-supported";
  return <span className={`capability-value ${statusClass}`}>{value}</span>;
}

function LimitMeter() {
  return (
    <div
      className="limit-meter"
      role="img"
      aria-label="A 12 GB file exceeds a 10 GB limit, so it becomes six equal 2 GB slices: five fit on the first line and one remains below."
    >
      <div className="limit-meter__track" aria-hidden="true">
        <span className="limit-meter__rail" />
        <span className="limit-meter__file" />
        <span className="limit-meter__over" />
        <span className="limit-meter__final" />
        <i className="limit-meter__cut" />
      </div>
      <i className="limit-meter__limit-line" aria-hidden="true" />
      <div className="limit-meter__slice-stack" aria-hidden="true">
        <div className="limit-meter__slice-row">
          {Array.from({ length: 5 }, (_, index) => <span className="limit-meter__slice limit-meter__slice--large" key={index} />)}
        </div>
        <span className="limit-meter__slice limit-meter__slice--small" />
      </div>
      <div className="limit-meter__labels" aria-hidden="true">
        <b className="limit-meter__label limit-meter__label--file">12 GB FILE</b>
        <b className="limit-meter__label limit-meter__label--over">2 GB OVER</b>
        <b className="limit-meter__label limit-meter__label--large">2 GB × 5 SLICES</b>
        <b className="limit-meter__label limit-meter__label--small">2 GB SLICE</b>
        <b className="limit-meter__label limit-meter__label--confirm">SPLIT TO FIT</b>
        <b className="limit-meter__label limit-meter__label--limit">10 GB LIMIT</b>
      </div>
    </div>
  );
}

const queue = [
  { name: "dataset.tar", state: "Running", status: "62%", kind: "running", detail: "Streaming slice 7 of 12 · local transfer" },
  { name: "footage.mov", state: "Queued", status: "High", kind: "queued", detail: "Waiting for the running transfer slot" },
  { name: "backup.img", state: "Paused", status: "—", kind: "paused", detail: "Paused by the operator · safe to resume" },
  { name: "archive.zip", state: "Recovery required", status: "—", kind: "recovery", detail: "One slice needs a retry before rebuild" },
] as const;

const evidence = [
  ["1 GiB", "streamed split + merge", "16 × 64 MiB slices · bounded memory"],
  ["Byte-for-byte", "reconstruction", "Original and rebuilt SHA-256 match"],
  ["Restart-aware", "recovery", "Safe to resume after interruption"],
  ["Bounded", "task admission", "Preflight catches space conflicts"],
  ["Local-only", "network verification", "No upload path in the web workflow"],
  ["Packaged", "Windows testing", "Preview installer is unsigned"],
];

const faqs = [
  ["Does CakeSplitter compress files?", "No. Splitting changes how the file is divided, not its total size."],
  ["Are files uploaded?", "No. Web and Desktop processing are designed to run locally. We'll not collect your information or use your data for analyzing. Privacy first."],
  ["Can each Slice be opened separately?", "Usually not. Slices are binary parts used to reconstruct the original file."],
  ["What happens if one Slice is missing?", "CakeSplitter reports the missing Slice and refuses to present the package as complete."],
  ["Is the Windows installer signed?", "The current preview build is unsigned and may trigger Windows SmartScreen."],
  ["Is it open source?", "CakeSplitter v0.7 source is now available on GitHub for review. A formal open-source license has not been published yet."],
];

function useReveal() {
  useEffect(() => {
    const targets = Array.from(document.querySelectorAll<HTMLElement>("[data-reveal]"));
    if ("IntersectionObserver" in window) {
      const observer = new IntersectionObserver(
        (entries) => entries.forEach((entry) => entry.isIntersecting && entry.target.classList.add("is-visible")),
        { threshold: 0.12 },
      );
      targets.forEach((target) => observer.observe(target));
      return () => observer.disconnect();
    }
    targets.forEach((target) => target.classList.add("is-visible"));
  }, []);
}

function Cake({ count = 12, compact = false, sliceSize = "4.00" }: { count?: number; compact?: boolean; sliceSize?: string }) {
  const [active, setActive] = useState<number | null>(null);
  const previousVisibleSlicesRef = useRef<number>(count);
  const transitionTimerRef = useRef<number | null>(null);
  const [transitionFrom, setTransitionFrom] = useState<number | null>(null);
  // Keep the visual and its hit targets in lockstep with the requested count.
  // The hero defaults to twelve while the calculator can pass any value from
  // two to twelve (for example, a 6 GB setting renders exactly two slices).
  const visibleSlices = Number.isFinite(count) ? Math.max(1, Math.min(12, Math.floor(count))) : 12;
  const radius = 43.5;
  const sliceColor = "#b86145";

  useEffect(() => {
    const previous = previousVisibleSlicesRef.current;
    if (previous === visibleSlices) return undefined;

    previousVisibleSlicesRef.current = visibleSlices;
    setActive(null);
    setTransitionFrom(previous);
    if (transitionTimerRef.current !== null) window.clearTimeout(transitionTimerRef.current);
    transitionTimerRef.current = window.setTimeout(() => {
      setTransitionFrom(null);
      transitionTimerRef.current = null;
    }, 420);

    return () => {
      if (transitionTimerRef.current !== null) {
        window.clearTimeout(transitionTimerRef.current);
        transitionTimerRef.current = null;
      }
    };
  }, [visibleSlices]);

  useEffect(() => () => {
    if (transitionTimerRef.current !== null) window.clearTimeout(transitionTimerRef.current);
  }, []);

  const renderVisual = (sliceCount: number, interactive: boolean) => {
    const angleStep = 360 / sliceCount;
    const sectorPath = (index: number) => {
      const start = -90 - angleStep / 2 + angleStep * index;
      const end = start + angleStep;
      const point = (angle: number) => {
        const radians = (angle * Math.PI) / 180;
        return [50 + radius * Math.cos(radians), 50 + radius * Math.sin(radians)];
      };
      const [startX, startY] = point(start);
      const [endX, endY] = point(end);
      return `M 50 50 L ${startX.toFixed(4)} ${startY.toFixed(4)} A ${radius} ${radius} 0 0 1 ${endX.toFixed(4)} ${endY.toFixed(4)} Z`;
    };

    return (
      <svg className={`cake-visual ${interactive ? (active ? "has-active" : "is-idle") : "cake-visual--previous"}`} viewBox="0 0 100 100" aria-label={interactive ? `${sliceCount} slices` : undefined} aria-hidden={interactive ? undefined : true} role={interactive ? "img" : undefined}>
        <circle className="cake-visual-backdrop" cx="50" cy="50" r="46.5" />
        {Array.from({ length: sliceCount }, (_, index) => {
          // Move only the active sector a few SVG units along its own radial
          // axis. The shell and all non-active sectors remain perfectly still.
          const centerAngle = (-90 + angleStep * index) * (Math.PI / 180);
          const isActive = interactive && active === index + 1;
          const shift = isActive ? 1.4 : 0;
          return (
            <path
              key={index + 1}
              className={`cake-sector ${isActive ? "is-active" : ""}`}
              data-cake-sector={interactive ? index + 1 : undefined}
              d={sectorPath(index)}
              fill={sliceColor}
              style={{
                "--sector-shift-x": `${Math.cos(centerAngle) * shift}px`,
                "--sector-shift-y": `${Math.sin(centerAngle) * shift}px`,
              } as CSSProperties}
              onPointerEnter={interactive ? () => setActive(index + 1) : undefined}
              onFocus={interactive ? () => setActive(index + 1) : undefined}
              onBlur={interactive ? () => setActive(null) : undefined}
              onClick={interactive ? () => setActive(index + 1) : undefined}
            />
          );
        })}
        <circle className="cake-edge" cx="50" cy="50" r={radius} />
        <circle className="cake-center-cap" cx="50" cy="50" r={3.25} />
      </svg>
    );
  };

  return (
    <div
      className={`cake-shell ${compact ? "cake-shell--compact" : ""}`}
      onPointerLeave={() => setActive(null)}
      data-slice-count={visibleSlices}
      data-angle-step={360 / visibleSlices}
    >
      <div className={`cake-visual-stack ${transitionFrom !== null ? "is-changing" : ""}`}>
        {transitionFrom !== null && <div className="cake-visual-layer cake-visual-layer--previous">{renderVisual(transitionFrom, false)}</div>}
        <div className={`cake-visual-layer ${transitionFrom !== null ? "cake-visual-layer--current" : ""}`}>{renderVisual(visibleSlices, true)}</div>
      </div>
      <div className="cake-controls">
        {Array.from({ length: visibleSlices }, (_, index) => {
          const number = index + 1;
          return (
            <button
              key={number}
              type="button"
              className="cake-control"
              onFocus={() => setActive(number)}
              onBlur={() => setActive(null)}
              onClick={() => setActive(number)}
              aria-label={`Slice ${number}, ${sliceSize} GB, SHA-256 verified`}
            />
          );
        })}
      </div>
      {!compact && (
        <div className={`slice-tooltip ${active ? "is-visible" : ""}`} aria-live="polite">
          <span className="eyebrow">SLICE {String(active ?? 1).padStart(2, "0")}</span>
          <strong>{sliceSize} GB</strong>
          <span className="verified-text">✓ SHA-256 verified</span>
        </div>
      )}
    </div>
  );
}

function VerificationDemo() {
  const sectionRef = useRef<HTMLDivElement>(null);
  const timerRef = useRef<number | null>(null);
  const [verified, setVerified] = useState(0);
  const [hashStage, setHashStage] = useState(0);
  const [complete, setComplete] = useState(false);

  useEffect(() => {
    const node = sectionRef.current;
    if (!node) return;
    let isInView = false;
    const clearTimer = () => {
      if (timerRef.current !== null) {
        window.clearTimeout(timerRef.current);
        timerRef.current = null;
      }
    };
    const reset = () => {
      clearTimer();
      setVerified(0);
      setHashStage(0);
      setComplete(false);
    };
    const play = () => {
      clearTimer();
      setVerified(0);
      setHashStage(0);
      setComplete(false);
      if (window.matchMedia("(prefers-reduced-motion: reduce)").matches) {
        setVerified(12);
        setHashStage(2);
        setComplete(true);
        return;
      }
      let slice = 0;
      const advanceSlice = () => {
        slice += 1;
        setVerified(slice);
        if (slice < 12) {
          timerRef.current = window.setTimeout(advanceSlice, 72);
          return;
        }
        timerRef.current = window.setTimeout(() => {
          setHashStage(1);
          timerRef.current = window.setTimeout(() => {
            setHashStage(2);
            timerRef.current = window.setTimeout(() => setComplete(true), 280);
          }, 420);
        }, 360);
      };
      timerRef.current = window.setTimeout(advanceSlice, 120);
    };
    if (!("IntersectionObserver" in window)) {
      play();
      return clearTimer;
    }
    const observer = new IntersectionObserver(([entry]) => {
      if (entry.isIntersecting && !isInView) {
        isInView = true;
        play();
      } else if (!entry.isIntersecting && isInView) {
        isInView = false;
        reset();
      }
    }, { threshold: 0.35 });
    observer.observe(node);
    return () => {
      observer.disconnect();
      clearTimer();
    };
  }, []);

  const rebuiltHash = hashStage === 0 ? "a83f91c2" : hashStage === 1 ? "49b7e04a" : "49bc20df";

  return (
    <div ref={sectionRef} className="verification-stage">
      <div className="verification-topline">
        <span className="eyebrow">PACKAGE CHECK / INTERACTIVE DEMO</span>
        <span className="mono">{verified} / 12 SLICES FOUND</span>
      </div>
      <p className="demo-disclaimer mono">SAMPLE DATA · REAL FILES ARE VERIFIED IN THE WEB APP</p>
      <div className="verification-slices" aria-label={`${verified} of 12 slices verified`}>
        {slices.map((slice) => <span key={slice} className={`verify-slice ${slice <= verified ? "is-verified" : ""}`}>{slice <= verified ? "✓" : "·"}</span>)}
      </div>
      <div className={`hash-match ${complete ? "is-complete" : ""}`}>
        <div><span>ORIGINAL SHA-256</span><code>49bc20df<span>…</span></code></div>
        <div className={`hash-bridge ${hashStage >= 2 ? "is-lit" : ""}`} aria-hidden="true">↔</div>
        <div><span>REBUILT SHA-256</span><code className={hashStage >= 2 ? "is-final" : "is-computing"} aria-live="polite">{rebuiltHash}<span>…</span></code></div>
      </div>
      <div className={`exact-match ${complete ? "is-complete" : ""}`}><span>✓</span> EXACT MATCH</div>
    </div>
  );
}

function AppPreviewLink({ children = "Open the Web App" }: { children?: ReactNode }) {
  return <a className="button button--primary" href="/app">{children}<span aria-hidden="true">↗</span></a>;
}

export default function Home() {
  useReveal();
  const [sliceSize, setSliceSize] = useState(4);
  const [focusedPackage, setFocusedPackage] = useState<number | null>(null);
  const [selectedPackage, setSelectedPackage] = useState<number | null>(null);
  const [openFaq, setOpenFaq] = useState<number | null>(0);
  const [shaHelpOpen, setShaHelpOpen] = useState(false);
  const [selectedTask, setSelectedTask] = useState<number | null>(null);
  const [queueProgress, setQueueProgress] = useState(62);
  const [queuePaused, setQueuePaused] = useState(false);
  const [queueVisible, setQueueVisible] = useState(false);
  const [reducedMotion, setReducedMotion] = useState(false);
  const queueSectionRef = useRef<HTMLElement | null>(null);
  const queueMotionRef = useRef({ holdUntil: 0, nextPauseAt: 0 });

  useEffect(() => {
    const media = window.matchMedia("(prefers-reduced-motion: reduce)");
    const updateMotion = () => setReducedMotion(media.matches);
    updateMotion();
    media.addEventListener("change", updateMotion);
    return () => media.removeEventListener("change", updateMotion);
  }, []);

  useEffect(() => {
    const section = queueSectionRef.current;
    if (!section || !("IntersectionObserver" in window)) {
      setQueueVisible(true);
      return undefined;
    }
    const observer = new IntersectionObserver(([entry]) => setQueueVisible(entry.isIntersecting), { rootMargin: "180px 0px" });
    observer.observe(section);
    return () => observer.disconnect();
  }, []);

  useEffect(() => {
    // Keep the queue progress live and fluid rather than a fixed decorative bar.
    // Small speed changes and short, irregular pauses make the transfer feel
    // like a real local operation while remaining slow enough to read.
    if (queuePaused || !queueVisible || reducedMotion) return undefined;
    const timer = window.setInterval(() => {

      const motion = queueMotionRef.current;
      const now = performance.now();
      if (motion.nextPauseAt === 0) {
        motion.nextPauseAt = now + 1200 + Math.random() * 1400;
      }
      if (now < motion.holdUntil) return;
      if (now >= motion.nextPauseAt) {
        motion.holdUntil = now + 220 + Math.random() * 520;
        motion.nextPauseAt = motion.holdUntil + 1000 + Math.random() * 1800;
        return;
      }

      setQueueProgress((value) => {
        const next = value + 0.12 + Math.random() * 0.24;
        if (next >= 100) {
          motion.holdUntil = 0;
          motion.nextPauseAt = now + 1200 + Math.random() * 1400;
          return 62;
        }
        return next;
      });
    }, 140);
    return () => window.clearInterval(timer);
  }, [queuePaused, queueVisible, reducedMotion]);

  const selectedQueueTask = selectedTask === null ? null : queue[selectedTask];
  const queueState = queuePaused ? "Paused" : queue[0].state;
  const queueStatus = `${Math.round(queueProgress)}%`;
  const handleQueueAction = () => {
    if (selectedTask === 0) {
      setQueuePaused((paused) => !paused);
      return;
    }
    if (selectedTask === 1) {
      // Starting a queued operation focuses the active transfer so its live
      // progress and pause/resume controls are immediately visible.
      setQueuePaused(false);
      setSelectedTask(0);
      return;
    }
    if (selectedTask === 2) {
      setQueuePaused(false);
      setSelectedTask(0);
      return;
    }
    if (selectedTask === 3) {
      setQueueProgress(0);
      setQueuePaused(false);
    }
  };
  const fileSize = 12;
  const estimatedSlices = Math.max(1, Math.ceil(fileSize / sliceSize));
  // The slider is a maximum Slice size. Every full Slice uses that size and
  // the final Slice contains only the remaining bytes (for 5 GB: 5 + 5 + 2).
  const finalSize = (fileSize - sliceSize * (estimatedSlices - 1)).toFixed(2);
  const requiredSpace = (12.01).toFixed(2);
  const previewCount = Math.min(12, Math.max(1, estimatedSlices));
  const packageCopy = useMemo(() => `${estimatedSlices} slices · 1 manifest`, [estimatedSlices]);
  const packageRows = useMemo(() => {
    const rows: [string, string, string][] = Array.from({ length: estimatedSlices }, (_, index) => {
      const currentSize = Math.min(sliceSize, fileSize - sliceSize * index);
      return [`archive.zip.${String(index + 1).padStart(3, "0")}.slice`, `${currentSize.toFixed(2)} GB`, "SHA-256 verified"];
    });
    rows.push(["archive.zip.cake.json", "2.4 KB", "Manifest · complete"]);
    return rows;
  }, [estimatedSlices, sliceSize]);
  const packageDetails = useMemo(() => {
    const details: [string, string, string][] = Array.from({ length: estimatedSlices }, (_, index) => {
      const start = (sliceSize * index).toFixed(2);
      const end = Math.min(fileSize, sliceSize * (index + 1)).toFixed(2);
      return [`Slice ${String(index + 1).padStart(2, "0")}`, `Binary data part · byte range ${start}–${end} GB`, `49bc20df…${String.fromCharCode(97 + (index % 26))}${String(index + 1).padStart(2, "0")}f`];
    });
    details.push(["Manifest", "Order, size, hash, and original file metadata", "formatVersion 1.0"]);
    return details;
  }, [estimatedSlices, sliceSize]);
  const selectedPackageRow = selectedPackage === null ? null : packageRows[selectedPackage];
  const selectedPackageDetail = selectedPackage === null ? null : packageDetails[selectedPackage];
  const sliceSizeIndex = sliceSizes.indexOf(sliceSize as (typeof sliceSizes)[number]);
  const chooseSliceSize = (index: number) => setSliceSize(sliceSizes[Math.max(0, Math.min(sliceSizes.length - 1, index))]);

  return (
    <main>
      <header className="site-header">
        <a className="brand" href="#top" aria-label="SplitTheCake home"><span className="brand-mark">◒</span><span>SplitTheCake</span></a>
        <nav className="desktop-nav" aria-label="Primary navigation">
          <a href="#about">About</a><a href="#workflow">How it works</a><a href="#trust">Trust</a><a href="#desktop">Desktop</a><a href="#faq">FAQ</a>
        </nav>
        <div className="header-actions"><span className="preview-pill"><i /> Development <span className="preview-word">Preview</span></span><AppPreviewLink /></div>
      </header>

      <section id="top" className="hero section-wrap">
        <div className="hero-copy" data-reveal="up">
          <div className="kicker"><span className="kicker-line" /> LOCAL-FIRST LARGE FILE WORKFLOWS</div>
          <h1><span>Split large files.</span><em>Rebuild them exactly.</em></h1>
          <p className="hero-lede">CakeSplitter turns one large file into verified <strong>.slice</strong> parts, then reconstructs the original byte for byte. Processed locally. No upload required.</p>
          <div className="hero-actions"><AppPreviewLink /><a className="text-link" href="#workflow">See how it works <span>↓</span></a></div>
          <div className="release-note"><span className="status-dot" /> Source preview available on <a href={githubUrl} target="_blank" rel="noopener noreferrer">GitHub</a><span className="mono">·</span><span className="mono">CURRENT v0.7</span></div>
        </div>
        <div className="hero-art" data-reveal="fade">
          <div className="art-label art-label--top mono">12 SLICES / 1 FILE</div>
          <Cake sliceSize="1.00" />
          <div className="art-label art-label--bottom"><span className="pulse-ring" /> SHA-256 demo <span className="mono">· SAMPLE</span></div>
        </div>
        <div className="scroll-cue mono"><span>SCROLL TO EXPLORE</span><i /></div>
      </section>

      <section className="marquee" aria-label="Product tagline"><div><span className="marquee-item marquee-item--accent">Local First • Verified Slices • Exact Rebuild • No Upload • Smart Queue • Recovery Ready • Private by Design • Byte Perfect •</span><span className="marquee-item marquee-item--muted">Local First • Verified Slices • Exact Rebuild • No Upload • Smart Queue • Recovery Ready • Private by Design • Byte Perfect •</span><span className="marquee-item marquee-item--accent">Local First • Verified Slices • Exact Rebuild • No Upload • Smart Queue • Recovery Ready • Private by Design • Byte Perfect •</span></div></section>

      <section className="about-section section-wrap" id="about">
        <div className="about-section__copy" data-reveal="up">
          <span className="eyebrow">ABOUT CAKESPLITTER</span>
          <h2>File tools should be understandable, inspectable, and dependable.</h2>
          <p><strong>CakeSplitter is a local-first tool for splitting large files into verified Slices and rebuilding them exactly.</strong></p>
          <p>Large files are often difficult to move, upload, archive, or share because platforms impose size limits and ordinary file-splitting tools provide little assurance that every part is complete.</p>
          <Link className="text-link" href="/about">Read the full story <span>→</span></Link>
        </div>
        <blockquote className="about-section__quote panel" data-reveal="fade">
          <span className="quote-mark" aria-hidden="true">“</span>
          <p>CakeSplitter creates a portable Cake Package with independently verifiable Slices and a Manifest that preserves their order and integrity. Files remain on your device, and a rebuild is only considered complete after verification.</p>
          <cite>We believe file tools should be understandable, inspectable, and dependable—not mysterious.</cite>
        </blockquote>
      </section>

      <section className="section-wrap problem-section" id="problem">
        <div className="section-intro" data-reveal="up"><h2>One file shouldn’t stop the transfer.</h2><p>When a platform, connection, or checksum fails, the whole workflow shouldn’t have to start over.</p></div>
        <div className="problem-grid" data-reveal="up">
          <article className="problem-card"><div className="problem-icon">↗</div><span className="eyebrow">UPLOAD LIMITS</span><h3>Too large for the platform.</h3><p>If a 12GB archive meets 10GB upload limit. Split it into parts that fit.</p><LimitMeter /></article>
          <article className="problem-card"><div className="problem-icon">⌁</div><span className="eyebrow">UNSTABLE TRANSFERS</span><h3>One failed transfer means starting over.</h3><p>Move individual parts. Retry only what did not make it across.</p><div className="transfer-meter" role="img" aria-label="Transfer progress moves smoothly from the start to 62 percent, pauses briefly at Error Occur, then retries only the missing part."><span /><i /><b aria-hidden="true">Error Occur</b></div></article>
          <article className="problem-card"><div className="problem-icon">?</div><span className="eyebrow">INTEGRITY UNCERTAINTY</span><h3>Done does not always mean intact.</h3><p>Each Slice gets a checksum, so a finished transfer can be checked.</p><div className={`checksum-meter ${shaHelpOpen ? "is-open" : ""}`}><button className="checksum-trigger" type="button" aria-expanded={shaHelpOpen} aria-controls="sha-help" onClick={() => setShaHelpOpen((open) => !open)}><span>Using SHA-256 to verify</span><i aria-hidden="true">?</i></button>{shaHelpOpen && <p id="sha-help">SHA-256 creates a fixed-length fingerprint for each Slice. Matching fingerprints confirm the bytes arrived unchanged.</p>}</div></article>
        </div>
      </section>

      <section className="section-wrap workflow-section" id="workflow">
        <div className="section-intro section-intro--center" data-reveal="up"><h2>One Cake. Many Slices. Exact reconstruction.</h2><p>Every step has a job. Every handoff stays inspectable.</p></div>
        <div className="workflow-steps" data-reveal="up">{workflow.map(([number, title, text], index) => <div className="workflow-step" key={title} style={{ "--index": index } as CSSProperties}><div className="workflow-node"><span>{number}</span><i /></div><h3>{title}</h3><p>{text}</p>{index < workflow.length - 1 && <div className="workflow-line" />}</div>)}</div>
        <div className="calculator panel" data-reveal="up">
          <div className="calculator-copy"><span className="eyebrow">TRY THE MODEL</span><h3>Set a Slice size.</h3><p>See how a 12 GB file becomes a portable Cake Package.</p><label htmlFor="slice-size">Slice size <strong>{sliceSize} GB</strong></label><input id="slice-size" type="range" min="0" max={sliceSizes.length - 1} step="1" value={sliceSizeIndex} aria-valuetext={`${sliceSize} GB`} onInput={(event) => chooseSliceSize(Number(event.currentTarget.value))} onChange={(event) => chooseSliceSize(Number(event.target.value))} /><div className="range-labels mono">{sliceSizes.map((size) => <span key={size}>{size} GB</span>)}</div></div>
          <div className="calculator-visual"><div className="mini-cake"><Cake count={previewCount} compact sliceSize={sliceSize.toFixed(2)} /></div><div className="calculator-metrics"><div><span>ESTIMATED SLICES</span><strong>{estimatedSlices}</strong></div><div><span>FINAL SLICE SIZE</span><strong>{finalSize} GB</strong></div><div><span>REQUIRED SPACE</span><strong>{requiredSpace} GB</strong></div><div><span>MANIFEST</span><strong>{packageCopy}</strong></div></div></div>
        </div>
      </section>

      <section className="section-wrap package-section" id="package">
        <div className="package-layout"><div className="section-intro" data-reveal="up"><h2>Every Slice knows where it belongs.</h2><p>A Cake Package pairs binary parts with one small manifest that preserves order, size, hashes, and the original file name.</p><div className="package-legend"><span><b className="legend-dot legend-dot--slice" /> .slice <small>actual data</small></span><span><b className="legend-dot legend-dot--manifest" /> .cake.json <small>manifest</small></span></div></div><div className="package-card panel" data-reveal="fade"><div className="package-card-head"><span className="eyebrow">CAKE PACKAGE / ARCHIVE.ZIP</span><span className="verified-text">✓ COMPLETE</span></div><div className="package-files">{packageRows.map(([name, size, status], index) => { const isManifest = index === packageRows.length - 1; return <button type="button" key={name} className={`package-row ${focusedPackage === index || selectedPackage === index ? "is-linked" : ""} ${isManifest ? "is-manifest" : ""}`} onMouseEnter={() => setFocusedPackage(index)} onFocus={() => setFocusedPackage(index)} onMouseLeave={() => setFocusedPackage(null)} onBlur={() => setFocusedPackage(null)} onClick={() => setSelectedPackage(selectedPackage === index ? null : index)} aria-expanded={selectedPackage === index}><span className="file-glyph">{isManifest ? "{}" : "▦"}</span><span className="file-name">{name}</span><span className="file-size mono">{size}</span><span className="file-status">{status}</span><span className="file-open" aria-hidden="true">{selectedPackage === index ? "−" : "+"}</span></button>; })}</div>{selectedPackageRow && selectedPackageDetail && <div className="package-inspector" aria-live="polite"><div><span className="eyebrow">OPEN FILE / {selectedPackageDetail[0]}</span><strong>{selectedPackageRow[0]}</strong></div><div><span className="inspector-label">DETAIL</span><p>{selectedPackageDetail[1]}</p></div><div><span className="inspector-label">CHECK</span><code>{selectedPackageDetail[2]}</code></div></div>}<div className="manifest-preview"><div className="manifest-head"><span className="eyebrow">MANIFEST PREVIEW</span><span className="mono">CAKE FORMAT 1.0</span></div><pre><code><span>{"{"}</span>{"\n  "}<b>&quot;formatVersion&quot;</b>: <em>&quot;1.0&quot;</em>,{"\n  "}<b>&quot;originalName&quot;</b>: <em>&quot;archive.zip&quot;</em>,{"\n  "}<b>&quot;sliceCount&quot;</b>: <strong>{estimatedSlices}</strong>,{"\n  "}<b>&quot;sha256&quot;</b>: <em>&quot;verified&quot;</em>{"\n"}{"}"}</code></pre></div></div></div>
      </section>

      <section className="section-wrap trust-section" id="trust">
        <div className="section-intro section-intro--center" data-reveal="up"><h2>Built to show clearly.</h2><p>Concrete controls make the local-first promise observable.</p></div>
        <div className="trust-grid" data-reveal="up"><article className="trust-card"><div className="trust-icon" aria-hidden="true"><svg className="trust-lock-icon" viewBox="0 0 24 24" focusable="false"><rect x="5" y="10" width="14" height="10" rx="2" /><path d="M8 10V7a4 4 0 0 1 8 0v3M12 14v3" /></svg></div><span className="eyebrow">LOCAL-ONLY</span><h3>Your files stay on your device.</h3><p>No upload path. No handoff to a remote service.</p><div className="trust-art trust-art--local" aria-label="Files stay on this device"><span className="local-device"><b>THIS DEVICE</b><strong>▦</strong></span><i className="local-route"><b>×</b></i><span className="local-server"><b>NO UPLOAD</b><strong>↗</strong></span></div></article><article className="trust-card"><div className="trust-icon trust-icon--verified">✓</div><span className="eyebrow">VERIFIED SLICES</span><h3>Every Slice can be checked independently.</h3><p>A checksum travels with each part.</p><div className="trust-art trust-art--checks">{[1, 2, 3].map((item) => <i key={item}>✓</i>)}</div></article><article className="trust-card"><div className="trust-icon trust-icon--verified">↺</div><span className="eyebrow">EXACT REBUILD</span><h3>The rebuilt file must match.</h3><p>Original and rebuilt SHA-256 are compared.</p><div className="trust-art trust-art--hash"><span>49bc…</span><i>＝</i><span>49bc…</span></div></article><article className="trust-card"><div className="trust-icon trust-icon--warning">×</div><span className="eyebrow">FAIL CLOSED</span><h3>Incomplete never looks complete.</h3><p>Missing, replaced, or corrupt data stops the flow.</p><div className="trust-art trust-art--closed"><span>12 / 12</span><i>!</i></div></article></div>
      </section>

      <section className="section-wrap compare-section" id="desktop"><div className="section-intro" data-reveal="up"><h2>Starts from the browser. Go further on desktop.</h2><p>Start local and lightweight. Move to the Windows desktop when the work needs queues, recovery, and native streaming.</p></div><div className="compare-table panel" data-reveal="up"><div className="comparison-row comparison-head"><span>CAPABILITY</span><span>WEB APP</span><span>WINDOWS DESKTOP</span></div>{comparisons.map(([feature, web, desktop]) => <div className="comparison-row" key={feature}><span>{feature}</span><CapabilityValue value={web} /><CapabilityValue value={desktop} /></div>)}</div><div className="comparison-legend" aria-label="Capability legend"><span><i className="capability-mark capability-mark--yes">✓</i> Available</span><span><i className="capability-mark capability-mark--no">×</i> Not available</span><span>Other labels show the specific limitation or capability.</span></div><p className="callout"><span>!</span> Web Direct Folder Mode is currently disabled.</p></section>

      <section ref={queueSectionRef} className="section-wrap queue-section"><div className="queue-layout"><div className="section-intro" data-reveal="up"><h2>Designed for work that takes time.</h2><p>Desktop CakeSplitter keeps long-running operations visible, resumable, and accountable.</p><div className="queue-features"><span>Queue</span><span>Priority</span><span>Pause / Resume</span><span>Receipts</span><span>Diagnostics</span></div></div><div className="queue-card panel" data-reveal="fade"><div className="queue-head"><span className="eyebrow">TASK QUEUE / INTERACTIVE DEMO</span><span className="mono">SAMPLE DATA · 4 OPERATIONS</span></div>{queue.map((task, index) => { const state = index === 0 ? queueState : task.state; const status = index === 0 ? queueStatus : task.status; return <button type="button" className={`task-row ${selectedTask === index ? "is-selected" : ""}`} data-state={task.kind} key={task.name} onClick={() => setSelectedTask(selectedTask === index ? null : index)} aria-expanded={selectedTask === index}><span className="task-order mono">0{index + 1}</span><span className="task-name"><strong>{task.name}</strong><small>{state}</small></span><span className={`task-state task-state--${task.kind}`}>{status}</span>{index === 0 ? <span className="task-progress" role="progressbar" aria-label={`${task.name} sample progress`} aria-valuemin={0} aria-valuemax={100} aria-valuenow={Math.round(queueProgress)}><i style={{ "--task-progress": queueProgress / 100 } as CSSProperties} /></span> : index === 1 ? <span className="task-indicator">·</span> : <span className="task-indicator">{task.kind === "recovery" ? "↻" : "Ⅱ"}</span>}</button>; })}{selectedQueueTask && <div className="queue-inspector" aria-live="polite"><div><span className="eyebrow">SELECTED TASK / 0{selectedTask! + 1}</span><strong>{selectedQueueTask.name}</strong><p>{selectedTask === 0 ? `${selectedQueueTask.detail} · ${Math.round(queueProgress)}% complete` : selectedQueueTask.detail}</p></div><div className="queue-actions"><button type="button" className="button button--secondary" onClick={handleQueueAction}>{selectedTask === 0 ? (queuePaused ? "Resume task" : "Pause task") : selectedTask === 2 ? "Resume task" : selectedTask === 3 ? "Retry task" : "Start task"}</button><span className="mono">Click another row to inspect it</span></div></div>}</div></div></section>

      <section className="section-wrap integrity-section"><div className="section-intro section-intro--center" data-reveal="up"><h2>Rebuild with receipts, not hope.</h2><p>Interactive sample of the verification flow. Real files are checked in the Web App.</p></div><VerificationDemo /></section>

      <section className="section-wrap evidence-section"><div className="section-intro section-intro--center" data-reveal="up"><h2>Tested beyond the happy path.</h2><p>Small, meaningful proof points from the workflows that matter.</p></div><div className="evidence-grid" data-reveal="up">{evidence.map(([number, title, detail]) => <article className="evidence-card" key={title}><strong>{number}</strong><span>{title}</span><p>{detail}</p></article>)}</div><div className="evidence-foot mono">122 RUST TESTS <i /> 98 NODE TESTS <i /> 12 PRODUCTION BROWSER TESTS <i /> CAKE PACKAGE FORMAT 1.0</div></section>

      <section className="section-wrap roadmap-section" id="roadmap"><div className="section-intro" data-reveal="up"><h2>Useful now. More to come.</h2><p>v0.7 makes the development source public while the packaged release continues toward v0.8.</p></div><div className="roadmap" data-reveal="up">{[["v0.3", "Web large-file workflows", "past"], ["v0.4", "Windows Desktop", "past"], ["v0.5", "Operational reliability", "past"], ["v0.6", "CLI and automation", "past"], ["v0.7", "GitHub source preview", "current"], ["v0.8", "First packaged release", "future"]].map(([version, label, state]) => <div className={`roadmap-node ${state}`} key={version}><span className="mono">{version}</span><i /><strong>{label}</strong>{state === "current" && <small>YOU ARE HERE</small>}</div>)}</div></section>

      <section className="section-wrap faq-section" id="faq"><div className="faq-layout"><div className="section-intro" data-reveal="up"><h2>Good questions make better workflows.</h2><p>Here is what the preview build does—and does not—promise.</p></div><div className="faq-list" data-reveal="up">{faqs.map(([question, answer], index) => { const isOpen = openFaq === index; const answerId = `faq-answer-${index}`; return <div className={`faq-item ${isOpen ? "is-open" : ""}`} key={question}><button type="button" onClick={() => setOpenFaq(isOpen ? null : index)} aria-expanded={isOpen} aria-controls={answerId}><span>{question}</span><i aria-hidden="true">{isOpen ? "−" : "+"}</i></button><div id={answerId} className="faq-answer" role="region" aria-hidden={!isOpen}><p>{answer}</p></div></div>; })}</div></div></section>

      <section className="final-cta section-wrap" id="docs"><div className="final-cta-inner" data-reveal="up"><span className="eyebrow">SPLITTHECAKE / v0.7 SOURCE PREVIEW</span><h2>Your file can be divided.<br /><em>Its integrity shouldn’t be.</em></h2><p>Open the local browser workflow or inspect the source on GitHub to see exactly what moves, what verifies, and what rebuilds.</p><div className="hero-actions"><AppPreviewLink /><a className="button button--secondary" href={githubUrl} target="_blank" rel="noopener noreferrer">View source <span>↗</span></a></div><div className="release-note"><span className="status-dot" /> Current development version <strong>v0.7</strong><span className="mono">·</span> Packaged release planned for <strong>v0.8</strong></div></div></section>

      <section className="support-section section-wrap" id="donate"><div className="support-inner panel"><div><span className="eyebrow">SUPPORT LOCAL-FIRST TOOLS</span><h2>Keep the workflow independent.</h2></div><div><p>Development is public on GitHub. Donations and packaged downloads remain planned for the v0.8 release.</p><a className="text-link" href={githubUrl} target="_blank" rel="noopener noreferrer">Follow on GitHub <span>↗</span></a></div></div></section>
      <footer className="site-footer section-wrap"><a className="brand" href="#top"><span className="brand-mark">◒</span><span>SplitTheCake</span></a><span>Built around Cake Package format 1.0.</span><div><Link href="/about">About</Link><a href={githubUrl} target="_blank" rel="noopener noreferrer">GitHub</a><Link href="/contact">Contact</Link><a href="#donate">Donate</a><Link href="/privacy">Privacy</Link><a href="#faq">FAQ</a><a href="#top">Back to top ↑</a></div></footer>
    </main>
  );
}
