import { IndexedDbObjectCache } from "./persistent-cache.js";

const MAX_CONCURRENCY = 32;
const DEFAULT_CONCURRENCY = 8;
const KEY_PATTERN = /^(tex|tfm):(.+)$/;
const DIGEST_PATTERN = /^[0-9a-f]{64}$/;
const FORMAT_NAME_PATTERN = /^[A-Za-z0-9._-]+$/;
const MAX_MANIFEST_BYTES = 64 * 1024 * 1024;
const MAX_OBJECT_BYTES = 64 * 1024 * 1024;
const DEFAULT_RESOLVED_FILES = 512;
const MAX_RESOLVED_FILES = 4096;
const DEFAULT_CACHED_BYTES = 64 * 1024 * 1024;
const MAX_CACHED_BYTES = 256 * 1024 * 1024;

export class ManifestResolverError extends Error {
	constructor(code, message, options) {
		super(message, options);
		this.name = "ManifestResolverError";
		this.code = code;
	}
}

export class HttpManifestResolver {
	static async create(options) {
		const fetchImplementation = options.fetch ?? platformFetch();
		if (typeof fetchImplementation !== "function") {
			throw new ManifestResolverError(
				"invalid-options",
				"fetch is unavailable",
			);
		}
		const response = await fetchImplementation(options.manifestUrl, {
			cache: cacheMode(options.persistentCache ?? "http"),
			signal: options.signal,
		});
		if (!response.ok) {
			throw new ManifestResolverError(
				"manifest-http",
				`manifest request failed with HTTP ${response.status}`,
			);
		}
		let manifest;
		try {
			const bytes = await boundedResponseBytes(response, {
				code: "manifest-length",
				label: "manifest",
				limit: MAX_MANIFEST_BYTES,
			});
			manifest = JSON.parse(new TextDecoder().decode(bytes));
		} catch (error) {
			if (error instanceof ManifestResolverError) throw error;
			throw new ManifestResolverError(
				"invalid-manifest",
				"manifest is not valid JSON",
				{
					cause: error,
				},
			);
		}
		return new HttpManifestResolver(manifest, {
			fetch: fetchImplementation,
			crypto: options.crypto ?? globalThis.crypto,
			concurrency: options.concurrency,
			persistentCache: options.persistentCache,
			cacheStore: options.cacheStore,
			indexedDB: options.indexedDB,
			maxFiles: options.maxFiles,
			maxBytes: options.maxBytes,
		});
	}

	constructor(manifest, options = {}) {
		this.manifest = validateManifest(manifest);
		this.fetch = options.fetch ?? platformFetch();
		this.crypto = options.crypto ?? globalThis.crypto;
		this.concurrency = validateConcurrency(
			options.concurrency ?? DEFAULT_CONCURRENCY,
		);
		this.maxFiles = validateResourceLimit(
			options.maxFiles ?? DEFAULT_RESOLVED_FILES,
			MAX_RESOLVED_FILES,
			"maxFiles",
		);
		this.maxBytes = validateResourceLimit(
			options.maxBytes ?? DEFAULT_CACHED_BYTES,
			MAX_CACHED_BYTES,
			"maxBytes",
		);
		const persistentMode = options.persistentCache ?? "http";
		this.fetchCache = cacheMode(persistentMode);
		this.persistentStore =
			options.cacheStore ??
			(persistentMode === "indexeddb"
				? new IndexedDbObjectCache({ indexedDB: options.indexedDB })
				: undefined);
		if (typeof this.fetch !== "function" || !this.crypto?.subtle) {
			throw new ManifestResolverError(
				"invalid-options",
				"fetch and Web Crypto SubtleCrypto are required",
			);
		}
		this.objectCache = new Map();
	}

	async resolve(requests, signal) {
		throwIfAborted(signal);
		const jobs = collectJobs(this.manifest, requests);
		validateJobBudget(jobs, this.maxFiles, this.maxBytes);
		const groups = groupByObject(jobs);
		const results = new Map();
		let next = 0;
		const worker = async () => {
			while (next < groups.length) {
				const group = groups[next++];
				try {
					const bytes = await this.#object(group[0].entry, signal);
					for (const job of group) {
						results.set(job.key, {
							request: decodeKey(job.key),
							virtualPath: job.entry.virtualPath,
							bytes,
						});
					}
				} catch (error) {
					const requested = group.find((job) => job.requested);
					if (requested !== undefined) {
						throw actionableError(requested.key, error);
					}
				}
			}
		};
		const workers = Array.from(
			{ length: Math.min(this.concurrency, groups.length) },
			() => worker(),
		);
		await Promise.all(workers);
		throwIfAborted(signal);
		return jobs.flatMap((job) =>
			results.has(job.key) ? [results.get(job.key)] : [],
		);
	}

