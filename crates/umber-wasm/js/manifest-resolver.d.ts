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
  persistentCache?: "http" | "none";
  concurrency?: number;
  signal?: AbortSignal;
  fetch?: typeof globalThis.fetch;
  crypto?: Crypto;
}

export interface ManifestFile {
  virtualPath: string;
  object: string;
  sha256: string;
  bytes: number;
  dependencies?: readonly string[];
}

export interface TexLiveManifest {
  schema: 1;
  distribution: string;
  objectsBaseUrl: string;
  files: Readonly<Record<string, ManifestFile>>;
}

export class ManifestResolverError extends Error {
  readonly code: string;
}

export class HttpManifestResolver {
  static create(options: HttpManifestResolverOptions): Promise<HttpManifestResolver>;
  constructor(
    manifest: TexLiveManifest,
    options?: Omit<HttpManifestResolverOptions, "manifestUrl" | "signal">,
  );
  readonly manifest: TexLiveManifest;
  resolve(
    requests: readonly FileRequest[],
    signal?: AbortSignal,
  ): Promise<readonly ResolvedDownload[]>;
}
