import assert from "node:assert/strict";
import test from "node:test";

async function render(pathname = "/") {
  const workerUrl = new URL("../dist/server/index.js", import.meta.url);
  workerUrl.searchParams.set("test", `${process.pid}-${Date.now()}`);
  const { default: worker } = await import(workerUrl.href);
  return worker.fetch(
    new Request(`http://localhost${pathname}`, {
      headers: { accept: "text/html" },
    }),
    {
      ASSETS: { fetch: async () => new Response("Not found", { status: 404 }) },
    },
    { waitUntil() {}, passThroughOnException() {} },
  );
}

test("server-renders the SplitTheCake marketing page", async () => {
  const response = await render();
  assert.equal(response.status, 200);
  assert.match(response.headers.get("content-type") ?? "", /^text\/html\b/i);
  const html = await response.text();
  assert.match(
    html,
    /<title>SplitTheCake — Split large files\. Rebuild them exactly\.<\/title>/i,
  );
  assert.match(html, /CURRENT v0\.7/);
  assert.match(
    html,
    /href="https:\/\/github\.com\/Ianlovesneko\/CakeSplitter"[^>]+target="_blank"/,
  );
  assert.match(html, /Split large files\./);
  assert.match(html, /Rebuild them exactly\./);
  assert.match(html, /ABOUT CAKESPLITTER/);
  assert.match(html, /Read the full story/);
  assert.match(
    html,
    /<footer class="site-footer section-wrap">[\s\S]*?<a[^>]+href="\/about">About/,
  );
  assert.match(html, /Web Direct Folder Mode is currently disabled/);
  assert.match(html, /Packaged release planned for/);
  assert.match(html, /data-slice-count="12" data-angle-step="30"/);
  assert.match(html, /id="slice-size"[^>]+max="4"/);
  assert.doesNotMatch(
    html,
    /class="range-labels mono"[^>]*>[\s\S]*<span>5 GB<\/span>/i,
  );
  assert.equal((html.match(/data-cake-sector=/g) ?? []).length, 15);
  assert.match(html, /class="cake-center-cap"/);
  assert.doesNotMatch(html, /cake-core|cake-hit-map|cake-slice/);
  assert.doesNotMatch(
    html,
    /codex-preview|Your site is taking shape|react-loading-skeleton/i,
  );
});

test("server-renders the About page", async () => {
  const response = await render("/about");
  assert.equal(response.status, 200);
  const html = await response.text();
  assert.match(html, /Large files deserve/);
  assert.match(html, /WHAT WE BUILT/);
  assert.doesNotMatch(html, /(?:^|>)0[1-8](?:\s*\/|<)/);
  assert.doesNotMatch(html, /<b>0[1-4]<\/b>/);
  assert.match(html, /Verification is part of the workflow/);
  assert.match(html, /Yu-En Huang/);
  assert.match(html, /href="\/about"[^>]*>About/);
  assert.match(html, /FULL PRODUCT OVERVIEW/);
  assert.match(html, /current v0\.7 development source/i);
  assert.match(
    html,
    /CakeSplitter-v0\.5\.0-Complete-Product-Overview-Revised\.pdf/,
  );
  assert.match(html, /target="_blank"[^>]+rel="noopener noreferrer"/);
  assert.match(
    html,
    /download="CakeSplitter-v0\.5\.0-Complete-Product-Overview-Revised\.pdf"/,
  );
  assert.match(html, /21 pages/);
});

test("server-renders the local browser app route", async () => {
  const response = await render("/app");
  assert.equal(response.status, 200);
  const html = await response.text();
  assert.match(html, /Split, verify, rebuild/);
  assert.match(html, /Nothing leaves this device/);
  assert.match(html, /Cake manifest/);
  assert.match(html, /id="file-input"[^>]+aria-describedby="file-status"/);
  assert.match(html, /id="app-slice-size"[^>]+max="4"/);
});

test("server-renders the local-only privacy page", async () => {
  const response = await render("/privacy");
  assert.equal(response.status, 200);
  const html = await response.text();
  assert.match(html, /Privacy is a product behavior/);
  assert.match(html, /Files stay local/);
  assert.match(html, /REMOTE UPLOAD/);
  assert.doesNotMatch(html, /<span class="mono">0[1-3]<\/span>/);
  assert.match(html, /This public page still makes ordinary network requests/);
});

test("renders a branded not-found page", async () => {
  const response = await render("/nonexistent-page-test");
  assert.equal(response.status, 404);
  const html = await response.text();
  assert.match(html, /This page is missing/);
  assert.match(html, /Back to overview/);
  assert.match(html, /Open the Web App/);
});