	async resolveFormat(name, compatibility = {}, signal) {
		throwIfAborted(signal);
		const entry = this.formatMetadata(name);
		if (
			compatibility.engineVersion !== undefined &&
			compatibility.engineVersion !== entry.engineVersion
		) {
			throw new ManifestResolverError(
				"incompatible-format",
				`format ${name} requires Umber ${entry.engineVersion}; this runtime is ${compatibility.engineVersion}`,
			);
		}
		if (
			compatibility.formatSchema !== undefined &&
			compatibility.formatSchema !== entry.formatSchema
		) {
			throw new ManifestResolverError(
				"incompatible-format",
				`format ${name} uses schema ${entry.formatSchema}; this runtime requires schema ${compatibility.formatSchema}`,
			);
		}
		try {
			return await this.#object(entry, signal);
		} catch (error) {
			throw actionableError(`format:${name}`, error);
		}
	}

	formatMetadata(name) {
		if (typeof name !== "string" || !FORMAT_NAME_PATTERN.test(name)) {
			throw new ManifestResolverError(
				"invalid-format",
				`invalid format name ${String(name)}`,
			);
		}
		const entry = this.manifest.formats[name];
		if (entry === undefined) {
			throw new ManifestResolverError(
				"missing-format",
				`manifest has no format named ${name}`,
			);
		}
		return entry;
	}

	#object(entry, signal) {
		let pending = this.objectCache.get(entry.sha256);
		if (pending === undefined) {
			pending = this.#download(entry, signal);
			this.objectCache.set(entry.sha256, pending);
			pending.catch(() => {
				if (this.objectCache.get(entry.sha256) === pending) {
					this.objectCache.delete(entry.sha256);
				}
			});
		}
		return pending;
	}

	async #download(entry, signal) {
		throwIfAborted(signal);
		const cached = await this.#cached(entry);
		if (cached !== undefined) return cached;
		const url = new URL(entry.object, this.manifest.objectsBaseUrl).href;
		const response = await this.fetch(url, { cache: this.fetchCache, signal });
		if (!response.ok) {
			throw new ManifestResolverError(
				"object-http",
				`${entry.object} request failed with HTTP ${response.status}`,
			);
		}
		const bytes = await boundedResponseBytes(response, {
			code: "object-length",
			label: entry.object,
			limit: entry.bytes,
			exact: entry.bytes,
		});
		await this.#verify(entry, bytes);
		try {
			await this.persistentStore?.put(
				this.manifest.distribution,
				entry.sha256,
				bytes,
			);
		} catch {
			// Persistent caching is an optimization and must not invalidate verified bytes.
		}
		return bytes;
	}

	async #cached(entry) {
		if (this.persistentStore === undefined) return undefined;
		let bytes;
		try {
			bytes = await this.persistentStore.get(
				this.manifest.distribution,
				entry.sha256,
			);
		} catch {
			return undefined;
		}
		if (bytes === undefined) return undefined;
		try {
			await this.#verify(entry, bytes);
			return bytes;
		} catch {
			try {
				await this.persistentStore.delete(
					this.manifest.distribution,
					entry.sha256,
				);
			} catch {
				// A corrupt cache entry remains a miss even if eviction fails.
			}
			return undefined;
		}
	}

	async #verify(entry, bytes) {
		if (!(bytes instanceof Uint8Array)) {
			throw new ManifestResolverError(
				"object-cache",
				`${entry.object} cache value is not bytes`,
			);
		}
		if (bytes.byteLength !== entry.bytes) {
			throw new ManifestResolverError(
				"object-length",
				`${entry.object} returned ${bytes.byteLength} bytes; expected ${entry.bytes}`,
			);
		}
		const digest = hex(await this.crypto.subtle.digest("SHA-256", bytes));
		if (digest !== entry.sha256) {
			throw new ManifestResolverError(
				"object-digest",
				`${entry.object} digest ${digest} does not match ${entry.sha256}`,
			);
		}
	}
}

