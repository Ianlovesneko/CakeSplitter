import Link from "next/link";

const githubUrl = "https://github.com/Ianlovesneko/CakeSplitter";

export default function NotFound() {
  return (
    <main className="not-found-page">
      <header className="site-header">
        <Link className="brand" href="/"><span className="brand-mark">◒</span><span>SplitTheCake</span></Link>
        <div className="header-actions"><span className="preview-pill"><i /> Development <span className="preview-word">Preview</span></span><Link className="button button--primary" href="/app">Open the Web App <span aria-hidden="true">↗</span></Link></div>
      </header>
      <section className="not-found-main section-wrap">
        <div className="not-found-card panel">
          <span className="section-number mono">ERROR / SLICE NOT FOUND</span>
          <h1>This page is missing.</h1>
          <p>The route you requested is not part of this Cake Package. Return to the overview or open the local workflow.</p>
          <div className="hero-actions"><Link className="button button--primary" href="/">Back to overview <span aria-hidden="true">↩</span></Link><Link className="button button--secondary" href="/app">Open the Web App <span aria-hidden="true">↗</span></Link></div>
        </div>
      </section>
      <footer className="site-footer section-wrap"><Link className="brand" href="/"><span className="brand-mark">◒</span><span>SplitTheCake</span></Link><span>Built around Cake Package format 1.0.</span><div><Link href="/about">About</Link><a href={githubUrl} target="_blank" rel="noopener noreferrer">GitHub</a><Link href="/contact">Contact</Link><Link href="/privacy">Privacy</Link><Link href="/">Back to overview ↑</Link></div></footer>
    </main>
  );
}
