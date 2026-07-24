import Link from "next/link";

const githubUrl = "https://github.com/Ianlovesneko/CakeSplitter";

const commitments = [
  ["Files stay local", "The browser workflow is designed to work with files on your device. CakeSplitter does not send selected file contents to this website."],
  ["No account required", "The current Development Preview does not ask for a user account, name, email address, or payment details."],
  ["Verification stays visible", "Slice hashes and rebuild checks are product controls you can inspect, rather than an invisible privacy promise."],
];

export default function PrivacyPage() {
  return (
    <main className="privacy-page">
      <header className="site-header"><Link className="brand" href="/"><span className="brand-mark">◒</span><span>SplitTheCake</span></Link><div className="header-actions"><span className="preview-pill"><i /> Development <span className="preview-word">Preview</span></span><Link className="button button--primary" href="/app">Open the Web App <span aria-hidden="true">↗</span></Link></div></header>
      <section className="privacy-hero section-wrap"><div><span className="section-number mono">PRIVACY / LOCAL-FIRST</span><h1>Privacy is a product behavior.<br /><em>Not a vague promise.</em></h1><p>SplitTheCake is built around local file workflows: split, verify, move, and rebuild without uploading your file contents to this site.</p></div><div className="privacy-signal panel" aria-hidden="true"><span>THIS DEVICE</span><i>×</i><span>REMOTE UPLOAD</span><strong>LOCAL PROCESSING</strong></div></section>
      <section className="privacy-content section-wrap"><div className="privacy-intro"><span className="eyebrow">WHAT THIS MEANS</span><h2>Clear boundaries, plainly stated.</h2><p>The local-first claim is about the product workflow. Your selected files remain under your control while you use CakeSplitter.</p></div><div className="privacy-grid">{commitments.map(([title, text]) => <article className="privacy-card" key={title}><h3>{title}</h3><p>{text}</p></article>)}</div></section>
      <section className="privacy-fineprint section-wrap"><div className="panel"><span className="eyebrow">A NOTE ABOUT THIS WEBSITE</span><p>This public page still makes ordinary network requests to load its content. That is different from uploading the file you select in CakeSplitter. The Development Preview does not include product accounts or a file-upload service.</p></div><div className="panel"><span className="eyebrow">DESKTOP PREVIEW</span><p>Windows Desktop capabilities such as diagnostics and operation receipts are shown as product features. If a future workflow needs data beyond your device, it should explain that before the action begins.</p></div></section>
      <section className="privacy-cta section-wrap"><div className="final-cta-inner"><span className="eyebrow">LOCAL FILES. CLEAR CONTROLS.</span><h2>See the workflow<br /><em>without sending the file.</em></h2><div className="hero-actions"><Link className="button button--primary" href="/app">Open the Web App <span aria-hidden="true">↗</span></Link><Link className="button button--secondary" href="/">Back to overview <span aria-hidden="true">←</span></Link></div></div></section>
      <footer className="site-footer section-wrap"><Link className="brand" href="/"><span className="brand-mark">◒</span><span>SplitTheCake</span></Link><span>Built around Cake Package format 1.0.</span><div aria-label="Footer navigation"><Link href="/about">About</Link><a href={githubUrl} target="_blank" rel="noopener noreferrer">GitHub</a><Link href="/contact">Contact</Link><Link href="/#donate">Donate</Link><Link href="/privacy">Privacy</Link><Link href="/#faq">FAQ</Link><Link href="/">Back to overview ↑</Link></div></footer>
    </main>
  );
}