function validateManifest(value) {
	if (!isRecord(value) || value.schema !== 1 || !isRecord(value.files)) {
		throw new ManifestResolverError(
			"invalid-manifest",
			"manifest schema 1 is required",
		);
	}
	if (
		typeof value.distribution !== "string" ||
		value.distribution.length === 0
	) {
		throw new ManifestResolverError(
			"invalid-manifest",
			"distribution is required",
		);
	}
	let objectsBaseUrl;
	try {
		objectsBaseUrl = new URL(value.objectsBaseUrl).href;
	} catch (error) {
		throw new ManifestResolverError(
			"invalid-manifest",
			"objectsBaseUrl is invalid",
			{
				cause: error,
			},
		);
	}
	if (!objectsBaseUrl.endsWith("/")) {
		throw new ManifestResolverError(
			"invalid-manifest",
			"objectsBaseUrl must end with '/'",
		);
	}
	const files = Object.create(null);
	const formats = Object.create(null);
	const hashLengths = new Map();
	const pathObjects = new Map();
	for (const [key, entry] of Object.entries(value.files)) {
		validateKey(key);
		if (!isRecord(entry) || !DIGEST_PATTERN.test(entry.sha256)) {
			throw new ManifestResolverError(
				"invalid-manifest",
				`invalid entry for ${key}`,
			);
		}
		if (entry.object !== `sha256-${entry.sha256}`) {
			throw new ManifestResolverError(
				"invalid-manifest",
				`invalid object name for ${key}`,
			);
		}
		if (
			!Number.isSafeInteger(entry.bytes) ||
			entry.bytes < 0 ||
			entry.bytes > MAX_OBJECT_BYTES
		) {
			throw new ManifestResolverError(
				"invalid-manifest",
				`invalid byte length for ${key}`,
			);
		}
		if (!isCanonicalPath(entry.virtualPath, "/texlive/")) {
			throw new ManifestResolverError(
				"invalid-manifest",
				`invalid virtual path for ${key}`,
			);
		}
		const dependencies = entry.dependencies ?? [];
		if (!Array.isArray(dependencies)) {
			throw new ManifestResolverError(
				"invalid-manifest",
				`invalid dependencies for ${key}`,
			);
		}
		for (const dependency of dependencies) validateKey(dependency);
		const previousLength = hashLengths.get(entry.sha256);
		if (previousLength !== undefined && previousLength !== entry.bytes) {
			throw new ManifestResolverError(
				"invalid-manifest",
				`inconsistent byte lengths for digest ${entry.sha256}`,
			);
		}
		hashLengths.set(entry.sha256, entry.bytes);
		const previousObject = pathObjects.get(entry.virtualPath);
		if (previousObject !== undefined && previousObject !== entry.sha256) {
			throw new ManifestResolverError(
				"invalid-manifest",
				`virtual path ${entry.virtualPath} has conflicting objects`,
			);
		}
		pathObjects.set(entry.virtualPath, entry.sha256);
		files[key] = Object.freeze({
			...entry,
			dependencies: Object.freeze([...dependencies]),
		});
	}
	const manifestFormats = value.formats ?? {};
	if (!isRecord(manifestFormats)) {
		throw new ManifestResolverError(
			"invalid-manifest",
			"formats must be an object",
		);
	}
	for (const [name, entry] of Object.entries(manifestFormats)) {
		if (!FORMAT_NAME_PATTERN.test(name) || !isRecord(entry)) {
			throw new ManifestResolverError(
				"invalid-manifest",
				`invalid format entry for ${name}`,
			);
		}
		validateObjectEntry(entry, `format ${name}`, hashLengths);
		if (
			entry.engine !== "umber" ||
			typeof entry.engineVersion !== "string" ||
			entry.engineVersion.length === 0 ||
			!Number.isSafeInteger(entry.formatSchema) ||
			entry.formatSchema < 1 ||
			typeof entry.sourceDistribution !== "string" ||
			entry.sourceDistribution.length === 0 ||
			!DIGEST_PATTERN.test(entry.sourceManifestSha256) ||
			!Number.isSafeInteger(entry.sourceDateEpoch) ||
			entry.sourceDateEpoch < 0
		) {
			throw new ManifestResolverError(
				"invalid-manifest",
				`invalid compatibility metadata for format ${name}`,
			);
		}
		formats[name] = Object.freeze({ ...entry });
	}
	for (const [key, entry] of Object.entries(files)) {
		for (const dependency of entry.dependencies) {
			if (files[dependency] === undefined) {
				throw new ManifestResolverError(
					"invalid-manifest",
					`dependency ${dependency} from ${key} is absent`,
				);
			}
		}
	}
	return Object.freeze({
		schema: 1,
		distribution: value.distribution,
		objectsBaseUrl,
		files: Object.freeze(files),
		formats: Object.freeze(formats),
	});
}

