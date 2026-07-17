import assert from "node:assert/strict";
import { webcrypto } from "node:crypto";
import test from "node:test";

import { decodeKey, encodeRequest, shardIndex, validateRootManifest } from "./manifest-schema.js";

test("canonical shard selection matches publisher parity vectors", async () => {
	const vectors = [
		["tex:plain.tex", 8, 138],
		["tfm:cmr10.tfm", 8, 145],
		["tex:article.cls", 8, 69],
		["tex:absent.tex", 8, 22],
	];
	for (const [key, bits, expected] of vectors) {
		assert.equal(await shardIndex(key, bits, webcrypto), expected, key);
	}
	assert.equal(await shardIndex("tex:plain.tex", 0, webcrypto), 0);
	assert.equal(await shardIndex("tex:plain.tex", 16, webcrypto), 35536);
});

test("classic request keys share the Rust distribution vocabulary", () => {
	assert.equal(
		encodeRequest({ kind: "classic-bib-data", name: "refs.bib" }),
		"classic-bib:refs.bib",
	);
	assert.deepEqual(decodeKey("bst:plain.bst"), {
		kind: "bib-style",
		name: "plain.bst",
	});
});

test("root validation requires a consistent power-of-two shard table", () => {
	const root = {
		schema: 2,
		distribution: "fixture",
		objectsBaseUrl: "https://cdn.example.test/objects/",
		shardBits: 1,
		shardCount: 2,
		shards: ["1".repeat(64), "2".repeat(64)],
		formats: {},
	};
	assert.equal(validateRootManifest(root).shardCount, 2);
	assert.throws(
		() => validateRootManifest({ ...root, shardCount: 3 }),
		/inconsistent/,
	);
	assert.throws(
		() =>
			validateRootManifest({
				...root,
				shards: [root.shards[0], root.shards[0]],
			}),
		/inconsistent/,
	);
});
