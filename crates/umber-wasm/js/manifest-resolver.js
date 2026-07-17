import {
	decodeKey,
	encodeRequest,
	fontRequestIdentity,
	isFormatName,
	ManifestResolverError,
	resourceDomain,
	shardIndex,
	validateIndexShard,
	validateRootManifest,
} from "./manifest-schema.js";
import { IndexedDbObjectCache } from "./persistent-cache.js";

export { ManifestResolverError } from "./manifest-schema.js";

const DIGEST_PATTERN = /^[0-9a-f]{64}$/;
const MAX_CONCURRENCY = 32;
const DEFAULT_CONCURRENCY = 8;
const MAX_ROOT_BYTES = 1024 * 1024;
const MAX_SHARD_BYTES = 64 * 1024 * 1024;
const DEFAULT_RESOLVED_FILES = 512;
const MAX_RESOLVED_FILES = 4096;
const DEFAULT_CACHED_BYTES = 64 * 1024 * 1024;
const MAX_CACHED_BYTES = 256 * 1024 * 1024;

export const TEXLIVE_2026_MANIFEST_URL =
	"https://assets.umber.ink/texlive/texlive-2026-r79639/manifest-v2.json";
export const TEXLIVE_2026_MANIFEST_SHA256 =
	"7c2784bca891844d37465083b93466b78429c7282d7ba915f40a08d150651fd0";

export class HttpManifestResolver {
	static async create(options) {
		const fetchImplementation = options.fetch ?? platformFetch();
		const crypto = options.crypto ?? globalThis.crypto;
		if (typeof fetchImplementation !== "function" || !crypto?.subtle) {
			throw new ManifestResolverError(
				"invalid-options",
				"fetch and Web Crypto SubtleCrypto are required",
			);
		}
		if (!DIGEST_PATTERN.test(options.manifestSha256)) {
			throw new ManifestResolverError(
				"invalid-options",
				"manifestSha256 must be a lowercase SHA-256 digest",
			);
		}
		const persistentMode = options.persistentCache ?? "http";
		const persistentStore =
			options.cacheStore ??
			(persistentMode === "indexeddb"
				? new IndexedDbObjectCache({ indexedDB: options.indexedDB })
				: undefined);
		const manifestIdentity = `manifest:${options.manifestUrl}`;
		let bytes;
		try {
			bytes = await persistentStore?.get(
				manifestIdentity,
				options.manifestSha256,
			);
		} catch {}
		if (bytes === undefined) {
			if (options.offline) {
				throw new ManifestResolverError(
					"manifest-offline",
					"pinned root manifest is unavailable in the persistent cache",
				);
			}
			const response = await fetchImplementation(options.manifestUrl, {
				cache: cacheMode(persistentMode),
				signal: options.signal,
			});
			if (!response.ok) {
				throw new ManifestResolverError(
					"manifest-http",
					`manifest request failed with HTTP ${response.status}`,
				);
			}
			bytes = await boundedResponseBytes(response, {
				code: "manifest-length",
				label: "root manifest",
				limit: MAX_ROOT_BYTES,
			});
		}
		const actual = await digestBytes(crypto, bytes);
		if (actual !== options.manifestSha256) {
			try {
				await persistentStore?.delete(manifestIdentity, options.manifestSha256);
			} catch {}
			throw new ManifestResolverError(
				"manifest-digest",
				`root manifest digest ${actual} does not match pinned ${options.manifestSha256}`,
			);
		}
		try {
			await persistentStore?.put(
				manifestIdentity,
				options.manifestSha256,
				bytes,
			);
		} catch {}
		let manifest;
		try {
			manifest = JSON.parse(new TextDecoder().decode(bytes));
		} catch (error) {
			throw new ManifestResolverError(
				"invalid-manifest",
				"root manifest is not valid JSON",
				{ cause: error },
			);
		}
		return new HttpManifestResolver(manifest, {
			fetch: fetchImplementation,
			crypto,
			concurrency: options.concurrency,
			persistentCache: options.persistentCache,
			cacheStore: persistentStore,
			indexedDB: options.indexedDB,
			offline: options.offline,
			maxFiles: options.maxFiles,
			maxBytes: options.maxBytes,
		});
	}

	constructor(manifest, options = {}) {
		this.manifest = validateRootManifest(manifest);
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
		this.offline = options.offline ?? false;
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
		this.shardCache = new Map();
	}