function validateObjectEntry(entry, label, hashLengths) {
	if (!DIGEST_PATTERN.test(entry.sha256)) {
		throw new ManifestResolverError(
			"invalid-manifest",
			`invalid digest for ${label}`,
		);
	}
	if (entry.object !== `sha256-${entry.sha256}`) {
		throw new ManifestResolverError(
			"invalid-manifest",
			`invalid object name for ${label}`,
		);
	}
	if (
		!Number.isSafeInteger(entry.bytes) ||
		entry.bytes < 0 ||
		entry.bytes > MAX_OBJECT_BYTES
	) {
		throw new ManifestResolverError(
			"invalid-manifest",
			`invalid byte length for ${label}`,
		);
	}
	const previousLength = hashLengths.get(entry.sha256);
	if (previousLength !== undefined && previousLength !== entry.bytes) {
		throw new ManifestResolverError(
			"invalid-manifest",
			`inconsistent byte lengths for digest ${entry.sha256}`,
		);
	}
	hashLengths.set(entry.sha256, entry.bytes);
}

function collectJobs(manifest, requests) {
	const requested = [];
	const seen = new Set();
	for (const request of requests) {
		const key = encodeRequest(request);
		if (seen.has(key)) continue;
		const entry = manifest.files[key];
		if (entry === undefined) {
			throw new ManifestResolverError(
				"missing-key",
				`manifest has no entry for ${key}`,
			);
		}
		seen.add(key);
		requested.push({ key, entry, requested: true });
	}
	const hints = [];
	for (let cursor = 0; cursor < requested.length + hints.length; cursor += 1) {
		const parent =
			cursor < requested.length
				? requested[cursor]
				: hints[cursor - requested.length];
		for (const key of parent.entry.dependencies) {
			if (seen.has(key)) continue;
			seen.add(key);
			hints.push({ key, entry: manifest.files[key], requested: false });
		}
	}
	return [...requested, ...hints];
}

function groupByObject(jobs) {
	const groups = [];
	const indexes = new Map();
	for (const job of jobs) {
		let index = indexes.get(job.entry.sha256);
		if (index === undefined) {
			index = groups.length;
			indexes.set(job.entry.sha256, index);
			groups.push([]);
		}
		groups[index].push(job);
	}
	return groups;
}

function validateJobBudget(jobs, maxFiles, maxBytes) {
	if (jobs.length > maxFiles) {
		throw new ManifestResolverError(
			"resource-limit",
			`resolution requires ${jobs.length} files; limit is ${maxFiles}`,
		);
	}
	const paths = new Map();
	let bytes = 0;
	for (const job of jobs) {
		if (paths.has(job.entry.virtualPath)) continue;
		paths.set(job.entry.virtualPath, true);
		bytes += job.entry.bytes;
		if (bytes > maxBytes) {
			throw new ManifestResolverError(
				"resource-limit",
				`resolution requires ${bytes} cached bytes; limit is ${maxBytes}`,
			);
		}
	}
}

function encodeRequest(request) {
	if (
		!isRecord(request) ||
		(request.kind !== "tex" && request.kind !== "tfm")
	) {
		throw new ManifestResolverError(
			"invalid-request",
			"request kind must be tex or tfm",
		);
	}
	const key = `${request.kind}:${request.name}`;
	validateKey(key);
	return key;
}

function decodeKey(key) {
	const match = KEY_PATTERN.exec(key);
	return { kind: match[1], name: match[2] };
}

function validateKey(key) {
	if (typeof key !== "string") {
		throw new ManifestResolverError(
			"invalid-manifest",
			`invalid lookup key ${String(key)}`,
		);
	}
	const match = KEY_PATTERN.exec(key);
	if (match === null || !isCanonicalPath(match[2], "")) {
		throw new ManifestResolverError(
			"invalid-manifest",
			`invalid lookup key ${key}`,
		);
	}
}

