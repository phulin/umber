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
	return { CompilerSession: FakeSession };
}

function need(kind, name) {
	return {
		kind: "need-resources",
		required: [{ type: "file", kind, name, originalName: name }],
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
				request,
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
	const wasm = bindings([
		{ kind: "need-resources", required: [font], prefetchHints: [hint] },
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
	assert.equal(wasm.CompilerSession.instances[0].resolved[0].type, "font");
});

test("rejects no progress, unresolved keys, and engine diagnostics actionably", async (t) => {
	await t.test("no progress", async () => {
		const wasm = bindings([need("tex", "required.tex")]);
		await assert.rejects(
			compile(
				{ mainPath: "main.tex" },
				new Map(),
				{
					async resolve() {
						return [
							{
								request: { kind: "tex", name: "hint.tex" },
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
		]);
		await assert.rejects(
			compile(
				{ mainPath: "main.tex" },
				new Map(),
				{
					async resolve(requests) {
						return [
							{
								request: requests[0],
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
							request,
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
		const wasm = bindings([need("tex", "a.tex")]);
		await assert.rejects(
			compile(
				{ mainPath: "main.tex", limits: { resolvedFiles: 1 } },
				new Map(),
				{
					async resolve(requests) {
						return [
							{
								request: requests[0],
								virtualPath: "/texlive/a.tex",
								bytes: new Uint8Array(),
							},
							{
								request: { kind: "tex", name: "hint.tex" },
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
		const wasm = bindings([need("tex", "a.tex")]);
		await assert.rejects(
			compile(
				{ mainPath: "main.tex", limits: { cachedFileBytes: 1 } },
				new Map(),
				{
					async resolve(requests) {
						return [
							{
								request: requests[0],
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
						request,
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
		const wasm = bindings([need("tex", "a.tex")]);
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
	});
});
