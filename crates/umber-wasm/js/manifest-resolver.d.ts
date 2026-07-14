import type { PersistentObjectCache } from "./persistent-cache.js";
import type { ResourceRequest, ResourceResponse } from "./umber_wasm.js";

export type FileKind = "tex" | "tfm";

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
	persistentCache?: "http" | "indexeddb" | "none";
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
	dependencies?: readonly string[];
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
}

export interface ManifestFont {
	object: string;
	sha256: string;
	bytes: number;
	container: "woff2";
	provenance?: string;
}

export interface FormatCompatibility {
	engineVersion?: string;
	formatSchema?: number;
}

export interface TexLiveManifest {
	schema: 1;
	distribution: string;
	objectsBaseUrl: string;
	files: Readonly<Record<string, ManifestFile>>;
	fonts?: Readonly<Record<string, ManifestFont>>;
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
		options?: Omit<HttpManifestResolverOptions, "manifestUrl" | "signal">,
	);
	readonly manifest: TexLiveManifest;
	resolve(
		requests: readonly ResourceRequest[],
		options?: AbortSignal | { signal?: AbortSignal },
	): Promise<readonly (ResolvedDownload | ResourceResponse)[]>;
	resolveFormat(
		name: string,
		compatibility?: FormatCompatibility,
		signal?: AbortSignal,
	): Promise<Uint8Array>;
	formatMetadata(name: string): ManifestFormat;
}
