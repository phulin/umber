import assert from "node:assert/strict";
import { webcrypto } from "node:crypto";
import { readFileSync } from "node:fs";
import test from "node:test";
import {
	deterministicAhash64Hex,
	HttpManifestResolver,
	ManifestResolverError,
} from "./manifest-resolver.js";
import { MemoryObjectCache } from "./persistent-cache.js";

const encoder = new TextEncoder();
const shardIndex = (key, bits) => {
	if (bits === 0) return 0;
	const value = BigInt(`0x${deterministicAhash64Hex(encoder.encode(key), 2)}`);
	const prefix = Number(((value & 0xffn) << 8n) | ((value >> 8n) & 0xffn));
	return prefix >>> (16 - bits);
};
const catalog = {
	catalogCreateSession(text) {
		const retained = new Map();
		return {
			prepareBatch(keys) {
				const prepared = catalog.catalogPrepareBatch(text, keys);
				prepared.shards = prepared.shards.filter(
					({ index }) => !retained.has(index),
				);
				return prepared;
			},
			provideShard(index, bytes) {
				const decoded = new TextDecoder("utf-8", { fatal: true }).decode(bytes);
				const previous = retained.get(index);
				if (previous !== undefined && previous !== decoded)
					throw new Error(`index shard ${index} conflicts with retained bytes`);
				retained.set(index, decoded);
			},
			planBatch(keys) {
				return catalog.catalogPlanBatch(
					text,
					[...retained].map(([index, shardText]) => ({
						index,
						text: shardText,
					})),
					keys,
				);
			},
			selectFormat(name) {
				return catalog.catalogSelectFormat(text, name);
			},
		};
	},
	catalogPrepareBatch(text, keys) {
		const root = JSON.parse(text);
		if (
			![8, 9].includes(root.schema) ||
			!Number.isInteger(root.shardBits) ||
			root.shardBits < 0 ||
			root.shardBits > 16 ||
			root.shardCount !== 2 ** root.shardBits ||
			root.shards?.length !== root.shardCount
		)
			throw new Error("shardBits or shard table is invalid");
		const indexes = [
			...new Set(keys.map((key) => shardIndex(key, root.shardBits))),
		].sort((left, right) => left - right);
		return {
			root: `${JSON.stringify(root)}\n`,
			shards: indexes.map((index) => ({
				index,
				object: `ahash64-v1-${root.shards[index]}`,
				ahash64: root.shards[index],
			})),
		};
	},
	catalogPlanBatch(rootText, rawShards, keys) {
		const root = JSON.parse(rootText);
		const shards = new Map();
		for (const raw of rawShards) {
			if (digest(encoder.encode(raw.text)) !== root.shards[raw.index])
				throw new Error(`index shard ${raw.index} digest mismatch`);
			const shard = JSON.parse(raw.text);
			if (
				shard.distribution !== root.distribution ||
				shard.index !== raw.index ||
				shard.schema !== (root.schema === 9 ? 4 : 3)
			)
				throw new Error(`index shard ${raw.index} identity mismatch`);
			for (const key of [
				...Object.keys(shard.files ?? {}),
				...Object.keys(shard.fonts ?? {}),
				...Object.keys(shard.legacyMappings ?? {}),
			])
				if (shardIndex(key, root.shardBits) !== raw.index)
					throw new Error(`lookup key ${key} is not in canonical shard`);
			shards.set(raw.index, shard);
		}
		const jobs = [];
		const misses = [];
		const seen = new Set();
		const hints = [];
		for (const [requestIndex, key] of keys.entries()) {
			if (seen.has(key)) continue;
			seen.add(key);
			const index = shardIndex(key, root.shardBits);
			const shard = shards.get(index);
			const entries = key.startsWith("font:")
				? (shard.fonts ?? {})
				: key.startsWith("legacy-mapping:")
					? (shard.legacyMappings ?? {})
					: shard.files;
			const entry = entries[key];
			if (entry === undefined) {
				misses.push(requestIndex);
				continue;
			}
			jobs.push({
				manifestKey: key,
				requirement: "required",
				kind: key.startsWith("font:")
					? "font"
					: key.startsWith("legacy-mapping:")
						? "legacy-font-mapping"
						: "file",
				requestIndex,
				entry:
					entry.provenance === undefined
						? entry
						: { ...entry, provenance: entry.provenance.identity },
			});
			hints.push(...(entry.dependencies ?? []));
		}
		for (const dependency of hints) {
			if (seen.has(dependency.key)) continue;
			seen.add(dependency.key);
			jobs.push({
				manifestKey: dependency.key,
				requirement: "hint",
				kind: "file",
				requestIndex: null,
				entry: dependency,
			});
		}
		return { jobs, misses };
	},
	catalogSelectFormat(rootText, name) {
		const entry = JSON.parse(rootText).formats?.[name];
		if (entry === undefined) throw new Error(`missing format ${name}`);
		return { name, ...entry };
	},
};
const digest = (bytes) => deterministicAhash64Hex(bytes);
const jsonBytes = (value) => encoder.encode(`${JSON.stringify(value)}\n`);

