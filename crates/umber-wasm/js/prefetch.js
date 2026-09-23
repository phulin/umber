import { encodeRequest, resourceDomain } from "./manifest-schema.js";

/** Shared names for the three states used by catalog/VFS adapters. */
export const ResourceReadiness = Object.freeze({
	Ready: "ready",
	ExistsNotReady: "exists-not-ready",
	Absent: "absent",
});

export const PREFETCH_POLICY_VERSION = "literal-groups-v1";

function prefetchClassForRequest(request) {
	return request?.kind === "image" ? "image" : undefined;
}

function rustRequestInput(request, required = false) {
	const className = prefetchClassForRequest(request);
	return {
		key: encodeRequest(request),
		domain: request.domain ?? resourceDomain(request.kind),
		kind: request.kind,
		name: request.name,
		originalSpelling: request.originalName ?? request.name ?? "",
		searchContext: request.searchContext ?? "literal",
		...(className === undefined ? {} : { class: className }),
		required: request.required === true || required,
		depth:
			Number.isSafeInteger(request.depth) && request.depth >= 0
				? request.depth
				: 0,
		...(typeof request.origin === "string" ? { origin: request.origin } : {}),
	};
}

function rustRequestOutput(request) {
	if (
		!request ||
		typeof request.key !== "string" ||
		typeof request.domain !== "string" ||
		typeof request.kind !== "string" ||
		typeof request.name !== "string" ||
		typeof request.origin !== "string"
	)
		throw new TypeError(
			"Rust prefetch policy returned an invalid semantic request",
		);
	return {
		type: "file",
		domain: request.domain,
		kind: request.kind,
		name: request.name,
		originalName: request.originalSpelling ?? request.name,
		searchContext: request.searchContext ?? "literal",
		depth: request.depth ?? 0,
		origin: request.origin,
	};
}

/** Binds browser transport to the policy owned by umber-distribution. */
export function createRustPrefetchPolicy(bindings) {
	if (
		typeof bindings?.prefetchLiteralHints !== "function" ||
		typeof bindings?.prefetchSelect !== "function" ||
		typeof bindings?.prefetchPolicyVersion !== "function" ||
		typeof bindings?.PrefetchPolicySession !== "function" ||
		![
			"enqueue",
			"enqueueEscalation",
			"enqueueLiteralHints",
			"select",
			"drain",
			"dependencyClosureRequest",
			"admitRequest",
			"noteReplayRequest",
		].every(
			(method) =>
				typeof bindings.PrefetchPolicySession.prototype?.[method] ===
				"function",
		)
	)
		return undefined;
	return Object.freeze({
		version: bindings.prefetchPolicyVersion(),
		literalHints(source, limits) {
			return bindings.prefetchLiteralHints(source, limits).map((hint) => ({
				...hint,
				kind:
					hint.kind === "includegraphics"
						? "includegraphics"
						: hint.kind === "documentclass"
							? "documentclass"
							: hint.kind === "package"
								? "package"
								: "input",
			}));
		},
		select(required, candidates, budget) {
			return bindings.prefetchSelect(required, candidates, budget);
		},
		createState() {
			const state = new bindings.PrefetchPolicySession();
			return Object.freeze({
				dispose() {
					state.free();
				},
				enqueue(requests) {
					state.enqueue(requests.map((request) => rustRequestInput(request)));
				},
				enqueueEscalation(requests, priority) {
					state.enqueueEscalation(
						requests.map((request) => rustRequestInput(request)),
						priority,
					);
				},
				enqueueLiteralHints(source) {
					return state.enqueueLiteralHints(source);
				},
				select(required, candidates, budget) {
					return state.select(required, candidates, budget);
				},
				drain(limit) {
					return state.drain(limit).map(rustRequestOutput);
				},
				dependencyClosure(request, tier) {
					return state
						.dependencyClosureRequest(rustRequestInput(request), tier)
						.map(rustRequestOutput);
				},
				admit(request, virtualPath, bytes, dependencies = []) {
					const encodedDependencies = dependencies.map((dependency) =>
						rustRequestInput(dependency),
					);
					state.admitRequest(
						rustRequestInput(request),
						virtualPath ?? "",
						bytes,
						request.kind === "image" ? "image" : undefined,
						encodedDependencies,
					);
				},
				noteReplay(region, request, discardedWork) {
					return state.noteReplayRequest(
						region,
						rustRequestInput(request),
						discardedWork,
					);
				},
			});
		},
	});
}

export function makePrefetchIdentity({
	engine = "tex82",
	format = "none",
	options = "{}",
	distribution,
	searchPolicy = PREFETCH_POLICY_VERSION,
} = {}) {
	const values = { engine, format, options, distribution, searchPolicy };
	for (const [name, value] of Object.entries(values)) {
		if (typeof value !== "string" || value.length === 0)
			throw new TypeError(`${name} must be a non-empty string`);
	}
	return Object.freeze({
		...values,
		canonicalKey: [
			values.engine,
			values.format,
			values.options,
			values.distribution,
			values.searchPolicy,
		]
			.map(escapeIdentity)
			.join("|"),
	});
}

