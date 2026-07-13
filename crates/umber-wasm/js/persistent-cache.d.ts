export interface PersistentObjectCache {
	get(distribution: string, sha256: string): Promise<Uint8Array | undefined>;
	put(distribution: string, sha256: string, bytes: Uint8Array): Promise<void>;
	delete(distribution: string, sha256: string): Promise<void>;
	close(): void;
}

export interface IndexedDbObjectCacheOptions {
	indexedDB?: IDBFactory;
	databaseName?: string;
}

export class IndexedDbObjectCache implements PersistentObjectCache {
	constructor(options?: IndexedDbObjectCacheOptions);
	get(distribution: string, sha256: string): Promise<Uint8Array | undefined>;
	put(distribution: string, sha256: string, bytes: Uint8Array): Promise<void>;
	delete(distribution: string, sha256: string): Promise<void>;
	close(): void;
}

export class MemoryObjectCache implements PersistentObjectCache {
	get(distribution: string, sha256: string): Promise<Uint8Array | undefined>;
	put(distribution: string, sha256: string, bytes: Uint8Array): Promise<void>;
	delete(distribution: string, sha256: string): Promise<void>;
	close(): void;
}

export function cacheKey(distribution: string, sha256: string): string;
