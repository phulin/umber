import {
	decodeKey,
	encodeRequest,
	fontRequestIdentity,
	legacyMappingRequestIdentity,
	ManifestResolverError,
	resourceDomain,
} from "./manifest-schema.js";
import { IndexedDbObjectCache } from "./persistent-cache.js";

export { ManifestResolverError } from "./manifest-schema.js";

const DIGEST_PATTERN = /^[0-9a-f]{16}$/;
const MAX_CONCURRENCY = 32;
const DEFAULT_CONCURRENCY = 8;
const MAX_ROOT_BYTES = 1024 * 1024;
const MAX_SHARD_BYTES = 64 * 1024 * 1024;
const DEFAULT_RESOLVED_FILES = 512;
const MAX_RESOLVED_FILES = 4096;
const DEFAULT_CACHED_BYTES = 64 * 1024 * 1024;
const MAX_CACHED_BYTES = 256 * 1024 * 1024;

// Installed by the external publication tracked in umber2-66p0.27.
export const TEXLIVE_2026_MANIFEST_URL = undefined;
export const TEXLIVE_2026_MANIFEST_AHASH64 = undefined;

export class HttpManifestResolver {
	static async create(options) {
		if (
			options.manifestUrl === undefined &&
			options.manifestAHash64 === undefined
		)
			throw new ManifestResolverError(
				"default-distribution-unpublished",
				"the default deterministic aHash64 distribution has not been published; provide a migrated manifestUrl and manifestAHash64",
			);
		const fetchImplementation = options.fetch ?? platformFetch();
		if (typeof fetchImplementation !== "function") {
			throw new ManifestResolverError("invalid-options", "fetch is required");
		}
		if (!DIGEST_PATTERN.test(options.manifestAHash64)) {
			throw new ManifestResolverError(
				"invalid-options",
				"manifestAHash64 must be a lowercase aHash64 digest",
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
				options.manifestAHash64,
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
		const actual = deterministicAhash64Hex(bytes);
		if (actual !== options.manifestAHash64) {
			try {
				await persistentStore?.delete(
					manifestIdentity,
					options.manifestAHash64,
				);
			} catch {}
			throw new ManifestResolverError(
				"manifest-digest",
				`root manifest digest ${actual} does not match pinned ${options.manifestAHash64}`,
			);
		}
		try {
			await persistentStore?.put(
				manifestIdentity,
				options.manifestAHash64,
				bytes,
			);
		} catch {}
		let rootText;
		try {
			rootText = new TextDecoder("utf-8", { fatal: true }).decode(bytes);
		} catch (error) {
			throw new ManifestResolverError(
				"invalid-manifest",
				"root manifest is not UTF-8",
				{ cause: error },
			);
		}
		return new HttpManifestResolver(rootText, {
			fetch: fetchImplementation,
			concurrency: options.concurrency,
			persistentCache: options.persistentCache,
			cacheStore: persistentStore,
			indexedDB: options.indexedDB,
			offline: options.offline,
			maxFiles: options.maxFiles,
			maxBytes: options.maxBytes,
			catalog: options.catalog,
		});
	}

	constructor(manifest, options = {}) {
		this.catalog = options.catalog;
		if (typeof this.catalog?.catalogCreateSession !== "function") {
			throw new ManifestResolverError(
				"invalid-options",
				"the umber-wasm catalog bindings are required",
			);
		}
		try {
			const rootText =
				typeof manifest === "string"
					? manifest
					: `${JSON.stringify(manifest)}\n`;
			this.catalogSession = this.catalog.catalogCreateSession(rootText);
			const prepared = this.catalogSession.prepareBatch([]);
			this.rootCanonical = prepared.root;
			this.manifest = JSON.parse(this.rootCanonical);
			this.manifest.formats ??= {};
		} catch (error) {
			throw new ManifestResolverError(
				"invalid-manifest",
				`root manifest failed canonical catalog validation: ${error?.message ?? error}`,
				{ cause: error },
			);
		}
		this.fetch = options.fetch ?? platformFetch();
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
		if (typeof this.fetch !== "function") {
			throw new ManifestResolverError("invalid-options", "fetch is required");
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
		const probes = Object.hasOwn(options ?? {}, "probes") ? options.probes : [];
		if (!Array.isArray(probes)) {
			throw new ManifestResolverError(
				"invalid-options",
				"probes must be an array",
			);
		}
		if (!Array.isArray(prefetchHints)) {
			throw new ManifestResolverError(
				"invalid-options",
				"prefetchHints must be an array",
			);
		}
		throwIfAborted(signal);
		const required = await this.#select(requests.concat(probes), signal, true);
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
		validateJobBudget(required.jobs, this.maxFiles, this.maxBytes);
		const jobs = mergeJobs(
			required.jobs,
			hinted.jobs,
			this.maxFiles,
			this.maxBytes,
		);
		const groups = groupByObject(jobs);
		const results = new Map();
		let next = 0;
		const worker = async () => {
			while (next < groups.length) {
				const group = groups[next++];
				try {
					const bytes = await this.#object(group[0].entry, signal);
					for (const job of group) {
						results.set(
							job.key,
							job.type === "file"
								? {
										type: "file",
										...(() => {
											const identity = job.request ?? decodeKey(job.key);
											return {
												domain:
													identity.domain ?? resourceDomain(identity.kind),
												kind: identity.kind,
												name: identity.name,
											};
										})(),
										virtualPath: job.entry.virtualPath,
										bytes,
									}
								: job.type === "font"
									? {
											...job.request,
											type: "font",
											container: job.entry.container,
											bytes,
											objectAHash64: job.entry.ahash64,
											...(job.entry.programIdentity === undefined
												? {}
												: { programIdentity: job.entry.programIdentity }),
											provenance: job.entry.provenance,
										}
									: {
											...job.request,
											type: "legacy-font-mapping",
											fontKey: job.entry.fontKey,
											container: job.entry.container,
											bytes,
											objectAHash64: job.entry.ahash64,
											...(job.entry.programIdentity === undefined
												? {}
												: { programIdentity: job.entry.programIdentity }),
											unicodeMap: job.entry.unicodeMap,
											fallback: job.entry.fallback,
											provenance: job.entry.provenance,
										},
						);
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
		const descriptors = requests.map((request) => ({
			request,
			type:
				request?.type === "font"
					? "font"
					: request?.type === "legacy-font-mapping"
						? "legacy-font-mapping"
						: "file",
			key:
				request?.type === "font"
					? fontRequestIdentity(request)
					: request?.type === "legacy-font-mapping"
						? legacyMappingRequestIdentity(request)
						: encodeRequest(request),
		}));
		try {
			const keys = descriptors.map(({ key }) => key);
			const prepared = this.catalogSession.prepareBatch(keys);
			await Promise.all(
				prepared.shards.map(async (shard) => {
					this.catalogSession.provideShard(
						shard.index,
						await this.#shard(shard, signal),
					);
				}),
			);
			const plan = this.catalogSession.planBatch(keys);
			return {
				jobs: plan.jobs.map((job) => ({
					key: job.manifestKey,
					manifestKey: job.manifestKey,
					entry: job.entry,
					request:
						job.requestIndex === null
							? undefined
							: descriptors[job.requestIndex].request,
					requested: job.requirement === "required",
					type: job.kind,
				})),
				misses: plan.misses.map((index) => ({
					type: descriptors[index].type,
					request: descriptors[index].request,
					manifestKey: descriptors[index].key,
				})),
			};
		} catch (error) {
			if (blocking)
				throw actionableError(descriptors[0]?.key ?? "catalog batch", error);
			throw error;
		}
	}

	async #shard(descriptor, signal) {
		let pending = this.shardCache.get(descriptor.index);
		if (pending === undefined) {
			pending = this.#object(descriptor, signal, {
				limit: MAX_SHARD_BYTES,
				code: "shard-length",
			});
			this.shardCache.set(descriptor.index, pending);
			pending.catch(() => {
				if (this.shardCache.get(descriptor.index) === pending)
					this.shardCache.delete(descriptor.index);
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
		try {
			return this.catalogSession.selectFormat(name);
		} catch (error) {
			throw new ManifestResolverError(
				"invalid-format",
				`invalid or missing format ${String(name)}`,
				{ cause: error },
			);
		}
	}

	formatPrefetchHints(name) {
		const closure = this.formatMetadata(name).inputClosure;
		return (
			closure?.keys.map((key) => {
				const decoded = decodeKey(key);
				return {
					type: "file",
					domain: resourceDomain(decoded.kind),
					...decoded,
					originalName: decoded.name,
				};
			}) ?? []
		);
	}

	#object(entry, signal, limits = {}) {
		let pending = this.objectCache.get(entry.ahash64);
		if (pending === undefined) {
			pending = this.#download(entry, signal, limits);
			this.objectCache.set(entry.ahash64, pending);
			pending.catch(() => {
				if (this.objectCache.get(entry.ahash64) === pending)
					this.objectCache.delete(entry.ahash64);
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
				entry.ahash64,
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
				entry.ahash64,
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
					entry.ahash64,
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
		const digest = deterministicAhash64Hex(bytes);
		if (digest !== entry.ahash64)
			throw new ManifestResolverError(
				"object-digest",
				`${entry.object} digest ${digest} does not match ${entry.ahash64}`,
			);
	}
}

function mergeJobs(required, hinted, maxFiles, maxBytes) {
	const jobs = [];
	const indexes = new Map();
	const paths = new Set();
	let bytes = 0;
	for (const [source, blocking] of [
		[required, true],
		[hinted, false],
	]) {
		for (const job of source) {
			const existing = indexes.get(job.key);
			const requested = job.requested;
			if (existing !== undefined) {
				jobs[existing].blocking ||= blocking && requested;
				jobs[existing].requested ||= requested;
				continue;
			}
			const pathBytes = paths.has(job.entry.virtualPath) ? 0 : job.entry.bytes;
			if (
				!blocking &&
				(jobs.length >= maxFiles || bytes + pathBytes > maxBytes)
			)
				continue;
			indexes.set(job.key, jobs.length);
			jobs.push({ ...job, requested, blocking: blocking && requested });
			paths.add(job.entry.virtualPath);
			bytes += pathBytes;
		}
	}
	return jobs;
}

function groupByObject(jobs) {
	const groups = [];
	const indexes = new Map();
	for (const job of jobs) {
		let index = indexes.get(job.entry.ahash64);
		if (index === undefined) {
			index = groups.length;
			indexes.set(job.entry.ahash64, index);
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

export function deterministicAhash64Hex(bytes, domain = 1) {
	const mask = (1n << 64n) - 1n;
	const multiple = 6364136223846793005n;
	const pad = 0x1319_8a2e_0370_7344n;
	let state = 0x243f_6a88_85a3_08d3n;
	let length = 0n;
	let tail = [];
	const rotateLeft = (value, bits) => {
		const shift = BigInt(bits) & 63n;
		return ((value << shift) | (value >> ((64n - shift) & 63n))) & mask;
	};
	const foldedMultiply = (left, right) => {
		const product = left * right;
		return ((product & mask) ^ (product >> 64n)) & mask;
	};
	const mix = (word) => {
		state = (foldedMultiply(state ^ word, multiple) + pad) & mask;
	};
	const write = (part) => {
		length += BigInt(part.length);
		for (const byte of part) {
			tail.push(byte);
			if (tail.length === 8) {
				let word = 0n;
				for (let index = 0; index < 8; index++)
					word |= BigInt(tail[index]) << BigInt(index * 8);
				mix(word);
				tail = [];
			}
		}
	};
	write(new TextEncoder().encode("umber-ahash64\0"));
	write(Uint8Array.of(1));
	const domainBytes = new Uint8Array(8);
	let domainValue = BigInt(domain);
	for (let index = 0; index < 8; index++) {
		domainBytes[index] = Number(domainValue & 0xffn);
		domainValue >>= 8n;
	}
	write(domainBytes);
	write(bytes);
	if (tail.length !== 0) {
		let word = 0n;
		for (let index = 0; index < tail.length; index++)
			word |= BigInt(tail[index]) << BigInt(index * 8);
		mix(word ^ rotateLeft(BigInt(tail.length), 48));
	}
	state = foldedMultiply(state ^ length, pad ^ rotateLeft(length, 17));
	state = rotateLeft(state, Number(state & 63n));
	return state.toString(16).padStart(16, "0");
}