/** Accepted lookup history for one source-independent execution identity. */
export class LookupManifest {
	constructor(identity, records = []) {
		this.identity = identity;
		this.records = [];
		for (const record of records) this.record(record);
	}

	record(record) {
		if (!record || typeof record !== "object") return false;
		const duplicate = this.records.find(
			(existing) =>
				existing.originalSpelling === record.originalSpelling &&
				existing.requestKey === record.requestKey &&
				existing.resourceKind === record.resourceKind &&
				existing.searchContext === record.searchContext &&
				existing.outcome?.kind === record.outcome?.kind &&
				(existing.outcome?.kind !== "absent" ||
					existing.outcome?.scope === record.outcome?.scope),
		);
		if (duplicate) {
			if (roleRank(record.role) > roleRank(duplicate.role))
				duplicate.role = record.role;
			return false;
		}
		this.records.push(structuredRecord(record));
		return true;
	}

	resolvedRequests() {
		return this.records
			.filter((record) => record.outcome?.kind === "resolved")
			.map((record) => {
				const name = record.requestKey.slice(
					record.requestKey.indexOf(":") + 1,
				);
				return {
					type: "file",
					domain: resourceDomain(record.resourceKind),
					kind: record.resourceKind,
					name,
					originalName: record.originalSpelling,
				};
			});
	}

	encode() {
		return new TextEncoder().encode(
			`${JSON.stringify({
				schema: 1,
				identity: {
					engine: this.identity.engine,
					format: this.identity.format,
					options: this.identity.options,
					distribution: this.identity.distribution,
					searchPolicy: this.identity.searchPolicy,
				},
				records: this.records,
			})}\n`,
		);
	}

	static decode(bytes, identity) {
		if (!(bytes instanceof Uint8Array)) return undefined;
		let value;
		try {
			value = JSON.parse(
				new TextDecoder("utf-8", { fatal: true }).decode(bytes),
			);
		} catch {
			return undefined;
		}
		if (
			value?.schema !== 1 ||
			JSON.stringify(value.identity) !==
				JSON.stringify({
					engine: identity.engine,
					format: identity.format,
					options: identity.options,
					distribution: identity.distribution,
					searchPolicy: identity.searchPolicy,
				}) ||
			!Array.isArray(value.records)
		)
			return undefined;
		return new LookupManifest(identity, value.records);
	}
}

export function prefetchManifestCacheKey(identity) {
	return stableHex64(identity.canonicalKey);
}

/**
 * Distinguishes admitted bytes from catalog evidence.  Undefined means that
 * no authoritative semantic answer exists yet (for example, a transport or
 * access failure); callers must not persist that state as absence.
 */
export function classifyReadiness({
	exists = undefined,
	payloadAdmitted = false,
	authoritativeAbsent = false,
} = {}) {
	if (authoritativeAbsent) return ResourceReadiness.Absent;
	if (payloadAdmitted) return ResourceReadiness.Ready;
	if (exists === true) return ResourceReadiness.ExistsNotReady;
	return undefined;
}

/** Converts a lexical hint to the distribution-facing typed request. */
export function literalHintRequest(hint) {
	if (!hint || typeof hint.name !== "string") return undefined;
	const kind = hint.kind === "includegraphics" ? "image" : "tex";
	return {
		type: "file",
		domain: resourceDomain(kind),
		kind,
		name: hint.name,
		originalName: hint.originalSpelling ?? hint.name,
	};
}

/**
 * Catalog key identity for a typed request.  The semantic kind is retained so
 * an image and a TeX input with the same spelling never alias in the VFS.
 */
export function typedRequestIdentity(request) {
	if (request?.type === "file") {
		return JSON.stringify([
			"file",
			request.domain ?? resourceDomain(request.kind),
			request.kind,
			request.name,
		]);
	}
	try {
		return `${request?.type ?? "unknown"}:${encodeRequest(request)}`;
	} catch {
		return JSON.stringify(request);
	}
}

function roleRank(role) {
	return role === "required" ? 3 : role === "probe" ? 2 : 1;
}

function structuredRecord(record) {
	return {
		originalSpelling: String(record.originalSpelling ?? ""),
		requestKey: String(record.requestKey ?? ""),
		resourceKind: String(record.resourceKind ?? "tex"),
		searchContext: String(record.searchContext ?? "distribution"),
		role:
			record.role === "required" || record.role === "probe"
				? record.role
				: "hint",
		outcome:
			record.outcome?.kind === "resolved"
				? {
						kind: "resolved",
						manifestKey: String(
							record.outcome.manifestKey ?? record.requestKey,
						),
						virtualPath: record.outcome.virtualPath,
						object: String(record.outcome.object ?? ""),
						ahash64: String(record.outcome.ahash64 ?? ""),
						bytes: Number(record.outcome.bytes ?? 0),
					}
				: { kind: "absent", scope: String(record.outcome?.scope ?? "") },
	};
}

function escapeIdentity(value) {
	return encodeURIComponent(value);
}

function stableHex64(value) {
	let hash = 0xcbf29ce484222325n;
	for (const byte of new TextEncoder().encode(value)) {
		hash ^= BigInt(byte);
		hash = BigInt.asUintN(64, hash * 0x100000001b3n);
	}
	return hash.toString(16).padStart(16, "0");
}
