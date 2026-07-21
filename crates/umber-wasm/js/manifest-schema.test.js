import assert from "node:assert/strict";
import { webcrypto } from "node:crypto";
import test from "node:test";

import {
	decodeKey,
	encodeRequest,
	resourceDomain,
	shardIndex,
	validateRootManifest,
} from "./manifest-schema.js";

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

test("typed PDF font resources retain wire identity over tex manifest keys", () => {
	for (const kind of ["vf", "font-map", "font-encoding", "font-program"]) {
		assert.equal(
			encodeRequest({ kind, name: "fixture.bin" }),
			"tex:fixture.bin",
		);
		assert.equal(resourceDomain(kind), "tex");
	}
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

test("schema three validates bounded canonical format closures", () => {
	const format = {
		object: `sha256-${"3".repeat(64)}`,
		sha256: "3".repeat(64),
		bytes: 4,
		engine: "umber",
		engineVersion: "0.1.0",
		formatSchema: 10,
		sourceDistribution: "fixture",
		sourceManifestSha256: "4".repeat(64),
		sourceDateEpoch: 0,
		inputClosure: { schema: 1, keys: ["tex:latex.ltx", "tfm:cmr10.tfm"] },
	};
	const root = {
		schema: 3,
		distribution: "fixture",
		objectsBaseUrl: "https://cdn.example.test/objects/",
		shardBits: 0,
		shardCount: 1,
		shards: ["1".repeat(64)],
		formats: { latex: format },
	};
	assert.deepEqual(validateRootManifest(root).formats.latex.inputClosure.keys, [
		"tex:latex.ltx",
		"tfm:cmr10.tfm",
	]);
	assert.throws(
		() => validateRootManifest({ ...root, schema: 2 }),
		/require root manifest schema 3/,
	);
	assert.throws(
		() =>
			validateRootManifest({
				...root,
				formats: {
					latex: {
						...format,
						inputClosure: {
							schema: 1,
							keys: ["tfm:cmr10.tfm", "tex:latex.ltx"],
						},
					},
				},
			}),
		/not strictly sorted/,
	);
});
