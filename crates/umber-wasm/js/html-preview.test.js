import assert from "node:assert/strict";
import test from "node:test";

import { installHtmlPreview } from "./html-preview.js";

const csp = "default-src 'none'; font-src data: 'self'; style-src 'unsafe-inline'; img-src data:";
const canonical = `<!doctype html>\n<html lang="und"><head><meta name="generator" content="umber-html/1"><meta http-equiv="Content-Security-Policy" content="${csp}"></head><body></body></html>\n`;

test("installs canonical output into a scriptless iframe", () => {
	const attributes = new Map();
	const iframe = { setAttribute: (name, value) => attributes.set(name, value) };
	installHtmlPreview(iframe, new TextEncoder().encode(canonical));
	assert.equal(iframe.srcdoc, canonical);
	assert.equal(attributes.get("sandbox"), "");
	assert.equal(attributes.get("referrerpolicy"), "no-referrer");
});

test("rejects active or non-Umber markup", () => {
	const iframe = { setAttribute() {} };
	assert.throws(() => installHtmlPreview(iframe, "<script>alert(1)</script>"));
	assert.throws(() => installHtmlPreview(iframe, canonical.replace("</body>", "<iframe></iframe></body>")));
});
