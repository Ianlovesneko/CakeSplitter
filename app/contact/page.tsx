import Link from "next/link";

const contactEmail = "enmity.coyotes.3o@icloud.com";
const githubUrl = "https://github.com/Ianlovesneko/CakeSplitter";

export default function ContactPage() {
  return (
    <main className="privacy-page contact-page">
      <header className="site-header"><Link className="brand" href="/"><span className="brand-mark">◒</span><span>SplitTheCake</span></Link><div className="header-actions"><span className="preview-pill"><i /> Development <span className="preview-word">Preview</span></span><Link className="button button--primary" href="/app">Open the Web App <span aria-hidden="true">↗</span></Link></div></header>
      <section className="privacy-hero contact-hero section-wrap"><div><span className="section-number mono">CONTACT / SPLITTHECAKE</span><h1>Keep the conversation<br /><em>local and direct.</em></h1><p>Have a question about CakeSplitter, the Cake Package format, or the development preview? Send a note and we will get back to you.</p></div><a className="contact-card panel" href={`mailto:${contactEmail}`}><span className="eyebrow">DIRECT EMAIL</span><strong>{contactEmail}</strong><span className="verified-text">Open your mail app ↗</span></a></section>
      <section className="contact-note section-wrap"><div className="panel"><span className="eyebrow">A NOTE ABOUT REPLIES</span><p>This is the direct contact for the SplitTheCake preview. Please do not attach private files or sensitive data to an email.</p></div></section>
      <section className="privacy-cta section-wrap"><div className="final-cta-inner"><span className="eyebrow">LOCAL FILES. CLEAR CONTROLS.</span><h2>See the workflow<br /><em>without sending the file.</em></h2><div className="hero-actions"><Link className="button button--primary" href="/app">Open the Web App <span aria-hidden="true">↗</span></Link><Link className="button button--secondary" href="/">Back to overview <span aria-hidden="true">←</span></Link></div></div></section>
      <footer className="site-footer section-wrap"><Link className="brand" href="/"><span className="brand-mark">◒</span><span>SplitTheCake</span></Link><span>Built around Cake Package format 1.0.</span><div aria-label="Footer navigation"><Link href="/about">About</Link><a href={githubUrl} target="_blank" rel="noopener noreferrer">GitHub</a><Link href="/contact">Contact</Link><Link href="/#donate">Donate</Link><Link href="/privacy">Privacy</Link><Link href="/#faq">FAQ</Link><Link href="/">Back to overview ↑</Link></div></footer>
    </main>
  );
}
