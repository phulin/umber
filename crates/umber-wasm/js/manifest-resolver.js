import {
	decodeKey,
	encodeRequest,
	fontRequestIdentity,
	legacyMappingRequestIdentity,
	ManifestResolverError,
	resourceDomain,
} from "./manifest-schema.js";
import { IndexedDbObjectCache } from "./persistent-cache.js";
import {
	extractLiteralHints,
	LookupManifest,
	literalHintRequest,
	makePrefetchIdentity,
	PREFETCH_POLICY_VERSION,
	prefetchManifestCacheKey,
	typedRequestIdentity,
} from "./prefetch.js";

export { ManifestResolverError } from "./manifest-schema.js";
export {
	classifyReadiness,
	extractLiteralHints,
	LookupManifest,
	makePrefetchIdentity,
	PREFETCH_POLICY_VERSION,
	ResourceReadiness,
} from "./prefetch.js";

const DIGEST_PATTERN = /^[0-9a-f]{16}$/;
const MAX_CONCURRENCY = 32;
const DEFAULT_CONCURRENCY = 8;
const MAX_ROOT_BYTES = 1024 * 1024;
const MAX_SHARD_BYTES = 64 * 1024 * 1024;
const DEFAULT_RESOLVED_FILES = 512;
const MAX_RESOLVED_FILES = 4096;
const DEFAULT_CACHED_BYTES = 64 * 1024 * 1024;
const MAX_CACHED_BYTES = 256 * 1024 * 1024;
const MAX_PACKAGE_SCAN_BYTES = 256 * 1024;
const MAX_PACKAGE_FOLLOWUP_HINTS = 32;

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
			prefetchPolicyVersion: options.prefetchPolicyVersion,
			rootAHash64: options.manifestAHash64,
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
		this.rootAHash64 =
			options.rootAHash64 ??
			deterministicAhash64Hex(new TextEncoder().encode(this.rootCanonical));
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
		this.prefetchPolicyVersion =
			options.prefetchPolicyVersion ?? PREFETCH_POLICY_VERSION;
		this.readiness = new Map();
		this.prefetchAdmitted = new Map();
		this.prefetchCountedPaths = new Set();
		this.prefetchUsed = new Set();
		this.demandCounted = new Set();
		this.prefetchMetrics = {
			startupPrefetchCandidates: 0,
			literalPrefetchHints: 0,
			packageGroupCandidates: 0,
			prefetchBytes: 0,
			demandBytes: 0,
			unusedPrefetchBytes: 0,
			readyResources: 0,
			existsNotReadyResources: 0,
			absentResources: 0,
		};
		this.currentRun = undefined;
	}

	async resolve(requests, options) {
		const signal = Object.hasOwn(options ?? {}, "signal")
			? options.signal
			: options;
		const prefetchHints = Object.hasOwn(options ?? {}, "prefetchHints")
			? options.prefetchHints
			: [];
		const admitPrefetch = options?.admitPrefetch === true;
		const prefetchTrace =
			options?.prefetchTrace instanceof Set ? options.prefetchTrace : new Set();
		const prefetchDepth = Number.isSafeInteger(options?.prefetchDepth)
			? options.prefetchDepth
			: 0;
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
		const requiredKeys = new Set(requests.map(typedRequestIdentity));
		const probeKeys = new Set(probes.map(typedRequestIdentity));
		const roleFor = (request, hinted = false) =>
			hinted
				? "hint"
				: requiredKeys.has(typedRequestIdentity(request))
					? "required"
					: probeKeys.has(typedRequestIdentity(request))
						? "probe"
						: "required";
		const required = await this.#select(requests.concat(probes), signal, true);
		let hinted = { jobs: [], misses: [] };
		try {
			hinted = await this.#select(prefetchHints, signal, false);
		} catch {
			throwIfAborted(signal);
			// Speculative index transport is best effort, like speculative objects.
		}
		for (const job of required.jobs.concat(hinted.jobs)) {
			const identity =
				job.request === undefined
					? typedRequestIdentity(decodeKey(job.manifestKey))
					: job.key;
			this.#setReadiness(identity, "exists-not-ready");
			if (job.request === undefined)
				this.prefetchMetrics.packageGroupCandidates += 1;
		}
		const unavailable = required.misses.map(({ type, request }) => ({
			...request,
			type: `${type}-unavailable`,
		}));
		for (const miss of required.misses) {
			this.#setReadiness(typedRequestIdentity(miss.request), "absent");
			this.#recordAbsent(miss.request, roleFor(miss.request), miss.manifestKey);
		}
		for (const miss of hinted.misses) {
			this.#setReadiness(typedRequestIdentity(miss.request), "absent");
			this.#recordAbsent(miss.request, "hint", miss.manifestKey);
		}
		validateJobBudget(required.jobs, this.maxFiles, this.maxBytes);
		const jobs = mergeJobs(
			required.jobs,
			hinted.jobs,
			this.maxFiles,
			this.maxBytes,
		);
		const groups = groupByObject(jobs);
		const results = new Map();
		const followupHints = [];
		let next = 0;
		const worker = async () => {
			while (next < groups.length) {
				const group = groups[next++];
				try {
					const bytes = await this.#object(group[0].entry, signal);
					if (
						admitPrefetch &&
						followupHints.length < MAX_PACKAGE_FOLLOWUP_HINTS
					)
						collectPackageHints(group, bytes, prefetchTrace, followupHints);
					for (const job of group) {
						const identity =
							job.request === undefined
								? typedRequestIdentity(decodeKey(job.manifestKey))
								: job.key;
						this.#setReadiness(identity, "ready");
						this.#recordResolved(
							job,
							job.request === undefined
								? "hint"
								: roleFor(job.request, job.hinted || !job.requested),
						);
						if (job.requested && !job.hinted) {
							const demandIdentity = job.key;
							if (!this.demandCounted.has(demandIdentity)) {
								this.demandCounted.add(demandIdentity);
								this.prefetchMetrics.demandBytes += bytes.byteLength;
							}
							for (const [prefetchIdentity, value] of this.prefetchAdmitted) {
								if (value.virtualPath === job.entry.virtualPath)
									this.prefetchUsed.add(prefetchIdentity);
							}
						} else if (admitPrefetch && (job.hinted || !job.requested)) {
							const prefetchIdentity = identity;
							if (!this.prefetchAdmitted.has(prefetchIdentity)) {
								this.prefetchAdmitted.set(prefetchIdentity, {
									bytes: bytes.byteLength,
									virtualPath: job.entry.virtualPath,
								});
								if (this.prefetchCountedPaths.add(job.entry.virtualPath))
									this.prefetchMetrics.prefetchBytes += bytes.byteLength;
							}
						}
						results.set(
							job.key,
							job.type === "file"
								? {
										type: "file",
										...(() => {
											const identity =
												job.request ?? decodeKey(job.manifestKey);
											return {
												domain:
													identity.domain ?? resourceDomain(identity.kind),
												kind: identity.kind,
												name: identity.name,
											};
										})(),
										virtualPath: job.entry.virtualPath,
										bytes,
										...(admitPrefetch && job.hinted
											? { speculative: true }
											: {}),
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
		const admitted = unavailable.concat(
			jobs.flatMap((job) =>
				(job.requested || admitPrefetch) && results.has(job.key)
					? [results.get(job.key)]
					: [],
			),
		);
		if (followupHints.length === 0 || prefetchDepth >= 1) return admitted;
		const followup = await this.resolve([], {
			signal,
			prefetchHints: deduplicateTypedRequests(followupHints),
			admitPrefetch: true,
			prefetchTrace,
			prefetchDepth: prefetchDepth + 1,
		});
		return admitted.concat(followup);
	}

	/** Returns the last catalog/payload state observed for a typed request. */
	readinessOf(request) {
		return this.readiness.get(typedRequestIdentity(request));
	}

	/**
	 * Starts accepted-run lookup recording and returns bounded startup hints.
	 * Identity fields deliberately contain execution policy, never source text.
	 */
	async beginRun(context = {}) {
		this.prefetchAdmitted.clear();
		this.prefetchCountedPaths.clear();
		this.prefetchUsed.clear();
		this.demandCounted.clear();
		this.readiness.clear();
		for (const key of Object.keys(this.prefetchMetrics))
			this.prefetchMetrics[key] = 0;
		const format = formatIdentity(
			context.options?.format,
			context.options?.formatSchema,
		);
		const persistable = format !== "unavailable";
		const identity = makePrefetchIdentity({
			engine: context.options?.engine ?? "tex82",
			format,
			options: stableOptionsIdentity(context.options),
			distribution: `root:${this.rootAHash64}`,
			searchPolicy: `${this.prefetchPolicyVersion};providers=project/generated/local/distribution;precedence=v1`,
		});
		let prior;
		if (persistable && this.persistentStore !== undefined) {
			try {
				const bytes = await this.persistentStore.get(
					"prefetch",
					prefetchManifestCacheKey(identity),
				);
				prior = LookupManifest.decode(bytes, identity);
			} catch {}
		}
		this.currentRun = { identity, manifest: new LookupManifest(identity) };
		const literalHints = this.literalPrefetchHints(context.source ?? "");
		const hints = [...(prior?.resolvedRequests() ?? []), ...literalHints];
		this.prefetchMetrics.startupPrefetchCandidates +=
			prior?.resolvedRequests().length ?? 0;
		this.prefetchMetrics.literalPrefetchHints += literalHints.length;
		const maxHints = Number.isSafeInteger(context.limits?.resolvedFiles)
			? Math.min(context.limits.resolvedFiles, MAX_RESOLVED_FILES)
			: DEFAULT_RESOLVED_FILES;
		return {
			identity,
			hints: deduplicateTypedRequests(hints).slice(0, maxHints),
		};
	}

	/** Publishes only the accepted run; failed discoveries are never persisted. */
	async commitRun() {
		const run = this.currentRun;
		this.currentRun = undefined;
		const usedPaths = new Set(
			[...this.prefetchAdmitted]
				.filter(([identity]) => this.prefetchUsed.has(identity))
				.map(([, value]) => value.virtualPath),
		);
		const countedPaths = new Set();
		this.prefetchMetrics.unusedPrefetchBytes = [
			...this.prefetchAdmitted,
		].reduce((total, [identity, value]) => {
			if (
				this.prefetchUsed.has(identity) ||
				usedPaths.has(value.virtualPath) ||
				!countedPaths.add(value.virtualPath)
			)
				return total;
			return total + value.bytes;
		}, 0);
		if (run === undefined || this.persistentStore === undefined) return;
		if (run.identity.format === "unavailable") return;
		try {
			await this.persistentStore.put(
				"prefetch",
				prefetchManifestCacheKey(run.identity),
				run.manifest.encode(),
			);
		} catch {}
	}

	discardRun() {
		this.currentRun = undefined;
		this.prefetchAdmitted.clear();
		this.prefetchCountedPaths.clear();
		this.prefetchUsed.clear();
		this.demandCounted.clear();
	}

	get metrics() {
		return { ...this.prefetchMetrics };
	}

	/**
	 * Supplies source-independent startup hints to a caller that owns the
	 * session options.  The engine still controls final lookup precedence.
	 */
	literalPrefetchHints(source, limits) {
		return extractLiteralHints(source, limits)
			.map(literalHintRequest)
			.filter((request) => request !== undefined);
	}

	#recordResolved(job, role) {
		const run = this.currentRun;
		const request =
			job.request ??
			(job.type === "file"
				? {
						type: "file",
						domain: resourceDomain(decodeKey(job.manifestKey).kind),
						...decodeKey(job.manifestKey),
						originalName: decodeKey(job.manifestKey).name,
					}
				: undefined);
		if (
			run === undefined ||
			request === undefined ||
			(request.type !== undefined && request.type !== "file")
		)
			return;
		run.manifest.record({
			originalSpelling: request.originalName ?? request.name,
			requestKey: job.manifestKey,
			resourceKind: request.kind,
			searchContext: request.searchContext ?? "distribution",
			role,
			outcome: {
				kind: "resolved",
				manifestKey: job.manifestKey,
				virtualPath: job.entry.virtualPath,
				object: job.entry.object,
				ahash64: job.entry.ahash64,
				bytes: job.entry.bytes,
			},
		});
	}

	#recordAbsent(request, role, manifestKey) {
		const run = this.currentRun;
		if (
			run === undefined ||
			request === undefined ||
			(request.type !== undefined && request.type !== "file")
		)
			return;
		run.manifest.record({
			originalSpelling: request.originalName ?? request.name,
			requestKey: manifestKey,
			resourceKind: request.kind,
			searchContext: request.searchContext ?? "distribution",
			role,
			outcome: {
				kind: "absent",
				scope: request.negativeScope ?? `distribution:${this.rootAHash64}`,
			},
		});
	}

	#setReadiness(identity, state) {
		const previous = this.readiness.get(identity);
		if (previous === state) return;
		for (const value of [previous, state]) {
			if (value === "ready")
				this.prefetchMetrics.readyResources += state === value ? 1 : -1;
			if (value === "exists-not-ready")
				this.prefetchMetrics.existsNotReadyResources +=
					state === value ? 1 : -1;
			if (value === "absent")
				this.prefetchMetrics.absentResources += state === value ? 1 : -1;
		}
		this.readiness.set(identity, state);
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
			catalogKey:
				request?.type === "font"
					? fontRequestIdentity(request)
					: request?.type === "legacy-font-mapping"
						? legacyMappingRequestIdentity(request)
						: encodeRequest(request),
			key: typedRequestIdentity(request),
		}));
		try {
			const keys = descriptors.map(({ catalogKey }) => catalogKey);
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
			const descriptorsByCatalogKey = new Map();
			for (const descriptor of descriptors) {
				const group = descriptorsByCatalogKey.get(descriptor.catalogKey) ?? [];
				if (!group.some((item) => item.key === descriptor.key))
					group.push(descriptor);
				descriptorsByCatalogKey.set(descriptor.catalogKey, group);
			}
			return {
				jobs: plan.jobs.flatMap((job) => {
					const matches = descriptorsByCatalogKey.get(job.manifestKey);
					if (job.requestIndex === null || matches === undefined)
						return [
							{
								key: `catalog:${job.manifestKey}`,
								manifestKey: job.manifestKey,
								entry: job.entry,
								request: undefined,
								requested: job.requirement === "required",
								hinted: !blocking,
								type: job.kind,
							},
						];
					return matches.map((descriptor) => ({
						key: descriptor.key,
						manifestKey: job.manifestKey,
						entry: job.entry,
						request: descriptor.request,
						requested: job.requirement === "required",
						hinted: !blocking,
						type: job.kind,
					}));
				}),
				misses: plan.misses.flatMap((index) =>
					(
						descriptorsByCatalogKey.get(descriptors[index].catalogKey) ?? []
					).map((descriptor) => ({
						type: descriptor.type,
						request: descriptor.request,
						manifestKey: descriptor.catalogKey,
					})),
				),
			};
		} catch (error) {
			if (blocking)
				throw actionableError(
					descriptors[0]?.catalogKey ?? "catalog batch",
					error,
				);
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

function deduplicateTypedRequests(requests) {
	const seen = new Set();
	return requests.filter((request) => {
		const identity = typedRequestIdentity(request);
		if (seen.has(identity)) return false;
		seen.add(identity);
		return true;
	});
}

function collectPackageHints(group, bytes, trace, output) {
	if (
		!(bytes instanceof Uint8Array) ||
		bytes.byteLength > MAX_PACKAGE_SCAN_BYTES
	)
		return;
	if (
		!group.some(
			(job) => job.type === "file" && isSmallRuntimeKey(job.manifestKey),
		)
	)
		return;
	if (group.some((job) => trace.has(job.key))) return;
	for (const job of group) trace.add(job.key);
	const source = new TextDecoder().decode(bytes);
	for (const hint of extractLiteralHints(source, {
		maxHints: MAX_PACKAGE_FOLLOWUP_HINTS,
		maxNameBytes: 1024,
	})) {
		const request = literalHintRequest(hint);
		if (request !== undefined && !trace.has(typedRequestIdentity(request))) {
			trace.add(typedRequestIdentity(request));
			output.push(request);
			if (output.length >= MAX_PACKAGE_FOLLOWUP_HINTS) break;
		}
	}
}

function isSmallRuntimeKey(key) {
	const name = key.split(":", 2)[1] ?? key;
	return /\.(?:tex|sty|cls|def|ltx)$/i.test(name);
}

function stableOptionsIdentity(options) {
	if (!options || typeof options !== "object") return "{}";
	const selected = {};
	for (const key of [
		"engine",
		"formatSchema",
		"profile",
		"providerPrecedence",
		"outputs",
		"fontLayoutPolicy",
		"fontMappingFallback",
		"mainPath",
		"jobName",
	]) {
		if (options[key] !== undefined) selected[key] = options[key];
	}
	return JSON.stringify(selected, Object.keys(selected).sort());
}

function formatIdentity(value, schema) {
	if (value === undefined) return "none";
	if (!Number.isSafeInteger(schema) || schema < 0) return "unavailable";
	try {
		return `content:${deterministicAhash64Hex(toUint8Array(value))}:schema=${schema}`;
	} catch {
		return "unavailable";
	}
}

function toUint8Array(value) {
	if (value instanceof Uint8Array) return value;
	if (value instanceof ArrayBuffer) return new Uint8Array(value);
	if (ArrayBuffer.isView(value))
		return new Uint8Array(value.buffer, value.byteOffset, value.byteLength);
	throw new TypeError("format identity requires byte data");
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