function fileEntry(path, bytes) {
	const ahash64 = digest(bytes);
	return {
		virtualPath: `/texlive/${path}`,
		object: `ahash64-v1-${ahash64}`,
		ahash64,
		bytes: bytes.byteLength,
	};
}

function formatEntry(bytes) {
	const ahash64 = digest(bytes);
	return {
		object: `ahash64-v1-${ahash64}`,
		ahash64,
		bytes: bytes.byteLength,
		engine: "umber",
		engineVersion: "0.1.0",
		formatSchema: 11,
		sourceDistribution: "fixture",
		sourceManifestAhash64: "1".repeat(16),
		sourceDateEpoch: 0,
	};
}

async function fixture() {
	const payloads = {
		plain: encoder.encode("plain"),
		cmr: encoder.encode("cmr"),
		alias: encoder.encode("plain"),
		hint: encoder.encode("hint"),
		format: new Uint8Array([0, 1, 0, 2]),
	};
	const cmr = fileEntry("fonts/cmr10.tfm", payloads.cmr);
	const hint = fileEntry("tex/hint.tex", payloads.hint);
	const files = {
		"tex:plain.tex": {
			...fileEntry("tex/plain.tex", payloads.plain),
			dependencies: [
				{ key: "tex:hint.tex", ...hint },
				{ key: "tfm:cmr10.tfm", ...cmr },
			],
		},
		"tex:alias.tex": fileEntry("tex/alias.tex", payloads.alias),
		"tfm:cmr10.tfm": cmr,
		"tex:hint.tex": hint,
	};
	const shardBits = 2;
	const shardFiles = Array.from({ length: 4 }, () => ({}));
	for (const [key, entry] of Object.entries(files)) {
		shardFiles[await shardIndex(key, shardBits, webcrypto)][key] = entry;
	}
	const objectBytes = new Map();
	for (const entry of Object.values(files))
		objectBytes.set(
			entry.object,
			payloads[
				entry === cmr
					? "cmr"
					: entry === hint
						? "hint"
						: entry.object === files["tex:plain.tex"].object
							? "plain"
							: "alias"
			],
		);
	const shards = shardFiles.map((shardFilesAtIndex, index) => {
		const bytes = jsonBytes({
			schema: 3,
			distribution: "texlive-fixture",
			index,
			files: shardFilesAtIndex,
		});
		const ahash64 = digest(bytes);
		objectBytes.set(`ahash64-v1-${ahash64}`, bytes);
		return ahash64;
	});
	const format = formatEntry(payloads.format);
	objectBytes.set(format.object, payloads.format);
	const root = {
		schema: 8,
		distribution: "texlive-fixture",
		objectsBaseUrl: "https://cdn.example.test/objects/",
		shardBits,
		shardCount: 4,
		shards,
		formats: { plain: format },
	};
	const rootBytes = jsonBytes(root);
	return {
		root,
		rootBytes,
		rootDigest: digest(rootBytes),
		objectBytes,
		files,
		payloads,
	};
}

