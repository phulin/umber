import assert from "node:assert/strict";
import { createHash, webcrypto } from "node:crypto";
import test from "node:test";

import {
	HttpManifestResolver,
	ManifestResolverError,
} from "./manifest-resolver.js";
import { MemoryObjectCache } from "./persistent-cache.js";

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

function formatEntry(bytes) {
	const sha256 = digest(bytes);
	return {
		object: `sha256-${sha256}`,
		sha256,
		bytes: bytes.byteLength,
		engine: "umber",
		engineVersion: "0.1.0",
		formatSchema: 4,
		sourceDistribution: "texlive-2025",
		sourceManifestSha256: "1".repeat(64),
		sourceDateEpoch: 0,
	};
}

function fontEntry(bytes) {
	const sha256 = digest(bytes);
	return {
		object: `sha256-${sha256}`,
		sha256,
		bytes: bytes.byteLength,
		container: "woff2",
		provenance: "fixture license",
	};
}

function fixture() {
	const bytes = {
		plain: new TextEncoder().encode("plain"),
		cmr: new TextEncoder().encode("cmr"),
		alias: new TextEncoder().encode("plain"),
		badHint: new TextEncoder().encode("hint"),
		font: new Uint8Array([0x77, 0x4f, 0x46, 0x32]),
		format: new Uint8Array([0, 1, 0, 2]),
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
			fonts: { cmr10: fontEntry(bytes.font) },
			formats: { plain: formatEntry(bytes.format) },
		},
		bytes,
	};
}

function response(bytes, options = {}) {
	return new Response(bytes, {
		status: options.status ?? 200,
		headers: options.withoutLength
			? {}
			: { "content-length": String(bytes.byteLength) },
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
	const downloads = await resolver.resolve(
		[
			{ kind: "tex", name: "plain.tex" },
			{ kind: "tex", name: "alias.tex" },
			{ kind: "tfm", name: "cmr10.tfm" },
			{ kind: "tex", name: "plain.tex" },
		],
		{ signal: undefined, prefetchHints: [] },
	);

	assert.equal(maximum, 2);
	assert.equal(calls.length, 2, "plain and alias share one content hash");
	assert.ok(calls.every(({ options }) => options.signal === undefined));
	assert.deepEqual(
		downloads.map(({ type, domain, kind, name }) => ({
			type,
			domain,
			kind,
			name,
		})),
		[
			{ type: "file", domain: "tex", kind: "tex", name: "plain.tex" },
			{ type: "file", domain: "tex", kind: "tex", name: "alias.tex" },
			{ type: "file", domain: "tex", kind: "tfm", name: "cmr10.tfm" },
		],
	);
	assert.equal(downloads[0].bytes, downloads[1].bytes);
});

test("answers manifest file and font misses with typed unavailable responses", async () => {
	const { manifest } = fixture();
	let fetches = 0;
	const resolver = new HttpManifestResolver(manifest, {
		fetch() {
			fetches += 1;
		},
		crypto: webcrypto,
	});
	const variations = [];
	const features = [];
	const responses = await resolver.resolve([
		{ type: "file", domain: "tex", kind: "tex", name: "absent.cfg" },
		{
			type: "font",
			logicalName: "absent-font",
			faceIndex: 0,
			variations,
			features,
		},
	]);
	assert.deepEqual(responses, [
		{
			type: "file-unavailable",
			domain: "tex",
			kind: "tex",
			name: "absent.cfg",
		},
		{
			type: "font-unavailable",
			logicalName: "absent-font",
			faceIndex: 0,
			variations,
			features,
		},
	]);
	assert.equal(fetches, 0);
});

test("resolves an explicit application-manifest font binding", async () => {
	const { manifest, bytes } = fixture();
	const resolver = new HttpManifestResolver(manifest, {
		async fetch() {
			return response(bytes.font);
		},
		crypto: webcrypto,
	});
	const request = {
		type: "font",
		logicalName: "cmr10",
		faceIndex: 0,
		variations: [],
		features: [
			{ tag: "kern", enabled: true },
			{ tag: "liga", enabled: true },
		],
		acceptedContainers: ["woff2"],
		purposes: ["layout", "html"],
	};

	const [resolved] = await resolver.resolve([request]);

	assert.deepEqual(resolved.bytes, bytes.font);
	assert.equal(resolved.type, "font");
	assert.equal(resolved.logicalName, "cmr10");
	assert.equal(resolved.container, "woff2");
	assert.equal(resolved.objectSha256, digest(bytes.font));
	assert.equal(resolved.provenance, "fixture license");
});

test("downloads a compatible named format through the verified object cache", async () => {
	const { manifest, bytes } = fixture();
	let fetches = 0;
	const resolver = new HttpManifestResolver(manifest, {
		async fetch() {
			fetches += 1;
			return response(bytes.format);
		},
		crypto: webcrypto,
	});

	assert.deepEqual(
		await resolver.resolveFormat("plain", {
			engineVersion: "0.1.0",
			formatSchema: 4,
		}),
		bytes.format,
	);
	assert.equal(
		resolver.formatMetadata("plain").sourceDistribution,
		"texlive-2025",
	);
	await resolver.resolveFormat("plain");
	assert.equal(fetches, 1);
});

test("rejects missing or incompatible formats before downloading", async (t) => {
	const { manifest } = fixture();
	let fetches = 0;
	const resolver = new HttpManifestResolver(manifest, {
		fetch() {
			fetches += 1;
		},
		crypto: webcrypto,
	});
	const cases = [
		["missing-format", () => resolver.resolveFormat("latex")],
		[
			"incompatible-format",
			() => resolver.resolveFormat("plain", { engineVersion: "0.2.0" }),
		],
		[
			"incompatible-format",
			() => resolver.resolveFormat("plain", { formatSchema: 5 }),
		],
	];
	for (const [code, operation] of cases) {
		await t.test(code, async () => {
			await assert.rejects(operation(), (error) => error.code === code);
		});
	}
	assert.equal(fetches, 0);
});

test("rejects malformed format compatibility metadata", () => {
	const { manifest } = fixture();
	manifest.formats.plain.formatSchema = 0;
	assert.throws(
		() =>
			new HttpManifestResolver(manifest, {
				fetch: () => {},
				crypto: webcrypto,
			}),
		/invalid compatibility metadata/,
	);
});

test("freezes validated metadata so it cannot redirect later fetches", async () => {
	const { manifest, bytes } = fixture();
	manifest.files["tex:plain.tex"].dependencies = [];
	const calls = [];
	const resolver = new HttpManifestResolver(manifest, {
		async fetch(url) {
			calls.push(url);
			return response(bytes.plain);
		},
		crypto: webcrypto,
	});

	assert.throws(() => {
		resolver.manifest.objectsBaseUrl = "https://attacker.invalid/";
	}, TypeError);
	assert.throws(() => {
		resolver.manifest.files["tex:plain.tex"].object =
			"https://attacker.invalid/object";
	}, TypeError);
	assert.throws(() => {
		resolver.manifest.files["tex:plain.tex"].dependencies.push(
			"tex:attacker.tex",
		);
	}, TypeError);
	assert.throws(() => {
		resolver.formatMetadata("plain").formatSchema = 99;
	}, TypeError);

	await resolver.resolve([{ kind: "tex", name: "plain.tex" }]);
	assert.deepEqual(calls, [
		`${manifest.objectsBaseUrl}${manifest.files["tex:plain.tex"].object}`,
	]);
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
			const resolver = new HttpManifestResolver(manifest, {
				fetch,
				crypto: webcrypto,
			});
			await assert.rejects(resolver.resolve(request), (error) => {
				assert(error instanceof ManifestResolverError);
				assert.equal(error.code, code);
				assert.match(error.message, /cannot resolve tex:plain\.tex/);
				return true;
			});
		});
	}
});

