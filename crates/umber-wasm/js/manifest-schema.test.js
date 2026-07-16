import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";

import { selectManifestJobs, validateManifest } from "./manifest-schema.js";

const fixtureRoot = new URL(
	"../../../tests/corpus/distribution/",
	import.meta.url,
);

test("shared Rust and JavaScript fixture selects identical jobs and misses", async () => {
	const manifestText = await readFile(
		new URL("manifest.json", fixtureRoot),
		"utf8",
	);
	const manifest = validateManifest(JSON.parse(manifestText));
	assert.deepEqual(
		validateManifest(JSON.parse(JSON.stringify(manifest))),
		manifest,
	);

	const caseText = await readFile(
		new URL("selection.case", fixtureRoot),
		"utf8",
	);
	const requests = [];
	const expectedJobs = [];
	const expectedMisses = [];
	for (const line of caseText
		.split(/\r?\n/u)
		.filter((line) => line.length > 0 && !line.startsWith("#"))) {
		const fields = line.split("\t");
		if (fields[0] === "request" && fields[1] === "file") {
			requests.push({ kind: fields[2], name: fields[3] });
		} else if (fields[0] === "request" && fields[1] === "font") {
			requests.push({
				type: "font",
				logicalName: fields[2],
				faceIndex: 0,
				variations: [],
				features: [],
			});
		} else if (fields[0] === "job") {
			expectedJobs.push(fields.slice(1).join("\t"));
		} else if (fields[0] === "miss") {
			expectedMisses.push(fields.slice(1).join("\t"));
		} else {
			assert.fail(`invalid shared fixture line: ${line}`);
		}
	}

	const selection = selectManifestJobs(manifest, requests);
	assert.deepEqual(
		selection.jobs.map((job) =>
			[
				job.requested ? "required" : "hint",
				job.type,
				job.manifestKey,
				job.entry.sha256,
			].join("\t"),
		),
		expectedJobs,
	);
	assert.deepEqual(
		selection.misses.map(({ type, manifestKey }) =>
			[type, manifestKey].join("\t"),
		),
		expectedMisses,
	);
});
