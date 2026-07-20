import assert from "node:assert/strict";
import test from "node:test";

import { CompileFacadeError, compile } from "./compile.js";

const encoder = new TextEncoder();

function bindings(results, hooks = {}) {
	class FakeSession {
		static instances = [];

		constructor(options) {
			this.options = options;
			this.results = [...results];
			this.userFiles = [];
			this.resolved = [];
			this.disposed = false;
			FakeSession.instances.push(this);
		}

		addUserFile(path, bytes) {
			hooks.addUserFile?.(path, bytes);
			this.userFiles.push({ path, bytes });
		}

		provideResources(responses) {
			hooks.provideResources?.(responses);
			this.resolved.push(...responses);
		}

		compileAttempt() {
			return this.results.shift();
		}

		dispose() {
			this.disposed = true;
		}
	}
	return { CompilerSession: FakeSession, ProjectSession: FakeSession };
}

function need(kind, name) {
	return {
		kind: "need-resources",
		required: [{ type: "file", kind, name, originalName: name }],
		probes: [],
		prefetchHints: [],
	};
}

function output() {
	return {
		terminal: "done",
		log: new Uint8Array([0, 1]),
		dvi: new Uint8Array([2, 0]),
		files: [],
	};
}

test("performs successful multi-round retries and always disposes", async () => {
	const compiled = output();
	const wasm = bindings([
		need("tex", "first.tex"),
		need("tfm", "font.tfm"),
		{
			kind: "complete",
			output: compiled,
		},
	]);
	const calls = [];
	const resolver = {
		async resolve(requests) {
			calls.push(requests.map(({ kind, name }) => `${kind}:${name}`));
			return requests.map((request, index) => ({
				...request,
				virtualPath: `/texlive/${request.name}`,
				bytes: new Uint8Array([index, 0, calls.length]),
			}));
		},
	};

	const result = await compile(
		{ mainPath: "main.tex" },
		new Map([["main.tex", encoder.encode("main")]]),
		resolver,
		undefined,
		wasm,
	);
	assert.equal(result, compiled);
	assert.deepEqual(calls, [["tex:first.tex"], ["tfm:font.tfm"]]);
	const session = wasm.CompilerSession.instances[0];
	assert.equal(session.resolved.length, 2);
	assert.deepEqual([...session.resolved[0].bytes], [0, 0, 1]);
	assert.equal(session.disposed, true);
});

test("drives file and font resources through one client-owned resolver API", async () => {
	const font = {
		type: "font",
		logicalName: "cmr10",
		faceIndex: 0,
		variations: [],
		features: [
			{ tag: "kern", enabled: true },
			{ tag: "liga", enabled: true },
		],
		acceptedContainers: ["woff2"],
	};
	const hint = {
		type: "file",
		kind: "tex",
		name: "next.tex",
		originalName: "next",
	};
	const probe = {
		type: "file",
		kind: "tex",
		name: "optional.cfg",
		originalName: "optional.cfg",
	};
	const wasm = bindings([
		{
			kind: "need-resources",
			required: [font],
			probes: [probe],
			prefetchHints: [hint],
		},
		{ kind: "complete", output: output() },
	]);
	let resolverOptions;
	await compile(
		{ mainPath: "main.tex" },
		new Map(),
		{
			async resolve(requests, options) {
				resolverOptions = options;
				return requests.map((request) => ({
					...request,
					container: "woff2",
					bytes: new Uint8Array([119, 79, 70, 50]),
					provenance: "application-selected",
				}));
			},
		},
		undefined,
		wasm,
	);
	assert.deepEqual(resolverOptions.prefetchHints, [hint]);
	assert.deepEqual(resolverOptions.probes, [probe]);
	assert.equal(wasm.CompilerSession.instances[0].resolved[0].type, "font");
});

test("resolves only the blocking probe frontier before retrying WASM", async () => {
	const frontier = {
		type: "file",
		kind: "tex",
		name: "first.cfg",
		originalName: "first.cfg",
	};
	const wasm = bindings([
		{
			kind: "need-resources",
			required: [],
			probes: [frontier],
			prefetchHints: [],
		},
		{ kind: "complete", output: output() },
	]);
	let received;
	await compile(
		{ mainPath: "main.tex" },
		new Map(),
		{
			async resolve(requests, options) {
				received = { requests, options };
				return [{ ...frontier, type: "file-unavailable" }];
			},
		},
		undefined,
		wasm,
	);

	assert.deepEqual(received.requests, []);
	assert.deepEqual(received.options.probes, [frontier]);
	assert.deepEqual(received.options.prefetchHints, []);
	assert.deepEqual(wasm.CompilerSession.instances[0].resolved, [
		{ ...frontier, type: "file-unavailable" },
	]);
});

