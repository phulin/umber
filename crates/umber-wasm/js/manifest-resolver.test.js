import assert from "node:assert/strict";
import { createHash, webcrypto } from "node:crypto";
import test from "node:test";
import {
	HttpManifestResolver,
	ManifestResolverError,
} from "./manifest-resolver.js";
import { shardIndex } from "./manifest-schema.js";
import { MemoryObjectCache } from "./persistent-cache.js";

const encoder = new TextEncoder();
const digest = (bytes) => createHash("sha256").update(bytes).digest("hex");
const jsonBytes = (value) => encoder.encode(`${JSON.stringify(value)}\n`);

function fileEntry(path, bytes) {
	const sha256 = digest(bytes);
	return {
		virtualPath: `/texlive/${path}`,
		object: `sha256-${sha256}`,
		sha256,
		bytes: bytes.byteLength,
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
		sourceDistribution: "fixture",
		sourceManifestSha256: "1".repeat(64),
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
			schema: 1,
			distribution: "texlive-fixture",
			index,
			files: shardFilesAtIndex,
		});
		const sha256 = digest(bytes);
		objectBytes.set(`sha256-${sha256}`, bytes);
		return sha256;
	});
	const format = formatEntry(payloads.format);
	objectBytes.set(format.object, payloads.format);
	const root = {
		schema: 2,
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
			...options,
		}),
		calls,
	};
}

test("create verifies the pinned root before accepting its selection metadata", async () => {
	const data = await fixture();
	const resolver = await HttpManifestResolver.create({
		manifestUrl: "https://cdn.example.test/manifest-v2.json",
		manifestSha256: data.rootDigest,
		fetch: async () => response(data.rootBytes),
		crypto: webcrypto,
	});
	assert.equal(resolver.manifest.schema, 2);
	await assert.rejects(
		HttpManifestResolver.create({
			manifestUrl: "https://cdn.example.test/manifest-v2.json",
			manifestSha256: "0".repeat(64),
			fetch: async () => response(data.rootBytes),
			crypto: webcrypto,
		}),
		(error) => error.code === "manifest-digest",
	);
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
	assert(requestedObjects.includes(`sha256-${data.root.shards[plainShard]}`));
	assert(requestedObjects.includes(`sha256-${data.root.shards[aliasShard]}`));
	const dependencyShard = await shardIndex(
		"tfm:cmr10.tfm",
		data.root.shardBits,
		webcrypto,
	);
	if (dependencyShard !== plainShard && dependencyShard !== aliasShard) {
		assert(
			!requestedObjects.includes(`sha256-${data.root.shards[dependencyShard]}`),
		);
	}
	assert.equal(
		requestedObjects.filter(
			(object) => object === data.files["tex:plain.tex"].object,
		).length,
		1,
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
	const shardObject = `sha256-${data.root.shards[plainIndex]}`;
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

	const wrongIndex = (plainIndex + 1) % data.root.shardCount;
	const wrongShard = JSON.parse(
		new TextDecoder().decode(
			data.objectBytes.get(`sha256-${data.root.shards[wrongIndex]}`),
		),
	);
	wrongShard.files["tex:plain.tex"] = data.files["tex:plain.tex"];
	const wrongBytes = jsonBytes(wrongShard);
	const wrongDigest = digest(wrongBytes);
	const wrongRoot = { ...data.root, shards: [...data.root.shards] };
	wrongRoot.shards[wrongIndex] = wrongDigest;
	const wrongObjects = new Map(data.objectBytes).set(
		`sha256-${wrongDigest}`,
		wrongBytes,
	);
	await assert.rejects(
		resolverFor({
			...data,
			root: wrongRoot,
			objectBytes: wrongObjects,
		}).resolver.resolve([
			{ kind: "tex", name: "plain.tex" },
			{ kind: "tex", name: Object.keys(wrongShard.files)[0].slice(4) },
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
			formatSchema: 4,
		}),
		data.payloads.format,
	);
	await resolver.resolveFormat("plain");
	assert.equal(calls.length, 1);
	await assert.rejects(
		resolver.resolveFormat("plain", { formatSchema: 5 }),
		(error) => error.code === "incompatible-format",
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
	const shardObject = `sha256-${data.root.shards[plainIndex]}`;
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
			manifestSha256: "bad",
			fetch: async () => response(data.rootBytes),
			crypto: webcrypto,
		}),
		(error) =>
			error instanceof ManifestResolverError &&
			error.code === "invalid-options",
	);
	assert.throws(
		() => new HttpManifestResolver({ ...data.root, shardBits: 17 }),
		/shardBits/,
	);
});
