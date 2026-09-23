import assert from "node:assert/strict";
import { mkdtemp, readFile, rm } from "node:fs/promises";
import { createServer } from "node:http";
import os from "node:os";
import path from "node:path";
import { pathToFileURL } from "node:url";
import { generateFixture } from "./generate-fixture.mjs";
import { runBrowserFixture } from "./run-browser.mjs";

const directory = path.dirname(new URL(import.meta.url).pathname);
const repository = path.resolve(directory, "../../..");
const packageDirectory = path.resolve(
	process.argv[2] ?? path.join(repository, "target/umber-wasm-package"),
);
const { deterministicAhash64Hex, HttpManifestResolver } = await import(
	pathToFileURL(path.join(packageDirectory, "manifest-resolver.js"))
);
const publisher = path.resolve(
	process.argv[3] ??
		path.join(
			repository,
			"tools/texlive-wasm-publish/target/debug/texlive-wasm-publish",
		),
);
const temporary = await mkdtemp(
	path.join(os.tmpdir(), "umber-browser-integration-"),
);
const publication = path.join(temporary, "publication");
const port = 39247;
let server;
try {
	server = createServer(async (request, response) => {
		try {
			const pathname = new URL(request.url, `http://127.0.0.1:${port}`)
				.pathname;
			const [prefix, ...parts] = pathname.split("/").filter(Boolean);
			const roots = {
				package: packageDirectory,
				publication,
				fixture: directory,
			};
			if (
				!Object.hasOwn(roots, prefix) ||
				parts.some((part) => part === "..")
			) {
				response.writeHead(404).end();
				return;
			}
			const file = path.join(roots[prefix], ...parts);
			const bytes = await readFile(file);
			const type = file.endsWith(".wasm")
				? "application/wasm"
				: file.endsWith(".js")
					? "text/javascript"
					: file.endsWith(".json")
						? "application/json"
						: file.endsWith(".html")
							? "text/html"
							: "application/octet-stream";
			response.writeHead(200, {
				"content-type": type,
				"content-length": bytes.length,
			});
			response.end(bytes);
		} catch (error) {
			response.writeHead(error.code === "ENOENT" ? 404 : 500).end();
		}
	});
	await new Promise((resolve, reject) => {
		server.once("error", reject);
		server.listen(port, "127.0.0.1", resolve);
	});
	const base = `http://127.0.0.1:${port}`;
	const { root, rootAHash64 } = await generateFixture(
		publication,
		`${base}/publication/objects/`,
		publisher,
	);
	await checkRealBindings(base, root, rootAHash64);
	await checkSharedResourceTransitions(base, rootAHash64);
	console.log("generated WASM packed catalog and Rust prefetch: PASS");
	const result = await runBrowserFixture(
		`${base}/fixture/fixture.html?digest=${rootAHash64}`,
		temporary,
	);
	if (result === undefined) {
		console.error(
			"browser distribution integration: BLOCKED (Chrome/Chromium unavailable)",
		);
		process.exitCode = 4;
	} else {
		assert.deepEqual(result, {
			catalog: "packed-rust",
			worker: "resource-round-trip",
		});
		console.log("headless browser packaged worker/resource flow: PASS");
	}
} finally {
	server?.close();
	await rm(temporary, { recursive: true, force: true });
}

