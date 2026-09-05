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
	createJavaScriptPrefetchPolicy,
	createRustPrefetchPolicy,
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
const SHARED_PREFETCH_BUDGET = Object.freeze({
	maxFiles: 64,
	maxBytes: 16 * 1024 * 1024,
	maxRuntimeBytes: 8 * 1024 * 1024,
	maxFontBytes: 2 * 1024 * 1024,
	maxImageBytes: 2 * 1024 * 1024,
	maxDocumentBytes: 512 * 1024,
	maxRuntimeScanBytes: 256 * 1024,
	maxFollowupHints: 32,
	maxFollowupDepth: 1,
});

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
		this.prefetchPolicy =
			options.prefetchPolicy ??
			createRustPrefetchPolicy(options.catalog) ??
			createJavaScriptPrefetchPolicy();
		this.prefetchState = undefined;
		this.prefetchQueue = [];
		this.prefetchQueued = new Set();
		this.prefetchAdmittedRequests = new Set();
		this.prefetchDepths = new Map();
		this.prefetchScanned = new Set();
		this.replayMisses = new Map();
		this.replayRegions = new Map();
		this.replayRegionRequests = new Map();
		this.lastDiscardedWork = 0;
		this.knownDependencies = new Map();
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

	/** Installs the production Rust policy after the WASM module is loaded. */
	bindPrefetchPolicy(bindings) {
		const policy = createRustPrefetchPolicy(bindings);
		if (policy !== undefined) {
			this.prefetchPolicy = policy;
			this.prefetchPolicyVersion = policy.version;
		}
		return this.prefetchPolicy.version;
	}

	async resolve(requests, options) {
		const signal = Object.hasOwn(options ?? {}, "signal")
			? options.signal
			: options;
		const prefetchHints = Object.hasOwn(options ?? {}, "prefetchHints")
			? options.prefetchHints
			: [];
		const admitPrefetch = options?.admitPrefetch === true;
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
		validateJobBudget(required.jobs, this.maxFiles, this.maxBytes);
		const jobs = this.#selectJobs(required.jobs, hinted.jobs);
		const groups = groupByObject(jobs);
		const results = new Map();
		let next = 0;
		const worker = async () => {
			while (next < groups.length) {
				const group = groups[next++];
				try {
					const bytes = await this.#object(group[0].entry, signal);
					for (const job of group) {
						this.#rememberDependencies(job);
						this.#recordResolved(
							job,
							job.request === undefined
								? "hint"
								: roleFor(job.request, job.hinted || !job.requested),
						);
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
		return admitted;
	}

	#selectJobs(required, hinted) {
		if (typeof this.prefetchPolicy.select !== "function")
			return mergeJobs(required, hinted, this.maxFiles, this.maxBytes);
		// Inline dependency jobs are returned alongside blocking jobs by the
		// catalogue plan. Their null request index makes them speculative even
		// though they came from the blocking lookup; keep them under the shared
		// optional budget instead of treating them as required payloads.
		const blocking = required.filter((job) => job.requested);
		const inlineHints = required.filter((job) => !job.requested);
		const candidates = inlineHints.concat(hinted);
		const candidate = (job, requiredFlag) => ({
			key: job.manifestKey,
			object: job.entry.object,
			ahash64: job.entry.ahash64,
			bytes: job.entry.bytes,
			required: requiredFlag,
		});
		const selection = this.prefetchPolicy.select(
			blocking.map((job) => candidate(job, true)),
			candidates.map((job) => candidate(job, false)),
			SHARED_PREFETCH_BUDGET,
		);
		const allowed = new Set(selection.hintKeys);
		return mergeSelectedJobs(
			blocking,
			candidates.filter((job) => allowed.has(job.manifestKey)),
		);
	}

	#rememberDependencies(job) {
		if (job.type !== "file" || !Array.isArray(job.entry.dependencies)) return;
		const request = job.request ?? requestFromManifestKey(job.manifestKey);
		if (request === undefined) return;
		const identity = typedRequestIdentity(request);
		const dependencies = this.knownDependencies.get(identity) ?? [];
		for (const key of job.entry.dependencies) {
			const dependency = requestFromManifestKey(key);
			if (
				dependency !== undefined &&
				!dependencies.some(
					(existing) =>
						typedRequestIdentity(existing) === typedRequestIdentity(dependency),
				)
			)
				dependencies.push(dependency);
		}
		this.knownDependencies.set(identity, dependencies);
	}

	/**
	 * Completes the policy admission callback after the Rust VFS transaction has
	 * accepted verified payloads. Cache/catalog responses alone do not reach
	 * this method and therefore cannot become engine-readable.
	 */
	noteAdmitted(responses) {
		for (const response of responses ?? []) {
			if (response?.type !== "file" || !(response.bytes instanceof Uint8Array))
				continue;
			const request = {
				type: "file",
				domain: response.domain ?? resourceDomain(response.kind),
				kind: response.kind,
				name: response.name,
				originalName: response.name,
			};
			const identity = typedRequestIdentity(request);
			if (this.prefetchState !== undefined)
				this.prefetchState.admit(
					request,
					response.virtualPath,
					response.bytes,
					this.knownDependencies.get(identity) ?? [],
				);
			this.prefetchAdmittedRequests.add(identity);
			this.#setReadiness(identity, "ready");
			if (response.speculative === true) {
				if (!this.prefetchAdmitted.has(identity)) {
					this.prefetchAdmitted.set(identity, {
						bytes: response.bytes.byteLength,
						virtualPath: response.virtualPath,
					});
					if (this.prefetchCountedPaths.add(response.virtualPath))
						this.prefetchMetrics.prefetchBytes += response.bytes.byteLength;
				}
			} else {
				if (!this.demandCounted.has(identity)) {
					this.demandCounted.add(identity);
					this.prefetchMetrics.demandBytes += response.bytes.byteLength;
				}
				for (const [prefetchIdentity, value] of this.prefetchAdmitted) {
					if (value.virtualPath === response.virtualPath)
						this.prefetchUsed.add(prefetchIdentity);
				}
			}
			if (
				this.prefetchState === undefined &&
				isSmallRuntimeKey(`${response.kind}:${response.name}`) &&
				response.bytes.byteLength <=
					SHARED_PREFETCH_BUDGET.maxRuntimeScanBytes &&
				!this.prefetchScanned.has(identity)
			) {
				this.prefetchScanned.add(identity);
				const depth = this.prefetchDepths.get(identity) ?? 0;
				if (depth < SHARED_PREFETCH_BUDGET.maxFollowupDepth) {
					const source = new TextDecoder().decode(response.bytes);
					const hints = this.prefetchPolicy.literalHints(source, {
						maxHints: SHARED_PREFETCH_BUDGET.maxFollowupHints,
						maxNameBytes: 1024,
					});
					for (const hint of hints.slice(
						0,
						SHARED_PREFETCH_BUDGET.maxFollowupHints,
					)) {
						const child = literalHintRequest(hint);
						if (child !== undefined) this.#enqueuePrefetch(child, depth + 1);
					}
				}
			}
		}
	}

	takePrefetchHints(limit = SHARED_PREFETCH_BUDGET.maxFiles) {
		if (this.prefetchState !== undefined)
			return this.prefetchState.drain(limit);
		return this.prefetchQueue.splice(0, Math.max(0, limit));
	}

	noteReplay(context, requests = []) {
		if (
			context === undefined ||
			context === null ||
			typeof context.region !== "string" ||
			!Number.isSafeInteger(context.discardedWork)
		)
			return;
		if (this.prefetchState !== undefined) {
			for (const request of [...(requests ?? [])].filter(
				(request) => request?.type === "file",
			)) {
				const advice = this.prefetchState.noteReplay(
					context.region,
					request,
					context.discardedWork,
				);
				if (advice?.tier > 0) {
					this.prefetchState.enqueueEscalation(
						this.prefetchState.dependencyClosure(request, advice.tier),
						advice.discardedWorkDelta,
					);
				}
			}
			return;
		}
		const delta = Math.max(0, context.discardedWork - this.lastDiscardedWork);
		this.lastDiscardedWork = Math.max(
			this.lastDiscardedWork,
			context.discardedWork,
		);
		const regionCount =
			(this.replayRegions.get(context.region) ?? 0) + (delta > 0 ? 1 : 0);
		if (delta > 0) this.replayRegions.set(context.region, regionCount);
		for (const request of [...(requests ?? [])].filter(
			(request) => request?.type === "file",
		)) {
			const identity = typedRequestIdentity(request);
			const regionRequests =
				this.replayRegionRequests.get(context.region) ?? new Set();
			const newRegionRequest = !regionRequests.has(identity);
			regionRequests.add(identity);
			this.replayRegionRequests.set(context.region, regionRequests);
			if (regionCount === 0 && !newRegionRequest) continue;
			const missKey = `${context.region}\u0000${identity}`;
			const miss = this.replayMisses.get(missKey) ?? { count: 0, tier: 0 };
			miss.count += 1;
			miss.tier =
				miss.tier > 0
					? Math.min(3, miss.tier + 1)
					: miss.count >= 2 || regionCount >= 2 || regionRequests.size >= 2
						? 1
						: 0;
			this.replayMisses.set(missKey, miss);
			if (miss.tier > 0) {
				for (const dependency of this.#knownDependencyClosure(
					request,
					miss.tier,
				))
					this.#enqueuePrefetch(dependency, 0);
			}
		}
	}

	#knownDependencyClosure(request, tier) {
		const output = [];
		const seen = new Set([typedRequestIdentity(request)]);
		let frontier = [request];
		for (let depth = 0; depth < Math.min(3, Math.max(1, tier)); depth += 1) {
			const next = [];
			for (const parent of frontier) {
				for (const dependency of this.knownDependencies.get(
					typedRequestIdentity(parent),
				) ?? []) {
					const identity = typedRequestIdentity(dependency);
					if (!seen.has(identity)) {
						seen.add(identity);
						output.push(dependency);
						next.push(dependency);
					}
				}
			}
			frontier = next;
		}
		return output;
	}

	#enqueuePrefetch(request, depth) {
		if (this.prefetchState !== undefined) {
			return this.prefetchState.enqueue([
				{
					...request,
					depth,
					searchContext: request.searchContext ?? "runtime",
				},
			]);
		}
		const identity = typedRequestIdentity(request);
		if (
			this.prefetchQueued.has(identity) ||
			this.prefetchAdmittedRequests.has(identity) ||
			this.prefetchQueue.length >= SHARED_PREFETCH_BUDGET.maxFiles
		)
			return false;
		this.prefetchQueued.add(identity);
		this.prefetchDepths.set(identity, depth);
		this.prefetchQueue.push(request);
		return true;
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
		this.prefetchState = undefined;
		this.prefetchAdmitted.clear();
		this.prefetchCountedPaths.clear();
		this.prefetchUsed.clear();
		this.prefetchQueue = [];
		this.prefetchQueued.clear();
		this.prefetchAdmittedRequests.clear();
		this.prefetchDepths.clear();
		this.prefetchScanned.clear();
		this.replayMisses.clear();
		this.replayRegions.clear();
		this.replayRegionRequests.clear();
		this.lastDiscardedWork = 0;
		this.knownDependencies.clear();
		this.demandCounted.clear();
		this.readiness.clear();
		for (const key of Object.keys(this.prefetchMetrics))
			this.prefetchMetrics[key] = 0;
		const format = formatIdentity(
			context.options?.format,
			context.options?.formatSchema,
		);
		const persistable = format !== "unavailable";
		const providerPrecedence =
			context.options?.providerPrecedence ??
			"project/generated/local/distribution";
		const identity = makePrefetchIdentity({
			engine: context.options?.engine ?? "tex82",
			format,
			options: stableOptionsIdentity(context.options),
			distribution: `root:${this.rootAHash64}`,
			searchPolicy: `${this.prefetchPolicyVersion};providers=${providerPrecedence};precedence=v1`,
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
		const priorRequests = prior?.resolvedRequests() ?? [];
		this.prefetchMetrics.startupPrefetchCandidates += priorRequests.length;
		const maxHints = Number.isSafeInteger(context.limits?.resolvedFiles)
			? Math.min(context.limits.resolvedFiles, MAX_RESOLVED_FILES)
			: DEFAULT_RESOLVED_FILES;
		let startupHints;
		if (typeof this.prefetchPolicy.createState === "function") {
			this.prefetchState = this.prefetchPolicy.createState();
			this.prefetchState.enqueue(priorRequests);
			this.prefetchMetrics.literalPrefetchHints +=
				this.prefetchState.enqueueLiteralHints(context.source ?? "");
			startupHints = this.prefetchState.drain(maxHints);
		} else {
			const literalHints = this.literalPrefetchHints(context.source ?? "");
			this.prefetchMetrics.literalPrefetchHints += literalHints.length;
			startupHints = deduplicateTypedRequests([
				...priorRequests,
				...literalHints,
			]).slice(0, maxHints);
		}
		for (const request of startupHints)
			this.prefetchDepths.set(typedRequestIdentity(request), 0);
		return {
			identity,
			hints: startupHints,
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
		this.prefetchState = undefined;
		this.prefetchAdmitted.clear();
		this.prefetchCountedPaths.clear();
		this.prefetchUsed.clear();
		this.prefetchQueue = [];
		this.prefetchQueued.clear();
		this.prefetchAdmittedRequests.clear();
		this.prefetchDepths.clear();
		this.prefetchScanned.clear();
		this.replayMisses.clear();
		this.replayRegions.clear();
		this.replayRegionRequests.clear();
		this.lastDiscardedWork = 0;
		this.knownDependencies.clear();
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
		return this.prefetchPolicy
			.literalHints(source, limits)
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

function mergeSelectedJobs(required, hinted) {
	const jobs = [];
	const indexes = new Map();
	for (const [source, blocking] of [
		[required, true],
		[hinted, false],
	]) {
		for (const job of source) {
			const existing = indexes.get(job.key);
			if (existing !== undefined) {
				jobs[existing].blocking ||= blocking && job.requested;
				jobs[existing].requested ||= job.requested;
				continue;
			}
			indexes.set(job.key, jobs.length);
			jobs.push({
				...job,
				requested: job.requested,
				blocking: blocking && job.requested,
			});
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

function isSmallRuntimeKey(key) {
	const name = key.split(":", 2)[1] ?? key;
	return /\.(?:tex|sty|cls|def|ltx)$/i.test(name);
}

function requestFromManifestKey(key) {
	try {
		const decoded = decodeKey(key);
		return {
			type: "file",
			domain: resourceDomain(decoded.kind),
			...decoded,
			originalName: decoded.name,
			searchContext: "distribution",
		};
	} catch {
		return undefined;
	}
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