function validateConcurrency(value) {
	if (!Number.isInteger(value) || value < 1 || value > MAX_CONCURRENCY) {
		throw new ManifestResolverError(
			"invalid-options",
			`concurrency must be an integer from 1 through ${MAX_CONCURRENCY}`,
		);
	}
	return value;
}

function validateResourceLimit(value, hard, name) {
	if (!Number.isSafeInteger(value) || value < 0 || value > hard) {
		throw new ManifestResolverError(
			"invalid-options",
			`${name} must be an integer from 0 through ${hard}`,
		);
	}
	return value;
}

function cacheMode(value) {
	if (value === "http") return "force-cache";
	if (value === "none" || value === "indexeddb") return "no-store";
	throw new ManifestResolverError(
		"invalid-options",
		"persistentCache must be 'http', 'indexeddb', or 'none'",
	);
}

async function boundedResponseBytes(response, options) {
	const declared = response.headers?.get?.("content-length");
	if (declared !== null && declared !== undefined) {
		const parsed = Number(declared);
		if (
			!Number.isSafeInteger(parsed) ||
			parsed < 0 ||
			parsed > options.limit ||
			(options.exact !== undefined && parsed !== options.exact)
		) {
			throw responseLengthError(options, `Content-Length ${declared}`);
		}
	}
	if (response.body === null) return new Uint8Array();
	if (typeof response.body?.getReader !== "function") {
		throw new ManifestResolverError(
			"unsupported-response",
			`${options.label} response body is not a readable byte stream`,
		);
	}

	const reader = response.body.getReader();
	const chunks = [];
	let total = 0;
	try {
		while (true) {
			const { done, value } = await reader.read();
			if (done) break;
			if (!(value instanceof Uint8Array)) {
				throw new ManifestResolverError(
					"unsupported-response",
					`${options.label} response yielded a non-byte chunk`,
				);
			}
			if (value.byteLength > options.limit - total) {
				await reader.cancel().catch(() => {});
				throw responseLengthError(
					options,
					`at least ${total + value.byteLength} streamed bytes`,
				);
			}
			total += value.byteLength;
			if (value.byteLength > 0) chunks.push(value);
		}
	} finally {
		reader.releaseLock();
	}

	const bytes = new Uint8Array(total);
	let offset = 0;
	for (const chunk of chunks) {
		bytes.set(chunk, offset);
		offset += chunk.byteLength;
	}
	return bytes;
}

function responseLengthError(options, actual) {
	const expected =
		options.exact === undefined
			? `the ${options.limit} byte ceiling`
			: `${options.exact} bytes`;
	return new ManifestResolverError(
		options.code,
		`${options.label} returned ${actual}; expected ${expected}`,
	);
}

function platformFetch() {
	return typeof globalThis.fetch === "function"
		? globalThis.fetch.bind(globalThis)
		: undefined;
}

function actionableError(key, error) {
	if (error instanceof ManifestResolverError) {
		return new ManifestResolverError(
			error.code,
			`cannot resolve ${key}: ${error.message}`,
			{
				cause: error,
			},
		);
	}
	return new ManifestResolverError(
		"object-fetch",
		`cannot resolve ${key}: ${error}`,
		{
			cause: error,
		},
	);
}

function throwIfAborted(signal) {
	if (signal?.aborted) {
		throw (
			signal.reason ??
			new DOMException("The operation was aborted", "AbortError")
		);
	}
}

function hex(buffer) {
	return Array.from(new Uint8Array(buffer), (byte) =>
		byte.toString(16).padStart(2, "0"),
	).join("");
}

function isRecord(value) {
	return value !== null && typeof value === "object" && !Array.isArray(value);
}

function isCanonicalPath(value, prefix) {
	if (typeof value !== "string" || !value.startsWith(prefix)) return false;
	const suffix = value.slice(prefix.length);
	if (
		suffix.length === 0 ||
		suffix.includes("\\") ||
		suffix.includes("\0") ||
		suffix.includes(":")
	) {
		return false;
	}
	return suffix
		.split("/")
		.every(
			(component) =>
				component !== "" && component !== "." && component !== "..",
		);
}