async function checkRealBindings(base, root, rootAHash64) {
	const bindings = await import(
		pathToFileURL(path.join(packageDirectory, "umber_wasm.js"))
	);
	const wasm = await readFile(
		path.join(packageDirectory, "umber_wasm_bg.wasm"),
	);
	bindings.initSync({ module: wasm });
	const session = bindings.catalogCreateSession(`${JSON.stringify(root)}\n`);
	const keys = ["tex:probe.tex", "tex:hint.tex"];
	const descriptors = session.prepareBatch(keys).shards;
	assert(descriptors.length > 0);
	assert.throws(() => session.planBatch(keys), /not been provided/);
	const first = descriptors[0];
	const firstBytes = await readFile(
		path.join(publication, "objects", first.object),
	);
	const altered = firstBytes.slice();
	altered[0] ^= 1;
	assert.throws(() => session.provideShard(first.index, altered), /digest/);
	for (const descriptor of descriptors) {
		const bytes = await readFile(
			path.join(publication, "objects", descriptor.object),
		);
		session.provideShard(descriptor.index, bytes);
		session.provideShard(descriptor.index, bytes);
	}
	const plan = session.planBatch(keys);
	assert.deepEqual(plan.misses, []);
	assert(plan.jobs.some((job) => job.manifestKey === "tex:probe.tex"));
	assert.equal(session.prepareBatch(keys).shards.length, 0);
	const other = (first.index + 1) % root.shardCount;
	const wrongRoot = { ...root, shards: [...root.shards] };
	wrongRoot.shards[first.index] = root.shards[other];
	wrongRoot.shards[other] = root.shards[first.index];
	const wrongSession = bindings.catalogCreateSession(
		`${JSON.stringify(wrongRoot)}\n`,
	);
	const wrongBytes = await readFile(
		path.join(publication, "objects", `ahash64-v1-${root.shards[other]}`),
	);
	assert.throws(() => wrongSession.provideShard(first.index, wrongBytes));
	const hints = bindings.prefetchLiteralHints("\\input probe.tex");
	assert(hints.some((hint) => hint.name === "probe.tex"));
	const catalogOnly = await HttpManifestResolver.create({
		manifestUrl: `${base}/publication/manifest.json`,
		manifestAHash64: rootAHash64,
		catalog: { catalogCreateSession: bindings.catalogCreateSession },
		maxFiles: 1,
	});
	assert.equal(catalogOnly.bindPrefetchPolicy({}), "unavailable-v1");
	const unboundRun = await catalogOnly.beginRun({
		source: "\\input hint.tex",
		options: { engine: "tex82" },
	});
	assert.deepEqual(unboundRun.hints, []);
	assert.deepEqual(catalogOnly.literalPrefetchHints("\\input hint.tex"), []);
	const unboundResources = await catalogOnly.resolve(
		[{ kind: "tex", name: "probe.tex" }],
		{
			signal: undefined,
			prefetchHints: [{ kind: "tex", name: "hint.tex" }],
			admitPrefetch: true,
		},
	);
	assert.deepEqual(
		unboundResources.map(({ name }) => name),
		["probe.tex"],
	);
	catalogOnly.noteAdmitted(unboundResources);
	assert.equal(catalogOnly.metrics.prefetchBytes, 0);
	assert.equal(catalogOnly.metrics.packageGroupCandidates, 0);
	assert(catalogOnly.metrics.demandBytes > 0);
	assert.deepEqual(catalogOnly.takePrefetchHints(), []);
	await catalogOnly.commitRun();
	assert.equal(
		catalogOnly.bindPrefetchPolicy(bindings),
		bindings.prefetchPolicyVersion(),
	);
	const reboundRun = await catalogOnly.beginRun({
		source: "\\input probe.tex",
		options: { engine: "tex82" },
	});
	assert(reboundRun.hints.some((hint) => hint.name === "probe.tex"));
	catalogOnly.discardRun();
	const resolver = await HttpManifestResolver.create({
		manifestUrl: `${base}/publication/manifest.json`,
		manifestAHash64: rootAHash64,
		catalog: bindings,
	});
	assert.equal(
		resolver.bindPrefetchPolicy(bindings),
		bindings.prefetchPolicyVersion(),
	);
	const run = await resolver.beginRun({
		source: "\\input probe.tex",
		options: { engine: "tex82" },
	});
	assert(run.hints.some((hint) => hint.name === "probe.tex"));
	assert.equal(resolver.metrics.literalPrefetchHints, 1);
	const resources = await resolver.resolve(
		[{ kind: "tex", name: "probe.tex" }],
		{ signal: undefined, admitPrefetch: true },
	);
	assert(
		resources.some(
			(resource) =>
				resource.name === "probe.tex" &&
				new TextDecoder()
					.decode(resource.bytes)
					.includes("PACKED-CATALOG-RESOURCE"),
		),
	);
	assert(resources.some((resource) => resource.name === "hint.tex"));
	assert(
		resources.some(
			(resource) =>
				resource.name === "hint.tex" && resource.speculative === true,
		),
	);
	resolver.noteAdmitted(resources);
	assert(resolver.metrics.demandBytes > 0);
	assert(resolver.metrics.prefetchBytes > 0);
	await resolver.commitRun();
	const absent = await resolver.resolve([{ kind: "tex", name: "absent.tex" }]);
	assert.equal(absent[0].type, "file-unavailable");
	assert.equal(
		deterministicAhash64Hex(
			await readFile(path.join(publication, "manifest.json")),
		),
		rootAHash64,
	);
}