async function htmlFontFixture() {
	const template = readFileSync(
		new URL(
			"../../../tests/corpus/distribution/cross-frontend-v1/html-font-shard.template.json",
			import.meta.url,
		),
		"utf8",
	);
	const fontBytes = encoder.encode("fixture woff2 payload");
	const fontDigest = digest(fontBytes);
	const shard = JSON.parse(
		template.replace(
			'"__UNICODE_MAP__"',
			[JSON.stringify("A"), ...Array(255).fill("null")].join(","),
		),
	);
	for (const entry of [
		...Object.values(shard.fonts),
		...Object.values(shard.legacyMappings),
	]) {
		entry.object = `ahash64-v1-${fontDigest}`;
		entry.ahash64 = fontDigest;
		entry.bytes = fontBytes.byteLength;
	}
	const shardBytes = jsonBytes(shard);
	const shardDigest = digest(shardBytes);
	return {
		root: {
			schema: 9,
			distribution: "html-font-fixture",
			objectsBaseUrl: "https://cdn.example.test/objects/",
			shardBits: 0,
			shardCount: 1,
			shards: [shardDigest],
			formats: {},
		},
		objectBytes: new Map([
			[`ahash64-v1-${shardDigest}`, shardBytes],
			[`ahash64-v1-${fontDigest}`, fontBytes],
		]),
		fontRequest: Object.values(shard.fonts)[0],
		fontKey: Object.keys(shard.fonts)[0],
		mappingRequest: Object.values(shard.legacyMappings)[0],
		mappingKey: Object.keys(shard.legacyMappings)[0],
		fontBytes,
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

function resolverFor(data, options = {}) {
	const calls = options.calls ?? [];
	const fetch =
		options.fetch ??
		(async (url, requestOptions) => {
			calls.push({ url, options: requestOptions });
			const bytes = data.objectBytes.get(url.split("/").at(-1));
			return bytes === undefined
				? response(new Uint8Array(), { status: 404 })
				: response(bytes);
		});
	return {
		resolver: new HttpManifestResolver(data.root, {
			fetch,
			crypto: webcrypto,
			catalog,
			...options,
		}),
		calls,
	};
}

test("create verifies the pinned root before accepting its selection metadata", async () => {
	const data = await fixture();
	const resolver = await HttpManifestResolver.create({
		manifestUrl: "https://cdn.example.test/manifest-v2.json",
		manifestAHash64: data.rootDigest,
		fetch: async () => response(data.rootBytes),
		crypto: webcrypto,
		catalog,
	});
	assert.equal(resolver.manifest.schema, 8);
	await assert.rejects(
		HttpManifestResolver.create({
			manifestUrl: "https://cdn.example.test/manifest-v2.json",
			manifestAHash64: "0".repeat(16),
			fetch: async () => response(data.rootBytes),
			crypto: webcrypto,
			catalog,
		}),
		(error) => error.code === "manifest-digest",
	);
});

test("pinned root and objects support zero-network warm and offline resolvers", async () => {
	const data = await fixture();
	const cacheStore = new MemoryObjectCache();
	let networkRequests = 0;
	const fetch = async (url) => {
		networkRequests += 1;
		if (url.endsWith("manifest-v2.json")) return response(data.rootBytes);
		const bytes = data.objectBytes.get(url.split("/").at(-1));
		return bytes === undefined
			? response(new Uint8Array(), { status: 404 })
			: response(bytes);
	};
	const options = {
		manifestUrl: "https://cdn.example.test/manifest-v2.json",
		manifestAHash64: data.rootDigest,
		persistentCache: "indexeddb",
		cacheStore,
		fetch,
		crypto: webcrypto,
		catalog,
	};
	const cold = await HttpManifestResolver.create(options);
	const coldDownloads = await cold.resolve([
		{ kind: "tex", name: "plain.tex" },
	]);
	assert(coldDownloads.length > 0);
	const coldRequests = networkRequests;

	const warm = await HttpManifestResolver.create(options);
	await warm.resolve([{ kind: "tex", name: "plain.tex" }]);
	assert.equal(networkRequests, coldRequests);

	const offline = await HttpManifestResolver.create({
		...options,
		offline: true,
	});
	await offline.resolve([{ kind: "tex", name: "plain.tex" }]);
	assert.equal(networkRequests, coldRequests);
	await assert.rejects(
		offline.resolveFormat("plain"),
		(error) => error.code === "object-offline",
	);
	assert.equal(networkRequests, coldRequests);
});

test("fetches canonical shards, deduplicates payloads, and uses inline hints without dependency index reads", async () => {
	const data = await fixture();
	const calls = [];
	const { resolver } = resolverFor(data, { calls, concurrency: 3 });
	const downloads = await resolver.resolve([
		{ kind: "tex", name: "plain.tex" },
		{ kind: "tex", name: "alias.tex" },
		{ kind: "tex", name: "plain.tex" },
	]);
	assert.deepEqual(
		downloads.map(({ name }) => name),
		["plain.tex", "alias.tex"],
	);
	const plainShard = await shardIndex(
		"tex:plain.tex",
		data.root.shardBits,
		webcrypto,
	);
	const aliasShard = await shardIndex(
		"tex:alias.tex",
		data.root.shardBits,
		webcrypto,
	);
	const requestedObjects = calls.map(({ url }) => url.split("/").at(-1));
	assert(
		requestedObjects.includes(`ahash64-v1-${data.root.shards[plainShard]}`),
	);
	assert(
		requestedObjects.includes(`ahash64-v1-${data.root.shards[aliasShard]}`),
	);
	const dependencyShard = await shardIndex(
		"tfm:cmr10.tfm",
		data.root.shardBits,
		webcrypto,
	);
	if (dependencyShard !== plainShard && dependencyShard !== aliasShard) {
		assert(
			!requestedObjects.includes(
				`ahash64-v1-${data.root.shards[dependencyShard]}`,
			),
		);
	}
	assert.equal(
		requestedObjects.filter(
			(object) => object === data.files["tex:plain.tex"].object,
		).length,
		1,
	);
});

test("typed virtual-font requests resolve through tex shards without losing identity", async () => {
	const data = await fixture();
	const { resolver } = resolverFor(data);
	const downloads = await resolver.resolve([
		{ type: "file", domain: "tex", kind: "vf", name: "plain.tex" },
	]);
	assert.deepEqual(
		downloads.map(({ type, domain, kind, name }) => ({
			type,
			domain,
			kind,
			name,
		})),
		[{ type: "file", domain: "tex", kind: "vf", name: "plain.tex" }],
	);
});

test("HTML profile resolves exact font and mapping records while preserving authoritative absence", async () => {
	const data = await htmlFontFixture();
	const { resolver } = resolverFor(data);
	const parsedFont = data.fontKey.split(":");
	const fontRequest = {
		type: "font",
		logicalName: "cmu-serif-roman",
		faceIndex: 0,
		variationInstance: "default",
		variations: [],
		features: [
			{ tag: "kern", value: 1 },
			{ tag: "liga", value: 1 },
		],
		direction: "ltr",
		script: "latn",
		language: "en",
	};
	assert.equal(parsedFont[0], "font");
	const mappingRequest = {
		type: "legacy-font-mapping",
		tfmAhash64: "c".repeat(16),
		layoutPolicyVersion: 1,
		purpose: "html-layout",
		encodingCatalog: "OT1",
	};
	const resolved = await resolver.resolve([fontRequest, mappingRequest]);
	assert.deepEqual(
		resolved.map(({ type }) => type),
		["font", "legacy-font-mapping"],
	);
	assert.deepEqual(resolved[0].bytes, data.fontBytes);
	assert.equal(resolved[0].provenance, data.fontRequest.provenance.identity);
	assert.equal(resolved[1].unicodeMap.length, 256);
	assert.equal(resolved[1].provenance, data.mappingRequest.provenance.identity);

	const absent = { ...fontRequest, logicalName: "missing" };
	assert.deepEqual(await resolver.resolve([absent]), [
		{ ...absent, type: "font-unavailable" },
	]);
	const broken = resolverFor(data, {
		fetch: async () => response(new Uint8Array(), { status: 503 }),
	}).resolver;
	await assert.rejects(
		broken.resolve([fontRequest]),
		(error) => error.code === "object-http",
	);
});

test("verified shard absence is typed unavailable while shard transport failure is actionable", async () => {
	const data = await fixture();
	const calls = [];
	const { resolver } = resolverFor(data, { calls });
	assert.deepEqual(
		await resolver.resolve([{ kind: "tex", name: "absent.cfg" }]),
		[{ type: "file-unavailable", kind: "tex", name: "absent.cfg" }],
	);
	assert.equal(
		calls.length,
		1,
		"absence should fetch only its canonical shard",
	);
	const failing = resolverFor(data, {
		fetch: async () => response(new Uint8Array(), { status: 503 }),
	}).resolver;
	await assert.rejects(
		failing.resolve([{ kind: "tex", name: "plain.tex" }]),
		(error) => {
			assert.equal(error.code, "object-http");
			assert.match(error.message, /cannot resolve tex:plain\.tex/);
			return true;
		},
	);
});

test("rejects tampered and mispartitioned shards", async () => {
	const data = await fixture();
	const plainIndex = await shardIndex(
		"tex:plain.tex",
		data.root.shardBits,
		webcrypto,
	);
	const shardObject = `ahash64-v1-${data.root.shards[plainIndex]}`;
	const tampered = new Map(data.objectBytes);
	const changed = tampered.get(shardObject).slice();
	changed[0] ^= 1;
	tampered.set(shardObject, changed);
	await assert.rejects(
		resolverFor({ ...data, objectBytes: tampered }).resolver.resolve([
			{ kind: "tex", name: "plain.tex" },
		]),
		(error) => error.code === "object-digest",
	);

	const wrongKey = Object.keys(data.files).find(
		(key) => shardIndex(key, data.root.shardBits) !== plainIndex,
	);
	assert.notEqual(wrongKey, undefined);
	const wrongIndex = shardIndex(wrongKey, data.root.shardBits);
	const wrongShard = JSON.parse(
		new TextDecoder().decode(
			data.objectBytes.get(`ahash64-v1-${data.root.shards[wrongIndex]}`),
		),
	);
	wrongShard.files["tex:plain.tex"] = data.files["tex:plain.tex"];
	const wrongBytes = jsonBytes(wrongShard);
	const wrongDigest = digest(wrongBytes);
	const wrongRoot = { ...data.root, shards: [...data.root.shards] };
	wrongRoot.shards[wrongIndex] = wrongDigest;
	const wrongObjects = new Map(data.objectBytes).set(
		`ahash64-v1-${wrongDigest}`,
		wrongBytes,
	);
	await assert.rejects(
		resolverFor({
			...data,
			root: wrongRoot,
			objectBytes: wrongObjects,
		}).resolver.resolve([
			{ kind: "tex", name: "plain.tex" },
			{ kind: "tex", name: wrongKey.slice(4) },
		]),
		/canonical shard/,
	);
});

test("immutable shards and payloads persist across resolver instances", async () => {
	const data = await fixture();
	const cacheStore = new MemoryObjectCache();
	let fetches = 0;
	const fetch = async (url) => {
		fetches += 1;
		return response(data.objectBytes.get(url.split("/").at(-1)));
	};
	const options = {
		fetch,
		crypto: webcrypto,
		catalog,
		persistentCache: "indexeddb",
		cacheStore,
	};
	await new HttpManifestResolver(data.root, options).resolve([
		{ kind: "tex", name: "plain.tex" },
	]);
	const coldFetches = fetches;
	await new HttpManifestResolver(data.root, options).resolve([
		{ kind: "tex", name: "plain.tex" },
	]);
	assert.equal(fetches, coldFetches);
});

test("formats remain inline and download through the verified object cache", async () => {
	const data = await fixture();
	const { resolver, calls } = resolverFor(data);
	assert.deepEqual(
		await resolver.resolveFormat("plain", {
			engineVersion: "0.1.0",
			formatSchema: 11,
		}),
		data.payloads.format,
	);
	await resolver.resolveFormat("plain");
	assert.equal(calls.length, 1);
	await assert.rejects(
		resolver.resolveFormat("plain", { formatSchema: 12 }),
		(error) => error.code === "incompatible-format",
	);
});

test("schema three format closures return validated positive responses", async () => {
	const data = await fixture();
	data.root = {
		...data.root,
		schema: 8,
		formats: {
			plain: {
				...data.root.formats.plain,
				inputClosure: {
					schema: 1,
					keys: ["tex:hint.tex", "tex:plain.tex"],
				},
			},
		},
	};
	const { resolver, calls } = resolverFor(data);
	const hints = resolver.formatPrefetchHints("plain");
	assert.deepEqual(
		hints.map(({ type, domain, kind, name }) => ({ type, domain, kind, name })),
		[
			{ type: "file", domain: "tex", kind: "tex", name: "hint.tex" },
			{ type: "file", domain: "tex", kind: "tex", name: "plain.tex" },
		],
	);
	const downloads = await resolver.resolve(
		[{ type: "file", kind: "tex", name: "alias.tex" }],
		{
			prefetchHints: [
				...hints,
				{ type: "file", kind: "tex", name: "absent.cfg" },
			],
		},
	);
	assert.deepEqual(
		downloads.map(({ name }) => name),
		["alias.tex", "hint.tex", "plain.tex"],
	);
	assert(
		calls.some(({ url }) => url.endsWith(data.files["tex:hint.tex"].object)),
	);
	assert(
		calls.some(({ url }) => url.endsWith(data.files["tex:plain.tex"].object)),
	);
});

test("format closure responses fit the speculative resource budget", async () => {
	const data = await fixture();
	const { resolver } = resolverFor(data, { maxFiles: 1 });
	const downloads = await resolver.resolve(
		[{ type: "file", kind: "tex", name: "absent.cfg" }],
		{
			prefetchHints: [
				{ type: "file", kind: "tex", name: "hint.tex" },
				{ type: "file", kind: "tex", name: "plain.tex" },
			],
		},
	);
	assert.deepEqual(
		downloads.map(({ type, name }) => ({ type, name })),
		[
			{ type: "file-unavailable", name: "absent.cfg" },
			{ type: "file", name: "hint.tex" },
		],
	);
});

test("resolves blocking probes positively or with authoritative absence", async () => {
	const data = await fixture();
	const { resolver } = resolverFor(data);
	const downloads = await resolver.resolve([], {
		probes: [
			{ type: "file", kind: "tex", name: "plain.tex" },
			{ type: "file", kind: "tex", name: "absent.cfg" },
		],
	});
	assert.deepEqual(
		downloads.map(({ type, name }) => ({ type, name })),
		[
			{ type: "file-unavailable", name: "absent.cfg" },
			{ type: "file", name: "plain.tex" },
		],
	);
});

test("prefetches dependency closures without returning dependency responses", async () => {
	const data = await fixture();
	const requestedObject = data.files["tex:plain.tex"].object;
	const dependencyObjects = new Set([
		data.files["tfm:cmr10.tfm"].object,
		data.files["tex:hint.tex"].object,
	]);
	const calls = [];
	const { resolver } = resolverFor(data, {
		async fetch(url) {
			const object = url.split("/").at(-1);
			calls.push(object);
			const bytes = data.objectBytes.get(object);
			return bytes === undefined
				? response(new Uint8Array(), { status: 404 })
				: response(bytes);
		},
	});

	const downloads = await resolver.resolve([
		{ kind: "tex", name: "plain.tex" },
	]);

	assert.deepEqual(
		downloads.map(({ type, domain, kind, name }) => ({
			type,
			domain,
			kind,
			name,
		})),
		[{ type: "file", domain: "tex", kind: "tex", name: "plain.tex" }],
	);
	assert(calls.includes(requestedObject));
	assert.deepEqual(
		new Set(calls.filter((object) => dependencyObjects.has(object))),
		dependencyObjects,
	);
});

test("cancellation and oversized streamed objects remain bounded", async () => {
	const data = await fixture();
	const controller = new AbortController();
	controller.abort(new DOMException("stop", "AbortError"));
	await assert.rejects(
		resolverFor(data).resolver.resolve(
			[{ kind: "tex", name: "plain.tex" }],
			controller.signal,
		),
		{ name: "AbortError" },
	);

	const plainIndex = await shardIndex(
		"tex:plain.tex",
		data.root.shardBits,
		webcrypto,
	);
	const shardObject = `ahash64-v1-${data.root.shards[plainIndex]}`;
	let cancelled = false;
	const { resolver } = resolverFor(data, {
		fetch: async (url) => {
			if (!url.endsWith(shardObject))
				return response(data.objectBytes.get(url.split("/").at(-1)));
			return new Response(
				new ReadableStream({
					pull(stream) {
						stream.enqueue(new Uint8Array(1024 * 1024));
					},
					cancel() {
						cancelled = true;
					},
				}),
			);
		},
	});
	await assert.rejects(
		resolver.resolve([{ kind: "tex", name: "plain.tex" }]),
		(error) => error.code === "shard-length",
	);
	assert(cancelled);
});

test("resource budgets include inline dependency payloads before fetching them", async () => {
	const data = await fixture();
	let fetches = 0;
	const { resolver } = resolverFor(data, {
		maxFiles: 2,
		fetch: async (url) => {
			fetches += 1;
			return response(data.objectBytes.get(url.split("/").at(-1)));
		},
	});
	await assert.rejects(
		resolver.resolve([{ kind: "tex", name: "plain.tex" }]),
		(error) => error.code === "resource-limit",
	);
	assert.equal(
		fetches,
		1,
		"only the selection shard may precede budget validation",
	);
});

test("invalid root pin and malformed shard options are typed", async () => {
	const data = await fixture();
	await assert.rejects(
		HttpManifestResolver.create({
			manifestUrl: "unused",
			manifestAHash64: "bad",
			fetch: async () => response(data.rootBytes),
			crypto: webcrypto,
			catalog,
		}),
		(error) =>
			error instanceof ManifestResolverError &&
			error.code === "invalid-options",
	);
	assert.throws(
		() =>
			new HttpManifestResolver({ ...data.root, shardBits: 17 }, { catalog }),
		/shardBits/,
	);
});