	async resolve(requests, options) {
		const signal = Object.hasOwn(options ?? {}, "signal")
			? options.signal
			: options;
		const prefetchHints = Object.hasOwn(options ?? {}, "prefetchHints")
			? options.prefetchHints
			: [];
		if (!Array.isArray(prefetchHints)) {
			throw new ManifestResolverError(
				"invalid-options",
				"prefetchHints must be an array",
			);
		}
		throwIfAborted(signal);
		const required = await this.#select(requests, signal, true);
		let hinted = { jobs: [], misses: [] };
		try {
			hinted = await this.#select(prefetchHints, signal, false);
		} catch {
			throwIfAborted(signal);
			// Speculative index transport is best effort, like speculative objects.
		}
		const unavailable = required.misses.map(({ type, request }) => ({
			...request,
			type: `${type}-unavailable`,
		}));
		const jobs = mergeJobs(required.jobs, hinted.jobs);
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
							type: "file",
							...(() => {
								const decoded = decodeKey(job.key);
								return { domain: resourceDomain(decoded.kind), ...decoded };
							})(),
							virtualPath: job.entry.virtualPath,
							bytes,
						});
					}
				} catch (error) {
					const requested = group.find((job) => job.blocking);
					if (requested !== undefined)
						throw actionableError(requested.key, error);
				}
			}
		};
		await Promise.all(
			Array.from({ length: Math.min(this.concurrency, groups.length) }, () =>
				worker(),
			),
		);
		throwIfAborted(signal);
		return unavailable.concat(
			jobs.flatMap((job) =>
				job.requested && results.has(job.key) ? [results.get(job.key)] : [],
			),
		);
	}

	async #select(requests, signal, blocking) {
		const selections = [];
		const seen = new Set();
		for (const request of requests) {
			if (request?.type === "font") {
				const identity = fontRequestIdentity(request);
				if (!seen.has(identity)) {
					seen.add(identity);
					selections.push(
						Promise.resolve({ type: "font", request, missing: true }),
					);
				}
				continue;
			}
			const key = encodeRequest(request);
			if (seen.has(key)) continue;
			seen.add(key);
			selections.push(
				(async () => {
					try {
						const index = await shardIndex(
							key,
							this.manifest.shardBits,
							this.crypto,
						);
						const shard = await this.#shard(index, signal);
						return { type: "file", request, key, entry: shard.files[key] };
					} catch (error) {
						if (blocking) throw actionableError(key, error);
						throw error;
					}
				})(),
			);
		}
		const resolved = await Promise.all(selections);
		const jobs = [];
		const misses = [];
		const hintedKeys = new Set();
		for (const item of resolved) {
			if (item.missing || item.entry === undefined) {
				misses.push({
					type: item.type,
					request: item.request,
					manifestKey: item.key ?? item.request.logicalName,
				});
				continue;
			}
			jobs.push({
				key: item.key,
				manifestKey: item.key,
				entry: item.entry,
				request: item.request,
				requested: true,
				type: "file",
			});
			for (const dependency of item.entry.dependencies) {
				if (seen.has(dependency.key) || hintedKeys.has(dependency.key))
					continue;
				hintedKeys.add(dependency.key);
				jobs.push({
					key: dependency.key,
					manifestKey: dependency.key,
					entry: dependency,
					requested: false,
					type: "file",
				});
			}
		}
		return { jobs, misses };
	}

	async #shard(index, signal) {
		let pending = this.shardCache.get(index);
		if (pending === undefined) {
			pending = (async () => {
				const sha256 = this.manifest.shards[index];
				const bytes = await this.#object(
					{ object: `sha256-${sha256}`, sha256 },
					signal,
					{ limit: MAX_SHARD_BYTES, code: "shard-length" },
				);
				let parsed;
				try {
					parsed = JSON.parse(new TextDecoder().decode(bytes));
				} catch (error) {
					throw new ManifestResolverError(
						"invalid-manifest",
						`index shard ${index} is not valid JSON`,
						{ cause: error },
					);
				}
				const shard = validateIndexShard(parsed, this.manifest, index);
				for (const key of Object.keys(shard.files)) {
					if (
						(await shardIndex(key, this.manifest.shardBits, this.crypto)) !==
						index
					) {
						throw new ManifestResolverError(
							"invalid-manifest",
							`lookup key ${key} is not in canonical shard ${index}`,
						);
					}
				}
				return shard;
			})();
			this.shardCache.set(index, pending);
			pending.catch(() => {
				if (this.shardCache.get(index) === pending)
					this.shardCache.delete(index);
			});
		}
		return pending;
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
		if (!isFormatName(name))
			throw new ManifestResolverError(
				"invalid-format",
				`invalid format name ${String(name)}`,
			);
		const entry = this.manifest.formats[name];
		if (entry === undefined)
			throw new ManifestResolverError(
				"missing-format",
				`manifest has no format named ${name}`,
			);
		return entry;
	}

	#object(entry, signal, limits = {}) {
		let pending = this.objectCache.get(entry.sha256);
		if (pending === undefined) {
			pending = this.#download(entry, signal, limits);
			this.objectCache.set(entry.sha256, pending);
			pending.catch(() => {
				if (this.objectCache.get(entry.sha256) === pending)
					this.objectCache.delete(entry.sha256);
			});
		}
		return pending;
	}

	async #download(entry, signal, limits) {
		throwIfAborted(signal);
		const cached = await this.#cached(entry, limits);
		if (cached !== undefined) return cached;
		if (this.offline) {
			throw new ManifestResolverError(
				"object-offline",
				`${entry.object} is unavailable in the persistent cache`,
			);
		}
		const response = await this.fetch(
			new URL(entry.object, this.manifest.objectsBaseUrl).href,
			{ cache: this.fetchCache, signal },
		);
		if (!response.ok)
			throw new ManifestResolverError(
				"object-http",
				`${entry.object} request failed with HTTP ${response.status}`,
			);
		const limit = entry.bytes ?? limits.limit;
		const bytes = await boundedResponseBytes(response, {
			code: limits.code ?? "object-length",
			label: entry.object,
			limit,
			exact: entry.bytes,
		});
		await this.#verify(entry, bytes, limits);
		try {
			await this.persistentStore?.put(
				this.manifest.distribution,
				entry.sha256,
				bytes,
			);
		} catch {}
		return bytes;
	}

	async #cached(entry, limits) {
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
			await this.#verify(entry, bytes, limits);
			return bytes;
		} catch {
			try {
				await this.persistentStore.delete(
					this.manifest.distribution,
					entry.sha256,
				);
			} catch {}
			return undefined;
		}
	}

	async #verify(entry, bytes, limits) {
		if (!(bytes instanceof Uint8Array))
			throw new ManifestResolverError(
				"object-cache",
				`${entry.object} cache value is not bytes`,
			);
		const limit = entry.bytes ?? limits.limit;
		if (
			bytes.byteLength > limit ||
			(entry.bytes !== undefined && bytes.byteLength !== entry.bytes)
		) {
			throw new ManifestResolverError(
				limits.code ?? "object-length",
				`${entry.object} returned ${bytes.byteLength} bytes; expected ${entry.bytes ?? `at most ${limit}`}`,
			);
		}
		const digest = await digestBytes(this.crypto, bytes);
		if (digest !== entry.sha256)
			throw new ManifestResolverError(
				"object-digest",
				`${entry.object} digest ${digest} does not match ${entry.sha256}`,
			);
	}
}

