import assert from "node:assert/strict";
import { createHash, webcrypto } from "node:crypto";
import test from "node:test";

import { HttpManifestResolver, ManifestResolverError } from "./manifest-resolver.js";

function digest(bytes) {
  return createHash("sha256").update(bytes).digest("hex");
}

function entry(path, bytes, dependencies = []) {
  const sha256 = digest(bytes);
  return {
    virtualPath: `/texlive/${path}`,
    object: `sha256-${sha256}`,
    sha256,
    bytes: bytes.byteLength,
    dependencies,
  };
}

function fixture() {
  const bytes = {
    plain: new TextEncoder().encode("plain"),
    cmr: new TextEncoder().encode("cmr"),
    alias: new TextEncoder().encode("plain"),
    badHint: new TextEncoder().encode("hint"),
  };
  const files = {
    "tex:plain.tex": entry("tex/plain.tex", bytes.plain, [
      "tfm:cmr10.tfm",
      "tex:hint.tex",
    ]),
    "tex:alias.tex": entry("tex/alias.tex", bytes.alias),
    "tfm:cmr10.tfm": entry("fonts/cmr10.tfm", bytes.cmr),
    "tex:hint.tex": entry("tex/hint.tex", bytes.badHint),
  };
  return {
    manifest: {
      schema: 1,
      distribution: "texlive-fixture",
      objectsBaseUrl: "https://cdn.example.test/objects/",
      files,
    },
    bytes,
  };
}

function response(bytes, options = {}) {
  return new Response(bytes, {
    status: options.status ?? 200,
    headers: options.withoutLength ? {} : { "content-length": String(bytes.byteLength) },
  });
}

test("fetches concurrently, deduplicates hashes, and binds every lookup key", async () => {
  const { manifest, bytes } = fixture();
  manifest.files["tex:plain.tex"].dependencies = [];
  let active = 0;
  let maximum = 0;
  const calls = [];
  const byObject = new Map([
    [manifest.files["tex:plain.tex"].object, bytes.plain],
    [manifest.files["tfm:cmr10.tfm"].object, bytes.cmr],
  ]);
  const fetch = async (url, options) => {
    calls.push({ url, options });
    active += 1;
    maximum = Math.max(maximum, active);
    await new Promise((resolve) => setTimeout(resolve, 10));
    active -= 1;
    return response(byObject.get(url.split("/").at(-1)));
  };
  const resolver = new HttpManifestResolver(manifest, {
    fetch,
    crypto: webcrypto,
    concurrency: 2,
  });
  const downloads = await resolver.resolve([
    { kind: "tex", name: "plain.tex" },
    { kind: "tex", name: "alias.tex" },
    { kind: "tfm", name: "cmr10.tfm" },
    { kind: "tex", name: "plain.tex" },
  ]);

  assert.equal(maximum, 2);
  assert.equal(calls.length, 2, "plain and alias share one content hash");
  assert.deepEqual(
    downloads.map(({ request }) => request),
    [
      { kind: "tex", name: "plain.tex" },
      { kind: "tex", name: "alias.tex" },
      { kind: "tfm", name: "cmr10.tfm" },
    ],
  );
  assert.equal(downloads[0].bytes, downloads[1].bytes);
});

test("validates status, byte length, and SHA-256 with actionable request errors", async (t) => {
  const { manifest, bytes } = fixture();
  manifest.files["tex:plain.tex"].dependencies = [];
  const request = [{ kind: "tex", name: "plain.tex" }];
  const cases = [
    ["object-http", async () => response(new Uint8Array(), { status: 404 })],
    ["object-length", async () => response(bytes.plain.subarray(1))],
    ["object-digest", async () => response(new TextEncoder().encode("other"))],
  ];
  for (const [code, fetch] of cases) {
    await t.test(code, async () => {
      const resolver = new HttpManifestResolver(manifest, { fetch, crypto: webcrypto });
      await assert.rejects(resolver.resolve(request), (error) => {
        assert(error instanceof ManifestResolverError);
        assert.equal(error.code, code);
        assert.match(error.message, /cannot resolve tex:plain\.tex/);
        return true;
      });
    });
  }
});

test("failed speculative hints are ignored and retried if actually requested", async () => {
  const { manifest, bytes } = fixture();
  const hintObject = manifest.files["tex:hint.tex"].object;
  let hintCalls = 0;
  const fetch = async (url) => {
    const object = url.split("/").at(-1);
    if (object === hintObject) {
      hintCalls += 1;
      return response(new Uint8Array(), { status: 503 });
    }
    const source =
      object === manifest.files["tex:plain.tex"].object ? bytes.plain : bytes.cmr;
    return response(source);
  };
  const resolver = new HttpManifestResolver(manifest, { fetch, crypto: webcrypto });
  const downloads = await resolver.resolve([{ kind: "tex", name: "plain.tex" }]);
  assert.deepEqual(
    downloads.map(({ request }) => request.name),
    ["plain.tex", "cmr10.tfm"],
  );
  await assert.rejects(
    resolver.resolve([{ kind: "tex", name: "hint.tex" }]),
    /cannot resolve tex:hint\.tex/,
  );
  assert.equal(hintCalls, 2);
});

test("warm resolver cache performs no object downloads and requests HTTP caching", async () => {
  const { manifest, bytes } = fixture();
  manifest.files["tex:plain.tex"].dependencies = [];
  const calls = [];
  const fetch = async (url, options) => {
    calls.push({ url, options });
    return response(bytes.plain);
  };
  const resolver = new HttpManifestResolver(manifest, { fetch, crypto: webcrypto });
  const request = [{ kind: "tex", name: "plain.tex" }];
  await resolver.resolve(request);
  await resolver.resolve(request);
  assert.equal(calls.length, 1);
  assert.equal(calls[0].options.cache, "force-cache");
});

test("loads and validates a manifest through injectable fetch", async () => {
  const { manifest, bytes } = fixture();
  manifest.files["tex:plain.tex"].dependencies = [];
  const calls = [];
  const fetch = async (url, options) => {
    calls.push({ url, options });
    if (url === "https://cdn.example.test/manifest.json") {
      return Response.json(manifest);
    }
    return response(bytes.plain);
  };
  const resolver = await HttpManifestResolver.create({
    manifestUrl: "https://cdn.example.test/manifest.json",
    fetch,
    crypto: webcrypto,
  });
  await resolver.resolve([{ kind: "tex", name: "plain.tex" }]);
  assert.equal(calls[0].options.cache, "force-cache");
  assert.equal(calls[1].url, `${manifest.objectsBaseUrl}${manifest.files["tex:plain.tex"].object}`);
});

test("rejects unsafe manifest object and virtual path metadata", () => {
  const { manifest } = fixture();
  manifest.files["tex:plain.tex"].object = "../plain.tex";
  assert.throws(
    () => new HttpManifestResolver(manifest, { fetch: () => {}, crypto: webcrypto }),
    /invalid object name/,
  );
});
