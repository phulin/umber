import assert from "node:assert/strict";
import test from "node:test";

import { cacheKey, MemoryObjectCache } from "./persistent-cache.js";

const DIGEST = "a".repeat(64);

test("cache keys isolate distributions and reject malformed identities", () => {
	assert.notEqual(cacheKey("texlive-a", DIGEST), cacheKey("texlive-b", DIGEST));
	assert.throws(() => cacheKey("", DIGEST), /distribution/);
	assert.throws(() => cacheKey("texlive-a", "bad"), /sha256/);
});

test("memory cache copies bytes and keeps versions isolated", async () => {
	const cache = new MemoryObjectCache();
	const source = new Uint8Array([1, 0, 2]);
	await cache.put("texlive-a", DIGEST, source);
	source[0] = 9;
	assert.deepEqual(
		await cache.get("texlive-a", DIGEST),
		new Uint8Array([1, 0, 2]),
	);
	assert.equal(await cache.get("texlive-b", DIGEST), undefined);
	const read = await cache.get("texlive-a", DIGEST);
	read[0] = 8;
	assert.deepEqual(
		await cache.get("texlive-a", DIGEST),
		new Uint8Array([1, 0, 2]),
	);
	await cache.delete("texlive-a", DIGEST);
	assert.equal(await cache.get("texlive-a", DIGEST), undefined);
});
