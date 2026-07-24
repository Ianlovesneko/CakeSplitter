import Image from "next/image";
import Link from "next/link";

const productOverviewPdf =
  "/docs/CakeSplitter-v0.5.0-Complete-Product-Overview-Revised.pdf";
const productOverviewCover =
  "/docs/CakeSplitter-v0.5.0-Complete-Product-Overview-Revised-cover.png";
const githubUrl = "https://github.com/Ianlovesneko/CakeSplitter";

const principles = [
  [
    "Local-first processing",
    "Files stay on your device while the workflow does its work.",
  ],
  [
    "Per-Slice verification",
    "Every part carries evidence that its bytes arrived unchanged.",
  ],
  [
    "Exact reconstruction",
    "A rebuild is complete only when it matches the original.",
  ],
  [
    "Portable Cake Package",
    "Slices and one manifest can move together across tools.",
  ],
  [
    "Pause and recovery",
    "Long-running work is designed for interruption and a safe restart.",
  ],
  [
    "Clear receipts and diagnostics",
    "The workflow shows what happened instead of hiding it.",
  ],
];

const audiences = [
  "Creators moving large media files",
  "Students and researchers archiving datasets",
  "Developers handling builds and disk images",
  "Teams working around platform size limits",
  "Anyone who wants verifiable file reconstruction",
];

const overviewHighlights = [
  "Product concept and terminology",
  "Split, Merge, Inspect, and Verify workflows",
  "Cake Package format 1.0",
  "Integrity and reliability model",
  "Privacy and security boundaries",
  "Web, Desktop, CLI, and Rust architecture",
  "Validation and large-file evidence",
  "Known limitations and roadmap",
];

