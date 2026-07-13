const DATABASE_VERSION = 1;
const STORE_NAME = "objects";
const DIGEST_PATTERN = /^[0-9a-f]{64}$/;

export class IndexedDbObjectCache {
	constructor(options = {}) {
		this.indexedDB = options.indexedDB ?? globalThis.indexedDB;
		this.databaseName = options.databaseName ?? "umber-texlive-cache";
		if (!this.indexedDB?.open) {
			throw new Error("IndexedDB is unavailable");
		}
		this.database = undefined;
	}

	async get(distribution, sha256) {
		const database = await this.#database();
		const record = await requestResult(
			database
				.transaction(STORE_NAME, "readonly")
				.objectStore(STORE_NAME)
				.get(cacheKey(distribution, sha256)),
		);
		if (record === undefined) return undefined;
		return new Uint8Array(record.bytes).slice();
	}

	async put(distribution, sha256, bytes) {
		if (!(bytes instanceof Uint8Array))
			throw new TypeError("cache bytes must be a Uint8Array");
		const database = await this.#database();
		const transaction = database.transaction(STORE_NAME, "readwrite");
		transaction.objectStore(STORE_NAME).put({
			key: cacheKey(distribution, sha256),
			bytes: bytes.slice().buffer,
		});
		await transactionDone(transaction);
	}

	async delete(distribution, sha256) {
		const database = await this.#database();
		const transaction = database.transaction(STORE_NAME, "readwrite");
		transaction.objectStore(STORE_NAME).delete(cacheKey(distribution, sha256));
		await transactionDone(transaction);
	}

	close() {
		this.database?.then((database) => database.close()).catch(() => {});
		this.database = undefined;
	}

	#database() {
		if (this.database === undefined) {
			this.database = openDatabase(this.indexedDB, this.databaseName);
		}
		return this.database;
	}
}

export class MemoryObjectCache {
	constructor() {
		this.objects = new Map();
	}

	async get(distribution, sha256) {
		return this.objects.get(cacheKey(distribution, sha256))?.slice();
	}

	async put(distribution, sha256, bytes) {
		this.objects.set(cacheKey(distribution, sha256), bytes.slice());
	}

	async delete(distribution, sha256) {
		this.objects.delete(cacheKey(distribution, sha256));
	}

	close() {
		this.objects.clear();
	}
}

export function cacheKey(distribution, sha256) {
	if (typeof distribution !== "string" || distribution.length === 0) {
		throw new TypeError("distribution must be a non-empty string");
	}
	if (!DIGEST_PATTERN.test(sha256)) {
		throw new TypeError("sha256 must be 64 lowercase hexadecimal characters");
	}
	return JSON.stringify([distribution, sha256]);
}

function openDatabase(indexedDB, databaseName) {
	return new Promise((resolve, reject) => {
		const request = indexedDB.open(databaseName, DATABASE_VERSION);
		request.onupgradeneeded = () => {
			const database = request.result;
			if (!database.objectStoreNames.contains(STORE_NAME)) {
				database.createObjectStore(STORE_NAME, { keyPath: "key" });
			}
		};
		request.onsuccess = () => resolve(request.result);
		request.onerror = () =>
			reject(request.error ?? new Error("opening IndexedDB failed"));
		request.onblocked = () =>
			reject(new Error("opening IndexedDB was blocked"));
	});
}

function requestResult(request) {
	return new Promise((resolve, reject) => {
		request.onsuccess = () => resolve(request.result);
		request.onerror = () =>
			reject(request.error ?? new Error("IndexedDB request failed"));
	});
}

function transactionDone(transaction) {
	return new Promise((resolve, reject) => {
		transaction.oncomplete = () => resolve();
		transaction.onerror = () =>
			reject(transaction.error ?? new Error("IndexedDB transaction failed"));
		transaction.onabort = () =>
			reject(transaction.error ?? new Error("IndexedDB transaction aborted"));
	});
}
