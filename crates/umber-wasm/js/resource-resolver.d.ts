import type { ResourceRequest, ResourceResponse } from "./umber_wasm.js";
import type {
	LegacyMappingRequest,
	ResolvedLegacyMapping,
	UnavailableLegacyMapping,
} from "./manifest-resolver.js";

export type TypedResourceRequest = ResourceRequest | LegacyMappingRequest;
export type TypedResourceResponse =
	| ResourceResponse
	| ResolvedLegacyMapping
	| UnavailableLegacyMapping;

export interface ResourceProvider {
	resolve(
		requests: readonly TypedResourceRequest[],
		options?: {
			signal?: AbortSignal;
			probes?: readonly TypedResourceRequest[];
			prefetchHints?: readonly TypedResourceRequest[];
		},
	): Promise<readonly TypedResourceResponse[]>;
}

export function resourceRequestIdentity(request: TypedResourceRequest): string;
export function resourceResponseIdentity(
	response: TypedResourceResponse,
): string;

/** Ordered provider composition with provider-scoped misses and final absence. */
export class CompositeResourceResolver implements ResourceProvider {
	constructor(providers: Iterable<ResourceProvider>);
	resolve(
		requests: readonly TypedResourceRequest[],
		options?: {
			signal?: AbortSignal;
			probes?: readonly TypedResourceRequest[];
			prefetchHints?: readonly TypedResourceRequest[];
		},
	): Promise<readonly TypedResourceResponse[]>;
}