export default function AboutPage() {
  return (
    <main className="privacy-page about-page">
      <header className="site-header">
        <Link className="brand" href="/">
          <span className="brand-mark">◒</span>
          <span>SplitTheCake</span>
        </Link>
        <nav className="desktop-nav" aria-label="Primary navigation">
          <Link href="/about">About</Link>
          <Link href="/#workflow">How it works</Link>
          <Link href="/#trust">Trust</Link>
          <Link href="/#faq">FAQ</Link>
        </nav>
        <div className="header-actions">
          <span className="preview-pill">
            <i /> Development <span className="preview-word">Preview</span>
          </span>
          <Link className="button button--primary" href="/app">
            Open the Web App <span aria-hidden="true">↗</span>
          </Link>
        </div>
      </header>

      <section className="about-hero section-wrap">
        <div>
          <span className="section-number mono">ABOUT / CAKESPLITTER</span>
          <h1>
            Large files deserve
            <br />
            <em>clear evidence.</em>
          </h1>
          <p>
            CakeSplitter is a local-first tool for splitting large files into
            verified Slices and rebuilding them exactly.
          </p>
        </div>
        <div className="about-package panel" aria-label="Cake Package model">
          <span className="eyebrow">ONE PACKAGE / MANY SLICES</span>
          <div className="about-package__parts">
            <span>.slice</span>
            <span>.slice</span>
            <span>.slice</span>
            <i aria-hidden="true">+</i>
            <strong>.cake.json</strong>
          </div>
          <p>
            Portable parts with one manifest preserving order, size, hashes, and
            the original file name.
          </p>
        </div>
      </section>

      <section className="about-content section-wrap">
        <div className="about-block about-block--wide">
          <span className="eyebrow">WHAT WE BUILT</span>
          <h2>One product, three surfaces, one package model.</h2>
          <p>
            <strong>CakeSplitter</strong> is the core product and desktop / CLI
            workflow. <strong>SplitTheCake</strong> is its browser experience
            and product site. The <strong>Cake Package</strong> is the portable
            format that connects them, so a package made in one surface can be
            inspected and rebuilt in another. The current v0.7 development
            source is now publicly available on{" "}
            <a href={githubUrl} target="_blank" rel="noopener noreferrer">
              GitHub
            </a>
            .
          </p>
        </div>

        <div className="about-story-grid">
          <article className="about-block panel">
            <span className="eyebrow">THE PROBLEM</span>
            <h2>Moving a big file should not feel like guessing.</h2>
            <p>
              Platforms impose upload limits. Connections fail. Transfers get
              interrupted. After a file is divided, ordinary tools rarely make
              it obvious whether every part is present, complete, and safe to
              rebuild.
            </p>
          </article>
          <article className="about-block panel">
            <span className="eyebrow">HOW IT WORKS</span>
            <div className="about-flow">
              <div>
                <span>Choose a file</span>
              </div>
              <div>
                <span>Split into verified Slices</span>
              </div>
              <div>
                <span>Move or store the Cake Package</span>
              </div>
              <div>
                <span>Inspect and rebuild exactly</span>
              </div>
            </div>
          </article>
        </div>

        <div className="about-block about-block--principles">
          <span className="eyebrow">WHAT MATTERS MOST</span>
          <h2>Verification belongs inside the workflow.</h2>
          <div className="about-principles">
            {principles.map(([title, text]) => (
              <article key={title}>
                <span className="about-principle-dot" aria-hidden="true">
                  ✓
                </span>
                <h3>{title}</h3>
                <p>{text}</p>
              </article>
            ))}
          </div>
        </div>

        <div className="about-story-grid about-story-grid--reverse">
          <article className="about-block panel">
            <span className="eyebrow">WHY CAKESPLITTER</span>
            <ul className="about-check-list">
              <li>
                Verification is part of the workflow, not an optional
                afterthought.
              </li>
              <li>Web, Desktop, and CLI use the same package model.</li>
              <li>
                Files are processed locally rather than uploaded to a service.
              </li>
              <li>
                Failures are surfaced clearly instead of silently ignored.
              </li>
              <li>
                Long-running tasks are designed for interruption and recovery.
              </li>
            </ul>
          </article>
          <article className="about-block panel">
            <span className="eyebrow">WHO IT IS FOR</span>
            <ul className="about-audience-list">
              {audiences.map((audience) => (
                <li key={audience}>{audience}</li>
              ))}
            </ul>
          </article>
        </div>

        <div className="about-closing-grid">
          <article className="about-block">
            <span className="eyebrow">VISION</span>
            <h2>More visible, verifiable, and controllable.</h2>
            <blockquote>
              We believe basic file operations should provide clear evidence of
              what happened. CakeSplitter is being built to make large-file
              workflows more visible, verifiable, and controllable across
              browsers, desktops, and automation tools.
            </blockquote>
          </article>
          <article className="about-block panel">
            <span className="eyebrow">BUILT BY</span>
            <h3>Independent by design.</h3>
            <p>
              CakeSplitter is designed and developed by Yu-En Huang as an
              independent software project focused on reliable, understandable,
              and privacy-aware file workflows.
            </p>
            <p>
              It began with a simple question: if a file is divided into parts,
              how can users know with confidence that every part is present and
              the rebuilt result is truly identical?
            </p>
          </article>
        </div>
      </section>

      <section
        className="about-document section-wrap"
        aria-labelledby="product-overview-heading"
      >
        <div className="about-document__preview panel">
          <span className="eyebrow">DOCUMENT PREVIEW</span>
          <div className="about-document__cover">
            <Image
              src={productOverviewCover}
              alt="Cover of the archived CakeSplitter v0.5.0 Complete Product Overview"
              width={595}
              height={842}
              loading="lazy"
              sizes="(max-width: 980px) 80vw, 420px"
            />
          </div>
          <span className="about-document__preview-note mono">
            21-PAGE LOCAL RELEASE DOCUMENTATION
          </span>
        </div>
        <div className="about-document__details">
          <span className="eyebrow">FULL PRODUCT OVERVIEW</span>
          <h2 id="product-overview-heading">
            Explore the complete CakeSplitter system.
          </h2>
          <p>
            Read the archived product overview for CakeSplitter v0.5.0,
            including its product model, Cake Package format, verification
            workflow, local-first privacy boundaries, technical architecture,
            validation evidence, known limitations, and roadmap.
          </p>
          <p className="about-document__current">
            This 21-page document records v0.5.0. For the current v0.7
            development source and latest changes, visit{" "}
            <a href={githubUrl} target="_blank" rel="noopener noreferrer">
              GitHub
            </a>
            .
          </p>
          <dl className="about-document__meta">
            <div>
              <dt>DOCUMENT VERSION</dt>
              <dd>0.5.0</dd>
            </div>
            <div>
              <dt>FORMAT</dt>
              <dd>Cake Package format 1.0</dd>
            </div>
            <div>
              <dt>PAGES</dt>
              <dd>21 pages</dd>
            </div>
            <div>
              <dt>TYPE</dt>
              <dd>Local Release Documentation</dd>
            </div>
            <div>
              <dt>AUTHOR</dt>
              <dd>Yu-En Huang</dd>
            </div>
            <div>
              <dt>UPDATED</dt>
              <dd>23 July 2026</dd>
            </div>
          </dl>
          <div className="about-document__highlights">
            <h3>Inside the overview</h3>
            <ul>
              {overviewHighlights.map((highlight) => (
                <li key={highlight}>{highlight}</li>
              ))}
            </ul>
          </div>
          <div className="hero-actions about-document__actions">
            <a
              className="button button--primary"
              href={productOverviewPdf}
              target="_blank"
              rel="noopener noreferrer"
            >
              Read full overview <span aria-hidden="true">↗</span>
            </a>
            <a
              className="button button--secondary"
              href={productOverviewPdf}
              download="CakeSplitter-v0.5.0-Complete-Product-Overview-Revised.pdf"
            >
              Download PDF <span aria-hidden="true">↓</span>
            </a>
          </div>
        </div>
      </section>

      <section className="privacy-cta section-wrap">
        <div className="final-cta-inner">
          <span className="eyebrow">LOCAL FILES. CLEAR CONTROLS.</span>
          <h2>
            See the workflow
            <br />
            <em>without leaking the files.</em>
          </h2>
          <div className="hero-actions">
            <Link className="button button--primary" href="/app">
              Open the Web App <span aria-hidden="true">↗</span>
            </Link>
            <Link className="button button--secondary" href="/">
              Back to overview <span aria-hidden="true">←</span>
            </Link>
          </div>
        </div>
      </section>
      <footer className="site-footer section-wrap">
        <Link className="brand" href="/">
          <span className="brand-mark">◒</span>
          <span>SplitTheCake</span>
        </Link>
        <span>Built around Cake Package format 1.0.</span>
        <div aria-label="Footer navigation">
          <Link href="/about">About</Link>
          <a href={githubUrl} target="_blank" rel="noopener noreferrer">
            GitHub
          </a>
          <Link href="/contact">Contact</Link>
          <Link href="/#donate">Donate</Link>
          <Link href="/privacy">Privacy</Link>
          <Link href="/#faq">FAQ</Link>
          <Link href="/">Back to overview ↑</Link>
        </div>
      </footer>
    </main>
  );
}