test("forwards shared Rust resource wire values without a JavaScript kind table", async () => {
	const request = {
		type: "file",
		domain: "bibliography",
		kind: "bib-data",
		name: "references.bib",
		originalName: "references",
	};
	let forwarded;
	const wasm = bindings(
		[
			{
				kind: "need-resources",
				required: [request],
				probes: [],
				prefetchHints: [],
			},
			{ kind: "complete", output: output() },
		],
		{
			provideResources(responses) {
				forwarded = responses;
			},
		},
	);
	const response = {
		...request,
		virtualPath: "/texlive/bib/references.bib",
		bytes: new Uint8Array([0, 255]),
	};
	await compile(
		{ mainPath: "main.tex" },
		new Map(),
		{
			async resolve() {
				return [response];
			},
		},
		undefined,
		wasm,
	);
	assert.deepEqual(forwarded, [response]);
});

test("selects the in-WASM project session while keeping acquisition generic", async () => {
	const request = {
		type: "file",
		domain: "bibliography",
		kind: "bib-data",
		name: "references.bib",
		originalName: "references.bib",
	};
	const compiled = { revision: 1, passes: 3, generatedFiles: [] };
	const wasm = bindings([
		{
			kind: "need-resources",
			required: [request],
			probes: [],
			prefetchHints: [],
		},
		{ kind: "complete", output: compiled },
	]);
	let acquired;
	const result = await compile(
		{
			mainPath: "/job/main.tex",
			bibliography: {
				controlPath: "/job/main.bcf",
				outputs: [{ path: "/job/main.bbl", format: "bbl" }],
			},
		},
		new Map(),
		{
			async resolve(requests) {
				acquired = requests;
				return requests.map((item) => ({
					...item,
					virtualPath: "/texlive/bib/references.bib",
					bytes: encoder.encode("@book{x,title={X}}"),
				}));
			},
		},
		undefined,
		wasm,
	);
	assert.equal(result, compiled);
	assert.deepEqual(acquired, [request]);
	assert.equal(wasm.ProjectSession.instances[0].disposed, true);
});

test("rejects no progress, unresolved keys, and engine diagnostics actionably", async (t) => {
	await t.test("no progress", async () => {
		const wasm = bindings([
			need("tex", "required.tex"),
			{
				kind: "error",
				diagnostic: { code: "no-progress", message: "retry made no progress" },
			},
		]);
		await assert.rejects(
			compile(
				{ mainPath: "main.tex" },
				new Map(),
				{
					async resolve() {
						return [
							{
								type: "file",
								kind: "tex",
								name: "hint.tex",
								virtualPath: "/texlive/hint.tex",
								bytes: new Uint8Array(),
							},
						];
					},
				},
				undefined,
				wasm,
			),
			(error) =>
				error instanceof CompileFacadeError && error.code === "no-progress",
		);
		assert.equal(wasm.CompilerSession.instances[0].disposed, true);
	});

	await t.test("replayed binding is not progress", async () => {
		const wasm = bindings([
			need("tex", "required.tex"),
			need("tex", "required.tex"),
			{
				kind: "error",
				diagnostic: { code: "no-progress", message: "retry made no progress" },
			},
		]);
		await assert.rejects(
			compile(
				{ mainPath: "main.tex" },
				new Map(),
				{
					async resolve(requests) {
						return [
							{
								...requests[0],
								virtualPath: "/texlive/required.tex",
								bytes: new Uint8Array([1]),
							},
						];
					},
				},
				undefined,
				wasm,
			),
			(error) =>
				error instanceof CompileFacadeError && error.code === "no-progress",
		);
	});

	await t.test("resolver failure", async () => {
		const wasm = bindings([need("tex", "absent.tex")]);
		await assert.rejects(
			compile(
				{ mainPath: "main.tex" },
				new Map(),
				{
					async resolve() {
						throw new Error("manifest has no entry");
					},
				},
				undefined,
				wasm,
			),
			(error) =>
				error instanceof CompileFacadeError &&
				error.code === "resolve" &&
				/manifest has no entry/.test(error.message),
		);
	});

	await t.test("engine diagnostic", async () => {
		const diagnostic = { message: "Undefined control sequence", line: 2 };
		const wasm = bindings([{ kind: "error", diagnostic }]);
		await assert.rejects(
			compile(
				{ mainPath: "main.tex" },
				new Map(),
				{
					async resolve() {
						return [];
					},
				},
				undefined,
				wasm,
			),
			(error) => error.code === "compile" && error.diagnostic === diagnostic,
		);
	});
});

