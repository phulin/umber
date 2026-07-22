import type { PersistentObjectCache } from "./persistent-cache.js";
import type { ResourceRequest, ResourceResponse } from "./umber_wasm.js";

export const TEXLIVE_2026_MANIFEST_URL: string;
export const TEXLIVE_2026_MANIFEST_SHA256: string;

export type FileKind =
	| "tex"
	| "tfm"
	| "vf"
	| "font-map"
	| "font-encoding"
	| "font-program"
	| "bib-aux"
	| "classic-bib-data"
	| "bib-style";

export interface FileRequestKey {
	kind: FileKind;
	name: string;
}

export interface FileRequest extends FileRequestKey {
	originalName?: string;
}

export interface ResolvedDownload {
	request: FileRequestKey;
	virtualPath: string;
	bytes: Uint8Array;
}

export interface HttpManifestResolverOptions {
	manifestUrl: string;
	manifestSha256: string;
	persistentCache?: "http" | "indexeddb" | "none";
	offline?: boolean;
	concurrency?: number;
	maxFiles?: number;
	maxBytes?: number;
	signal?: AbortSignal;
	fetch?: typeof globalThis.fetch;
	crypto?: Crypto;
	cacheStore?: PersistentObjectCache;
	indexedDB?: IDBFactory;
}

export interface ManifestFile {
	virtualPath: string;
	object: string;
	sha256: string;
	bytes: number;
	dependencies?: readonly ManifestDependency[];
}

export interface ManifestDependency extends Omit<ManifestFile, "dependencies"> {
	key: string;
}

export interface ManifestFormat {
	object: string;
	sha256: string;
	bytes: number;
	engine: "umber";
	engineVersion: string;
	formatSchema: number;
	sourceDistribution: string;
	sourceManifestSha256: string;
	sourceDateEpoch: number;
	inputClosure?: { schema: 1; keys: readonly string[] };
}

export interface ProvenanceRecord {
	identity: string;
	upstream: string;
	upstreamVersion: string;
	sourceUrl: string;
	conversionTool: string;
	conversionVersion: string;
}

export interface LicenseRecord {
	identity: string;
	object: string;
	sha256: string;
	bytes: number;
	spdx: string;
	embeddable: true;
	redistributable: true;
}

export interface ManifestFontRecord {
	schema: 1;
	object: string;
	sha256: string;
	bytes: number;
	container: "woff2";
	programIdentity?: string;
	featurePolicyVersion: 1;
	provenance: ProvenanceRecord;
	license: LicenseRecord;
}

export interface LegacyMappingRequest {
	type: "legacy-font-mapping";
	tfmSha256: string;
	layoutPolicyVersion: 1;
	purpose: "html-layout" | "html-paint";
	encodingCatalog?: string;
}

export interface ResolvedLegacyMapping extends LegacyMappingRequest {
	fontKey: string;
	container: "woff2";
	bytes: Uint8Array;
	objectSha256: string;
	programIdentity?: string;
	unicodeMap: readonly (string | null)[];
	fallback: "classic-tfm-exact" | "error";
	provenance: string;
}

export interface UnavailableLegacyMapping
	extends Omit<LegacyMappingRequest, "type"> {
	type: "legacy-font-mapping-unavailable";
}

export interface FormatCompatibility {
	engineVersion?: string;
	formatSchema?: number;
}

export interface TexLiveManifest {
	schema: 2 | 3 | 4;
	distribution: string;
	objectsBaseUrl: string;
	shardBits: number;
	shardCount: number;
	shards: readonly string[];
	formats?: Readonly<Record<string, ManifestFormat>>;
}

export class ManifestResolverError extends Error {
	readonly code: string;
}

export class HttpManifestResolver {
	static create(
		options: HttpManifestResolverOptions,
	): Promise<HttpManifestResolver>;
	constructor(
		manifest: TexLiveManifest,
		options?: Omit<
			HttpManifestResolverOptions,
			"manifestUrl" | "manifestSha256" | "signal"
		>,
	);
	readonly manifest: TexLiveManifest;
	resolve(
		requests: readonly (ResourceRequest | LegacyMappingRequest)[],
		options?:
			| AbortSignal
			| {
					signal?: AbortSignal;
					prefetchHints?: readonly ResourceRequest[];
			  },
	): Promise<
		readonly (
			| ResolvedDownload
			| ResourceResponse
			| ResolvedLegacyMapping
			| UnavailableLegacyMapping
		)[]
	>;
	resolveFormat(
		name: string,
		compatibility?: FormatCompatibility,
		signal?: AbortSignal,
	): Promise<Uint8Array>;
	formatMetadata(name: string): ManifestFormat;
	formatPrefetchHints(name: string): readonly ResourceRequest[];
}
