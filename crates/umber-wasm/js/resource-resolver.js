import {
	fontRequestIdentity,
	legacyMappingRequestIdentity,
} from "./manifest-schema.js";
import { typedRequestIdentity } from "./prefetch.js";

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

	/** Starts accepted-run prediction on providers that own immutable catalogs. */
	async beginRun(context = {}) {
		const hints = [];
		const providerPrecedence = this.providers
			.map((provider, index) => `${index}:${providerIdentity(provider, index)}`)
			.join(">");
		const options = {
			...(context.options ?? {}),
			providerPrecedence,
		};
		const providerContext = { ...context, options };
		for (const provider of this.providers) {
			if (typeof provider.beginRun !== "function") continue;
			const result = (await provider.beginRun(providerContext)) ?? {};
			if (Array.isArray(result.hints)) hints.push(...result.hints);
		}
		return { hints };
	}

	async commitRun() {
		for (const provider of this.providers) await provider.commitRun?.();
	}

	discardRun() {
		for (const provider of this.providers) provider.discardRun?.();
	}

	async resolve(requests, options = {}) {
		if (!Array.isArray(requests))
			throw new TypeError("requests must be an array");
		const probes = options?.probes ?? [];
		if (!Array.isArray(probes)) throw new TypeError("probes must be an array");
		const prefetchHints = options?.prefetchHints ?? [];
		if (!Array.isArray(prefetchHints))
			throw new TypeError("prefetchHints must be an array");
		const admitPrefetch = options?.admitPrefetch === true;
		const signal = options?.signal;
		throwIfAborted(signal);

		const blocking = deduplicateRequests(requests.concat(probes));
		const blockingKeys = new Set(blocking.map(resourceRequestIdentity));
		const hints = deduplicateRequests(prefetchHints).filter(
			(request) => !blockingKeys.has(resourceRequestIdentity(request)),
		);
		const ordered = blocking.concat(hints);
		const pending = new Map(
			ordered.map((request) => [resourceRequestIdentity(request), request]),
		);
		const accepted = new Map();
		const speculative = new Map();
		const probeKeys = new Set(probes.map(resourceRequestIdentity));
		const hintKeys = new Set(hints.map(resourceRequestIdentity));

		for (const [providerIndex, provider] of this.providers.entries()) {
			if (pending.size === 0) break;
			throwIfAborted(signal);
			const providerPending = [...pending.values()];
			const providerProbes = providerPending.filter((request) =>
				probeKeys.has(resourceRequestIdentity(request)),
			);
			const providerRequests = providerPending.filter(
				(request) =>
					!probeKeys.has(resourceRequestIdentity(request)) &&
					!hintKeys.has(resourceRequestIdentity(request)),
			);
			const providerHints = providerPending.filter((request) =>
				hintKeys.has(resourceRequestIdentity(request)),
			);
			const responses = await provider.resolve(providerRequests, {
				signal,
				probes: providerProbes,
				prefetchHints: providerHints,
				admitPrefetch,
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
				if (!pending.has(identity) && response?.speculative !== true)
					throw new TypeError(
						`resource provider returned unexpected response ${identity}`,
					);
				seen.add(identity);
				if (isUnavailable(response)) {
					const metadata = responseToRequest(response);
					const pendingRequest = pending.get(identity);
					if (metadata !== undefined && pendingRequest !== undefined) {
						pending.set(identity, {
							...pendingRequest,
							...(metadata.searchContext === undefined
								? {}
								: { searchContext: metadata.searchContext }),
							...(metadata.negativeScope === undefined
								? {}
								: { negativeScope: metadata.negativeScope }),
						});
					}
					continue;
				}
				if (!pending.has(identity)) {
					const request = responseToRequest(response);
					let higher;
					if (request !== undefined) {
						for (const higherProvider of this.providers.slice(
							0,
							providerIndex,
						)) {
							const higherResponses = await higherProvider.resolve([request], {
								signal,
								probes: [],
								prefetchHints: [],
								admitPrefetch: false,
							});
							const positive = [...(higherResponses ?? [])].find(
								(candidate) => !isUnavailable(candidate),
							);
							if (positive !== undefined) {
								higher = positive;
								break;
							}
						}
					}
					speculative.set(identity, markSpeculative(higher ?? response));
					continue;
				}
				accepted.set(identity, response);
				pending.delete(identity);
			}
		}

		for (const [identity, request] of pending)
			accepted.set(identity, unavailableResponse(request));
		const responses = blocking.map((request) =>
			accepted.get(resourceRequestIdentity(request)),
		);
		if (admitPrefetch)
			responses.push(
				...hints.flatMap((request) => {
					const response = accepted.get(resourceRequestIdentity(request));
					return response === undefined || isUnavailable(response)
						? []
						: [response];
				}),
			);
		return responses.concat([...speculative.values()]);
	}
}

function providerIdentity(provider, index) {
	const configured = provider.prefetchProviderIdentity;
	const identity =
		typeof configured === "function" ? configured.call(provider) : configured;
	if (typeof identity === "string" && identity.length > 0) return identity;
	return provider?.constructor?.name || `provider-${index}`;
}

function responseToRequest(response) {
	if (response?.type === "file" || response?.type === "file-unavailable") {
		return {
			type: "file",
			domain: response.domain,
			kind: response.kind,
			name: response.name,
			originalName: response.originalName ?? response.name,
			...(response.searchContext === undefined
				? {}
				: { searchContext: response.searchContext }),
			...(response.negativeScope === undefined
				? {}
				: { negativeScope: response.negativeScope }),
		};
	}
	if (response?.type === "font") return { ...response, type: "font" };
	if (response?.type === "legacy-font-mapping")
		return { ...response, type: "legacy-font-mapping" };
	return undefined;
}

function markSpeculative(response) {
	return response?.speculative === true
		? response
		: { ...response, speculative: true };
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
	if (request?.type === "file") return typedRequestIdentity(request);
	if (request?.type === "font") return fontRequestIdentity(request);
	if (request?.type === "pk-font") return pkFontRequestIdentity(request);
	if (request?.type === "legacy-font-mapping")
		return legacyMappingRequestIdentity(request);
	return typedRequestIdentity(request);
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
		return typedRequestIdentity({ ...response, type: "file" });
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
