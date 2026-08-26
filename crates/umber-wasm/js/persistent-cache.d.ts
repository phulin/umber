export interface PersistentObjectCache {
	get(distribution: string, ahash64: string): Promise<Uint8Array | undefined>;
	put(distribution: string, ahash64: string, bytes: Uint8Array): Promise<void>;
	delete(distribution: string, ahash64: string): Promise<void>;
	close(): void;
}

export interface IndexedDbObjectCacheOptions {
	indexedDB?: IDBFactory;
	databaseName?: string;
}

export class IndexedDbObjectCache implements PersistentObjectCache {
	constructor(options?: IndexedDbObjectCacheOptions);
	get(distribution: string, ahash64: string): Promise<Uint8Array | undefined>;
	put(distribution: string, ahash64: string, bytes: Uint8Array): Promise<void>;
	delete(distribution: string, ahash64: string): Promise<void>;
	close(): void;
}

export class MemoryObjectCache implements PersistentObjectCache {
	get(distribution: string, ahash64: string): Promise<Uint8Array | undefined>;
	put(distribution: string, ahash64: string, bytes: Uint8Array): Promise<void>;
	delete(distribution: string, ahash64: string): Promise<void>;
	close(): void;
}

export function cacheKey(distribution: string, ahash64: string): string;
