import type { Metadata } from "next";
import "./globals.css";

export const metadata: Metadata = {
  metadataBase: new URL("https://splitthecake.ianlovesneko.chatgpt.site"),
  title: "SplitTheCake — Split large files. Rebuild them exactly.",
  description:
    "CakeSplitter turns one large local file into verified .slice parts, then reconstructs the original byte for byte.",
  keywords: ["CakeSplitter", "file splitting", "SHA-256", "local-first", "slice workflow"],
  icons: {
    icon: "/favicon.svg",
    shortcut: "/favicon.svg",
  },
  openGraph: {
    title: "SplitTheCake — Split large files. Rebuild them exactly.",
    description: "A local-first, verifiable workflow for splitting and rebuilding large files.",
    type: "website",
    images: [{ url: "/og.png", width: 1731, height: 909, alt: "SplitTheCake — Split large files. Rebuild them exactly." }],
  },
  twitter: {
    card: "summary_large_image",
    title: "SplitTheCake — Split large files. Rebuild them exactly.",
    description: "A local-first, verifiable workflow for splitting and rebuilding large files.",
    images: ["/og.png"],
  },
};

export default function RootLayout({
  children,
}: Readonly<{
  children: React.ReactNode;
}>) {
  return (
    <html lang="en">
      <body>{children}</body>
    </html>
  );
}