function mergeJobs(required, hinted) {
	const jobs = [];
	const indexes = new Map();
	for (const [source, blocking] of [
		[required, true],
		[hinted, false],
	]) {
		for (const job of source) {
			const existing = indexes.get(job.key);
			const jobBlocks = blocking && job.requested;
			if (existing !== undefined) {
				jobs[existing].blocking ||= jobBlocks;
				continue;
			}
			indexes.set(job.key, jobs.length);
			jobs.push({ ...job, blocking: jobBlocks });
		}
	}
	return jobs;
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
	if (jobs.length > maxFiles)
		throw new ManifestResolverError(
			"resource-limit",
			`resolution requires ${jobs.length} files; limit is ${maxFiles}`,
		);
	const paths = new Set();
	let bytes = 0;
	for (const job of jobs) {
		if (paths.has(job.entry.virtualPath)) continue;
		paths.add(job.entry.virtualPath);
		bytes += job.entry.bytes;
		if (bytes > maxBytes)
			throw new ManifestResolverError(
				"resource-limit",
				`resolution requires ${bytes} cached bytes; limit is ${maxBytes}`,
			);
	}
}

function validateConcurrency(value) {
	if (!Number.isInteger(value) || value < 1 || value > MAX_CONCURRENCY)
		throw new ManifestResolverError(
			"invalid-options",
			`concurrency must be an integer from 1 through ${MAX_CONCURRENCY}`,
		);
	return value;
}

function validateResourceLimit(value, hard, name) {
	if (!Number.isSafeInteger(value) || value < 0 || value > hard)
		throw new ManifestResolverError(
			"invalid-options",
			`${name} must be an integer from 0 through ${hard}`,
		);
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
	if (typeof response.body?.getReader !== "function")
		throw new ManifestResolverError(
			"unsupported-response",
			`${options.label} response body is not a readable byte stream`,
		);
	const reader = response.body.getReader();
	const chunks = [];
	let total = 0;
	try {
		while (true) {
			const { done, value } = await reader.read();
			if (done) break;
			if (!(value instanceof Uint8Array))
				throw new ManifestResolverError(
					"unsupported-response",
					`${options.label} response yielded a non-byte chunk`,
				);
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
	if (error instanceof ManifestResolverError)
		return new ManifestResolverError(
			error.code,
			`cannot resolve ${key}: ${error.message}`,
			{ cause: error },
		);
	return new ManifestResolverError(
		"object-fetch",
		`cannot resolve ${key}: ${error}`,
		{ cause: error },
	);
}

function throwIfAborted(signal) {
	if (signal?.aborted)
		throw (
			signal.reason ??
			new DOMException("The operation was aborted", "AbortError")
		);
}

async function digestBytes(crypto, bytes) {
	return Array.from(
		new Uint8Array(await crypto.subtle.digest("SHA-256", bytes)),
		(byte) => byte.toString(16).padStart(2, "0"),
	).join("");
}
