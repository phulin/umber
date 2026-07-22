import {
	encodeRequest,
	fontRequestIdentity,
	legacyMappingRequestIdentity,
} from "./manifest-schema.js";

/**
 * Ordered, output-neutral composition of typed resource providers.
 *
 * A provider's typed unavailable response is a miss at that provider only.
 * Transport and validation failures are deliberately not converted to misses.
 */
export class CompositeResourceResolver {
	constructor(providers) {
		if (!providers || typeof providers[Symbol.iterator] !== "function")
			throw new TypeError("providers must be an iterable");
		this.providers = [...providers];
		if (this.providers.length === 0)
			throw new TypeError("at least one resource provider is required");
		for (const provider of this.providers) {
			if (!provider || typeof provider.resolve !== "function")
				throw new TypeError("every resource provider must implement resolve");
		}
	}

	async resolve(requests, options = {}) {
		if (!Array.isArray(requests))
			throw new TypeError("requests must be an array");
		const probes = options?.probes ?? [];
		if (!Array.isArray(probes)) throw new TypeError("probes must be an array");
		const signal = options?.signal;
		throwIfAborted(signal);

		const ordered = deduplicateRequests(requests.concat(probes));
		const pending = new Map(
			ordered.map((request) => [resourceRequestIdentity(request), request]),
		);
		const accepted = new Map();

		for (const provider of this.providers) {
			if (pending.size === 0) break;
			throwIfAborted(signal);
			const providerProbeKeys = new Set(probes.map(resourceRequestIdentity));
			const providerPending = [...pending.values()];
			const providerProbes = providerPending.filter((request) =>
				providerProbeKeys.has(resourceRequestIdentity(request)),
			);
			const providerRequests = providerPending.filter(
				(request) => !providerProbeKeys.has(resourceRequestIdentity(request)),
			);
			const responses = await provider.resolve(providerRequests, {
				signal,
				probes: providerProbes,
				// A speculative response must not bind a lower-precedence provider
				// before a higher-precedence provider sees a real request.
				prefetchHints: [],
			});
			throwIfAborted(signal);
			if (!responses || typeof responses[Symbol.iterator] !== "function")
				throw new TypeError("resource provider must return an iterable");
			const seen = new Set();
			for (const response of responses) {
				const identity = resourceResponseIdentity(response);
				if (seen.has(identity))
					throw new TypeError(
						`resource provider returned duplicate response ${identity}`,
					);
				if (!pending.has(identity))
					throw new TypeError(
						`resource provider returned unexpected response ${identity}`,
					);
				seen.add(identity);
				if (isUnavailable(response)) continue;
				accepted.set(identity, response);
				pending.delete(identity);
			}
		}

		for (const [identity, request] of pending)
			accepted.set(identity, unavailableResponse(request));
		return ordered.map((request) =>
			accepted.get(resourceRequestIdentity(request)),
		);
	}
}

function deduplicateRequests(requests) {
	const unique = new Map();
	for (const request of requests) {
		const identity = resourceRequestIdentity(request);
		if (!unique.has(identity)) unique.set(identity, request);
	}
	return [...unique.values()];
}

export function resourceRequestIdentity(request) {
	if (request?.type === "font") return fontRequestIdentity(request);
	if (request?.type === "pk-font") return pkFontRequestIdentity(request);
	if (request?.type === "legacy-font-mapping")
		return legacyMappingRequestIdentity(request);
	return encodeRequest(request);
}

export function resourceResponseIdentity(response) {
	if (response?.type === "font" || response?.type === "font-unavailable")
		return fontRequestIdentity({ ...response, type: "font" });
	if (
		response?.type === "legacy-font-mapping" ||
		response?.type === "legacy-font-mapping-unavailable"
	)
		return legacyMappingRequestIdentity({
			...response,
			type: "legacy-font-mapping",
		});
	if (response?.type === "file" || response?.type === "file-unavailable")
		return encodeRequest({ ...response, type: "file" });
	if (response?.type === "pk-font" || response?.type === "pk-font-unavailable")
		return pkFontRequestIdentity({ ...response, type: "pk-font" });
	throw new TypeError("resource provider returned an unknown response type");
}

function pkFontRequestIdentity(request) {
	if (
		!(request?.texName instanceof Uint8Array) ||
		!(request?.mode instanceof Uint8Array)
	)
		throw new TypeError("PK font names and modes must be Uint8Array values");
	if (
		!Number.isSafeInteger(request.dpi) ||
		request.dpi < 0 ||
		request.dpi > 0xffff_ffff
	)
		throw new TypeError("PK font DPI must be an unsigned 32-bit integer");
	const hex = (bytes) =>
		[...bytes].map((byte) => byte.toString(16).padStart(2, "0")).join("");
	return `pk-font:${hex(request.texName)}:${request.dpi}:${hex(request.mode)}`;
}

function unavailableResponse(request) {
	return { ...request, type: `${request.type ?? "file"}-unavailable` };
}

function isUnavailable(response) {
	return (
		typeof response?.type === "string" && response.type.endsWith("-unavailable")
	);
}

function throwIfAborted(signal) {
	if (signal?.aborted)
		throw (
			signal.reason ??
			new DOMException("The operation was aborted", "AbortError")
		);
}