test("cancels an oversized chunked object before buffering the full body", async () => {
	const { manifest } = fixture();
	manifest.files["tex:plain.tex"].dependencies = [];
	let pulls = 0;
	let cancelled = false;
	const resolver = new HttpManifestResolver(manifest, {
		async fetch() {
			return new Response(
				new ReadableStream({
					pull(controller) {
						pulls += 1;
						controller.enqueue(new Uint8Array([1, 2, 3]));
					},
					cancel() {
						cancelled = true;
					},
				}),
			);
		},
		crypto: webcrypto,
	});

	await assert.rejects(
		resolver.resolve([{ kind: "tex", name: "plain.tex" }]),
		(error) => error.code === "object-length",
	);
	assert(cancelled, "oversized response stream was not cancelled");
	assert(pulls < 10, `oversized response pulled ${pulls} chunks`);
});

test("bounds manifest responses and declared object sizes", async () => {
	const { manifest } = fixture();
	await assert.rejects(
		HttpManifestResolver.create({
			manifestUrl: "https://cdn.example.test/manifest.json",
			fetch: async () =>
				new Response("{}", {
					headers: { "content-length": String(64 * 1024 * 1024 + 1) },
				}),
			crypto: webcrypto,
		}),
		(error) => error.code === "manifest-length",
	);

	manifest.files["tex:plain.tex"].bytes = 128 * 1024 * 1024 + 1;
	assert.throws(
		() =>
			new HttpManifestResolver(manifest, {
				fetch: () => {},
				crypto: webcrypto,
			}),
		/invalid byte length/,
	);
});

test("accepts a pinned LaTeX-scale format object", () => {
	const { manifest } = fixture();
	manifest.formats.plain.bytes = 74_240_748;
	assert.equal(
		new HttpManifestResolver(manifest, {
			fetch: () => {},
			crypto: webcrypto,
		}).formatMetadata("plain").bytes,
		74_240_748,
	);
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
			object === manifest.files["tex:plain.tex"].object
				? bytes.plain
				: bytes.cmr;
		return response(source);
	};
	const resolver = new HttpManifestResolver(manifest, {
		fetch,
		crypto: webcrypto,
	});
	const downloads = await resolver.resolve([
		{ kind: "tex", name: "plain.tex" },
	]);
	assert.deepEqual(
		downloads.map(({ name }) => name),
		["plain.tex", "cmr10.tfm"],
	);
	await assert.rejects(
		resolver.resolve([{ kind: "tex", name: "hint.tex" }]),
		/cannot resolve tex:hint\.tex/,
	);
	assert.equal(hintCalls, 2);
});