async function checkSharedResourceTransitions(base, rootAHash64) {
	const fixture = JSON.parse(
		await readFile(
			path.join(repository, "tests/resource-transition-cases.json"),
		),
	);
	assert.equal(fixture.schema, 1);
	const expectedNames = [
		"authoritative-missing-probe",
		"empty-speculation-then-demand",
		"required-positive-retry",
	];
	assert.deepEqual(
		fixture.cases.map(({ name }) => name).sort(),
		expectedNames,
		"shared resource cases must be unique and complete",
	);
	const bindings = await import(
		pathToFileURL(path.join(packageDirectory, "umber_wasm.js"))
	);
	const encoder = new TextEncoder();
	const decoder = new TextDecoder();
	for (const testCase of fixture.cases) {
		const resolver = await HttpManifestResolver.create({
			manifestUrl: `${base}/publication/manifest.json`,
			manifestAHash64: rootAHash64,
			catalog: { catalogCreateSession: bindings.catalogCreateSession },
		});
		assert.equal(resolver.bindPrefetchPolicy({}), "unavailable-v1");
		await resolver.beginRun({
			source: testCase.source,
			options: { engine: "tex82" },
		});
		const session = new bindings.CompilerSession({
			mainPath: "/job/main.tex",
			outputs: ["dvi"],
			formatPrefetchHints: testCase.initialHints.map((name) => ({
				type: "file",
				domain: "tex",
				kind: "tex",
				name,
				originalName: name,
			})),
		});
		try {
			session.addUserFile("main.tex", encoder.encode(testCase.source));
			for (const [index, step] of testCase.steps.entries()) {
				const attempt = session.compileAttempt();
				assert.equal(
					attempt.kind,
					"need-resources",
					`${testCase.name} step ${index}`,
				);
				for (const role of ["required", "probes", "prefetchHints"]) {
					assert.deepEqual(
						attempt[role].map((request) => {
							assert.equal(request.type, "file");
							assert.equal(request.domain, "tex");
							assert.equal(request.kind, "tex");
							return request.name;
						}),
						step.need[role],
						`${testCase.name} step ${index} ${role}`,
					);
				}
				assert(session.acceptedInputObservations == null);
				const requests = [
					...attempt.required,
					...attempt.probes,
					...attempt.prefetchHints,
				];
				const acquired = await resolver.resolve(attempt.required, {
					signal: undefined,
					probes: attempt.probes,
					prefetchHints: attempt.prefetchHints,
					admitPrefetch: true,
				});
				const responses = step.responses.map((expected) => {
					const request = requests.find(({ name }) => name === expected.name);
					assert(request, `${testCase.name}: response has a matching request`);
					const response = acquired.find(({ name }) => name === expected.name);
					assert(
						response,
						`${testCase.name}: resolver answered ${expected.name}`,
					);
					assert.equal(
						response.type,
						expected.outcome === "file" ? "file" : "file-unavailable",
					);
					if (expected.outcome === "file") {
						assert.equal(decoder.decode(response.bytes), expected.bytes);
					} else {
						assert.equal(expected.bytes, undefined);
					}
					return response;
				});
				assert.equal(
					acquired.length,
					responses.length,
					`${testCase.name}: extra response`,
				);
				if (step.rejectLateConflict) {
					assert.equal(responses.length, 1);
					const conflict = {
						...responses[0],
						bytes: encoder.encode("conflicting-late-payload"),
					};
					assert.throws(
						() => session.provideResources([responses[0], conflict]),
						(error) => error.code === "conflicting-resource",
						`${testCase.name}: late batch must reject atomically`,
					);
					assert.equal(session.resolvedFileCount, 0);
					assert(session.acceptedInputObservations == null);
				}
				session.provideResources(responses);
				resolver.noteAdmitted(responses);
			}
			const completed = session.compileAttempt();
			assert.equal(completed.kind, "complete", testCase.name);
			assert(
				completed.output.terminal.includes(testCase.terminal),
				testCase.name,
			);
			assert(session.acceptedInputObservations, testCase.name);
			await resolver.commitRun();
		} finally {
			resolver.discardRun();
			session.dispose();
		}
	}
}
