import assert from "node:assert/strict";
import { webcrypto } from "node:crypto";
import { readFileSync } from "node:fs";
import test from "node:test";

import {
	decodeKey,
	encodeRequest,
	fontRequestIdentity,
	legacyMappingRequestIdentity,
	parseFontRequestIdentity,
	parseLegacyMappingIdentity,
	parseManifestJson,
	resourceDomain,
	serializeIndexShard,
	shardIndex,
	validateIndexShard,
	validateRootManifest,
} from "./manifest-schema.js";

const htmlRootFixture = readFileSync(
	new URL(
		"../../../tests/corpus/distribution/html-font-root.json",
		import.meta.url,
	),
	"utf8",
);
const htmlShardTemplate = readFileSync(
	new URL(
		"../../../tests/corpus/distribution/html-font-shard.template.json",
		import.meta.url,
	),
	"utf8",
);
const htmlShardFixture = () =>
	htmlShardTemplate.replace(
		'"__UNICODE_MAP__"',
		[JSON.stringify("A"), ...Array(255).fill("null")].join(","),
	);

test("font request identity isolates advanced instance inputs", () => {
	const base = {
		logicalName: "fixture",
		faceIndex: 0,
		variationInstance: "default",
		variations: [],
		features: [{ tag: "liga", value: 1 }],
		direction: "ltr",
		script: "latn",
		language: "en",
	};
	const identity = fontRequestIdentity(base);
	for (const changed of [
		{ ...base, faceIndex: 1 },
		{ ...base, variationInstance: { namedNameId: 300 } },
		{ ...base, features: [{ tag: "liga", value: 0 }] },
		{ ...base, script: "cyrl" },
		{ ...base, language: "sr" },
	]) {
		assert.notEqual(fontRequestIdentity(changed), identity);
	}
});

test("shared HTML font fixture canonicalizes, selects, and serializes identically", async () => {
	const root = validateRootManifest(JSON.parse(htmlRootFixture));
	const fixture = htmlShardFixture();
	const shard = validateIndexShard(JSON.parse(fixture), root, 0);
	assert.equal(
		serializeIndexShard(shard),
		`${JSON.stringify(JSON.parse(fixture))}\n`,
	);
	const fontKey = Object.keys(shard.fonts)[0];
	const mappingKey = Object.keys(shard.legacyMappings)[0];
	assert.equal(fontRequestIdentity(parseFontRequestIdentity(fontKey)), fontKey);
	assert.equal(
		legacyMappingRequestIdentity(parseLegacyMappingIdentity(mappingKey)),
		mappingKey,
	);
	assert.equal(await shardIndex(fontKey, root.shardBits, webcrypto, true), 0);
	assert.equal(
		await shardIndex(mappingKey, root.shardBits, webcrypto, true),
		0,
	);
});

test("shared HTML fixture rejects identity, mapping, version, object, and license failures", () => {
	const root = validateRootManifest(JSON.parse(htmlRootFixture));
	const fixture = htmlShardFixture();
	const digest = "c".repeat(64);
	const invalid = [
		fixture.replace(
			`"tfmSha256": "${digest}"`,
			`"tfmSha256": "${"a".repeat(64)}"`,
		),
		fixture.replace('"mappingVersion": 1', '"mappingVersion": 2'),
		fixture.replace('"unicodeMap": ["A",null', '"unicodeMap": ["A"'),
		fixture.replace('"license": {', '"missingLicense": {'),
		fixture.replace('"embeddable": true', '"embeddable": false'),
		fixture.replace(
			"6b65726e=00000001,6c696761=00000001",
			"6b65726e=00000001,6b65726e=00000001",
		),
		fixture.replace(
			'"schema": 1,\n      "object"',
			'"schema": 2,\n      "object"',
		),
	];
	for (const value of invalid)
		assert.throws(() => validateIndexShard(JSON.parse(value), root, 0));
	const conflict = fixture
		.replace(`sha256-${"e".repeat(64)}`, `sha256-${"d".repeat(64)}`)
		.replace(`"sha256": "${"e".repeat(64)}"`, `"sha256": "${"d".repeat(64)}"`);
	assert.throws(() => validateIndexShard(JSON.parse(conflict), root, 0));
	assert.throws(
		() =>
			parseManifestJson(
				fixture.replace(
					'"mappingVersion": 1',
					'"mappingVersion": 1, "mappingVersion": 1',
				),
			),
		/duplicate object key mappingVersion/,
	);
});

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