test("rejects over-budget dependency closures before object fetches", async (t) => {
	const { manifest } = fixture();
	const cases = [
		["files", { maxFiles: 2, maxBytes: 64 * 1024 * 1024 }],
		["bytes", { maxFiles: 512, maxBytes: 4 }],
	];
	for (const [name, limits] of cases) {
		await t.test(name, async () => {
			let fetches = 0;
			const resolver = new HttpManifestResolver(manifest, {
				fetch() {
					fetches += 1;
				},
				crypto: webcrypto,
				...limits,
			});
			await assert.rejects(
				resolver.resolve([{ kind: "tex", name: "plain.tex" }]),
				(error) => error.code === "resource-limit",
			);
			assert.equal(fetches, 0);
		});
	}
});

test("budget accounting counts aliases by logical file and unique path bytes", async () => {
	const { manifest, bytes } = fixture();
	manifest.files["tex:plain.tex"].dependencies = [];
	manifest.files["tex:alias.tex"].virtualPath =
		manifest.files["tex:plain.tex"].virtualPath;
	let fetches = 0;
	const resolver = new HttpManifestResolver(manifest, {
		async fetch() {
			fetches += 1;
			return response(bytes.plain);
		},
		crypto: webcrypto,
		maxFiles: 2,
		maxBytes: bytes.plain.byteLength,
	});
	const downloads = await resolver.resolve([
		{ kind: "tex", name: "plain.tex" },
		{ kind: "tex", name: "alias.tex" },
	]);
	assert.equal(downloads.length, 2);
	assert.equal(fetches, 1);
});

test("warm resolver cache performs no object downloads and requests HTTP caching", async () => {
	const { manifest, bytes } = fixture();
	manifest.files["tex:plain.tex"].dependencies = [];
	const calls = [];
	const fetch = async (url, options) => {
		calls.push({ url, options });
		return response(bytes.plain);
	};
	const resolver = new HttpManifestResolver(manifest, {
		fetch,
		crypto: webcrypto,
	});
	const request = [{ kind: "tex", name: "plain.tex" }];
	await resolver.resolve(request);
	await resolver.resolve(request);
	assert.equal(calls.length, 1);
	assert.equal(calls[0].options.cache, "force-cache");
});

test("persistent cache is isolated by distribution and avoids later downloads", async () => {
	const { manifest, bytes } = fixture();
	manifest.files["tex:plain.tex"].dependencies = [];
	const cacheStore = new MemoryObjectCache();
	let fetches = 0;
	const fetch = async () => {
		fetches += 1;
		return response(bytes.plain);
	};
	const request = [{ kind: "tex", name: "plain.tex" }];
	const first = new HttpManifestResolver(manifest, {
		fetch,
		crypto: webcrypto,
		persistentCache: "indexeddb",
		cacheStore,
	});
	await first.resolve(request);
	const warm = new HttpManifestResolver(manifest, {
		fetch,
		crypto: webcrypto,
		persistentCache: "indexeddb",
		cacheStore,
	});
	await warm.resolve(request);
	assert.equal(fetches, 1);

	const nextManifest = { ...manifest, distribution: "texlive-fixture-next" };
	const isolated = new HttpManifestResolver(nextManifest, {
		fetch,
		crypto: webcrypto,
		persistentCache: "indexeddb",
		cacheStore,
	});
	await isolated.resolve(request);
	assert.equal(
		fetches,
		2,
		"a different distribution must not reuse the object",
	);
});

test("corrupt persistent bytes are evicted and replaced from the network", async () => {
	const { manifest, bytes } = fixture();
	manifest.files["tex:plain.tex"].dependencies = [];
	const entry = manifest.files["tex:plain.tex"];
	const cacheStore = new MemoryObjectCache();
	await cacheStore.put(
		manifest.distribution,
		entry.sha256,
		new Uint8Array(entry.bytes),
	);
	let fetches = 0;
	const resolver = new HttpManifestResolver(manifest, {
		async fetch() {
			fetches += 1;
			return response(bytes.plain);
		},
		crypto: webcrypto,
		persistentCache: "indexeddb",
		cacheStore,
	});
	await resolver.resolve([{ kind: "tex", name: "plain.tex" }]);
	assert.equal(fetches, 1);
	assert.deepEqual(
		await cacheStore.get(manifest.distribution, entry.sha256),
		bytes.plain,
	);
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
	assert.equal(
		calls[1].url,
		`${manifest.objectsBaseUrl}${manifest.files["tex:plain.tex"].object}`,
	);
});

test("rejects unsafe manifest object and virtual path metadata", () => {
	const { manifest } = fixture();
	manifest.files["tex:plain.tex"].object = "../plain.tex";
	assert.throws(
		() =>
			new HttpManifestResolver(manifest, {
				fetch: () => {},
				crypto: webcrypto,
			}),
		/invalid object name/,
	);
});