test("enforces attempt, file, and byte ceilings outside custom resolvers", async (t) => {
	await t.test("attempts", async () => {
		const wasm = bindings([need("tex", "a.tex"), need("tex", "a.tex")]);
		await assert.rejects(
			compile(
				{ mainPath: "main.tex", limits: { attempts: 1 } },
				new Map(),
				{
					async resolve(requests) {
						return requests.map((request) => ({
							...request,
							virtualPath: "/texlive/a.tex",
							bytes: new Uint8Array(),
						}));
					},
				},
				undefined,
				wasm,
			),
			(error) => error.code === "attempt-limit",
		);
	});

	await t.test("user bytes", async () => {
		const wasm = bindings([]);
		await assert.rejects(
			compile(
				{ mainPath: "main.tex", limits: { oneFileBytes: 1 } },
				new Map([["main.tex", new Uint8Array(2)]]),
				{
					async resolve() {
						return [];
					},
				},
				undefined,
				wasm,
			),
			(error) => error.code === "limit" && /one user file/.test(error.message),
		);
	});

	await t.test("user files", async () => {
		const wasm = bindings([]);
		await assert.rejects(
			compile(
				{ mainPath: "main.tex", limits: { userFiles: 1 } },
				new Map([
					["main.tex", new Uint8Array()],
					["extra.tex", new Uint8Array()],
				]),
				{
					async resolve() {
						return [];
					},
				},
				undefined,
				wasm,
			),
			(error) => error.code === "limit" && /user files/.test(error.message),
		);
	});

	await t.test("resolved files", async () => {
		const error = Object.assign(new Error("resolved files requires 2"), {
			code: "limit",
		});
		const wasm = bindings([need("tex", "a.tex")], {
			provideResources() {
				throw error;
			},
		});
		await assert.rejects(
			compile(
				{ mainPath: "main.tex", limits: { resolvedFiles: 1 } },
				new Map(),
				{
					async resolve(requests) {
						return [
							{
								...requests[0],
								virtualPath: "/texlive/a.tex",
								bytes: new Uint8Array(),
							},
							{
								type: "file",
								kind: "tex",
								name: "hint.tex",
								virtualPath: "/texlive/hint.tex",
								bytes: new Uint8Array(),
							},
						];
					},
				},
				undefined,
				wasm,
			),
			(error) => error.code === "limit" && /resolved files/.test(error.message),
		);
	});

	await t.test("resolved bytes", async () => {
		const error = Object.assign(new Error("cached file bytes requires 2"), {
			code: "limit",
		});
		const wasm = bindings([need("tex", "a.tex")], {
			provideResources() {
				throw error;
			},
		});
		await assert.rejects(
			compile(
				{ mainPath: "main.tex", limits: { cachedFileBytes: 1 } },
				new Map(),
				{
					async resolve(requests) {
						return [
							{
								...requests[0],
								virtualPath: "/texlive/a.tex",
								bytes: new Uint8Array(2),
							},
						];
					},
				},
				undefined,
				wasm,
			),
			(error) =>
				error.code === "limit" && /cached file bytes/.test(error.message),
		);
	});

	await t.test("aliases share cached byte accounting", async () => {
		const wasm = bindings([
			{
				kind: "need-resources",
				required: [
					{ type: "file", kind: "tex", name: "a.tex", originalName: "a" },
					{
						type: "file",
						kind: "tex",
						name: "path/a.tex",
						originalName: "path/a",
					},
				],
				probes: [],
				prefetchHints: [],
			},
			{ kind: "complete", output: output() },
		]);
		await compile(
			{ mainPath: "main.tex", limits: { cachedFileBytes: 1 } },
			new Map(),
			{
				async resolve(requests) {
					return requests.map((request) => ({
						...request,
						virtualPath: "/texlive/path/a.tex",
						bytes: new Uint8Array([1]),
					}));
				},
			},
			undefined,
			wasm,
		);
	});
});

test("observes abort before attempts and after an in-flight fetch", async (t) => {
	await t.test("before attempt", async () => {
		const controller = new AbortController();
		controller.abort(new Error("stop before"));
		const wasm = bindings([]);
		await assert.rejects(
			compile(
				{ mainPath: "main.tex" },
				new Map(),
				{
					async resolve() {
						return [];
					},
				},
				controller.signal,
				wasm,
			),
			/stop before/,
		);
		assert.equal(wasm.CompilerSession.instances.length, 0);
	});

	await t.test("during fetch", async () => {
		const controller = new AbortController();
		let provisioned = false;
		const wasm = bindings([need("tex", "a.tex")], {
			provideResources() {
				provisioned = true;
			},
		});
		await assert.rejects(
			compile(
				{ mainPath: "main.tex" },
				new Map(),
				{
					async resolve() {
						controller.abort(new Error("stop during fetch"));
						await Promise.resolve();
						return [];
					},
				},
				controller.signal,
				wasm,
			),
			/stop during fetch/,
		);
		assert.equal(wasm.CompilerSession.instances[0].disposed, true);
		assert.equal(provisioned, false);
	});
});
