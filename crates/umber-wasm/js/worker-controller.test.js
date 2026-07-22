import assert from "node:assert/strict";
import test from "node:test";

import { compileInWorker, WorkerCompileError } from "./worker-controller.js";
import { outputTransfers, runCompileMessage } from "./worker-entry.js";

const manifestSha256 = "1".repeat(64);

function privateFontResponse(bytes) {
	return {
		type: "font",
		logicalName: "cmr10",
		faceIndex: 0,
		variationInstance: "default",
		variations: [],
		features: [],
		direction: "ltr",
		container: "woff2",
		bytes,
	};
}

function fakeWorker(behavior) {
	return class FakeWorker {
		static instances = [];

		constructor(url, options) {
			this.url = url;
			this.options = options;
			this.listeners = new Map();
			this.terminated = 0;
			FakeWorker.instances.push(this);
		}

		addEventListener(name, listener) {
			this.listeners.set(name, listener);
		}

		removeEventListener(name, listener) {
			if (this.listeners.get(name) === listener) this.listeners.delete(name);
		}

		postMessage(message, transfer) {
			this.message = message;
			this.transfer = transfer;
			behavior?.(this, message);
		}

		terminate() {
			this.terminated += 1;
		}

		emit(name, event) {
			this.listeners.get(name)?.(event);
		}
	};
}

function request(Worker, control = {}) {
	return compileInWorker(
		{ mainPath: "main.tex", format: new Uint8Array([4, 0, 5]) },
		new Map([["main.tex", new Uint8Array([1, 0, 2])]]),
		{
			manifestUrl: "https://cdn.example.test/manifest.json",
			manifestSha256,
			persistentCache: "http",
		},
		{ Worker, workerUrl: "worker.js", timeoutMs: 100, ...control },
	);
}

test("standard entry transfers binary copies and tears down after success", async () => {
	const output = {
		terminal: "done",
		log: new Uint8Array([9, 0]),
		dvi: new Uint8Array([8, 0, 7]),
		files: [{ path: "/job/x.aux", bytes: new Uint8Array([6, 0]) }],
	};
	const Worker = fakeWorker((worker) => {
		queueMicrotask(() =>
			worker.emit("message", { data: { kind: "complete", output } }),
		);
	});
	const result = await request(Worker);
	const worker = Worker.instances[0];
	assert.equal(result, output);
	assert.equal(worker.options.type, "module");
	assert.equal(worker.terminated, 1);
	assert.deepEqual([...worker.message.userFiles[0][1]], [1, 0, 2]);
	assert.deepEqual([...worker.message.options.format], [4, 0, 5]);
	assert.equal(worker.transfer.length, 2);
	assert(worker.transfer.every((value) => value instanceof ArrayBuffer));
});

test("tears down after typed failure and worker protocol failures", async (t) => {
	await t.test("compile failure", async () => {
		const Worker = fakeWorker((worker) => {
			queueMicrotask(() =>
				worker.emit("message", {
					data: {
						kind: "error",
						error: { code: "compile", message: "bad TeX" },
					},
				}),
			);
		});
		await assert.rejects(
			request(Worker),
			(error) =>
				error instanceof WorkerCompileError && error.code === "compile",
		);
		assert.equal(Worker.instances[0].terminated, 1);
	});

	await t.test("worker error", async () => {
		const Worker = fakeWorker((worker) => {
			queueMicrotask(() =>
				worker.emit("error", { message: "crashed", error: new Error("x") }),
			);
		});
		await assert.rejects(request(Worker), /crashed/);
		assert.equal(Worker.instances[0].terminated, 1);
	});

	await t.test("message cloning failure", async () => {
		const Worker = fakeWorker((worker) => {
			queueMicrotask(() => worker.emit("messageerror", {}));
		});
		await assert.rejects(request(Worker), /could not be cloned/);
		assert.equal(Worker.instances[0].terminated, 1);
	});
});

test("owner abort and timeout terminate a worker even while it is unresponsive", async (t) => {
	await t.test("abort", async () => {
		const controller = new AbortController();
		const Worker = fakeWorker(() => {
			queueMicrotask(() => controller.abort(new Error("owner stopped")));
		});
		await assert.rejects(
			request(Worker, { signal: controller.signal }),
			/owner stopped/,
		);
		assert.equal(Worker.instances[0].terminated, 1);
	});

	await t.test("nonterminating TeX timeout", async () => {
		const Worker = fakeWorker();
		const source = new Uint8Array(
			new TextEncoder().encode("\\def\\loop{\\loop}\\loop"),
		);
		const promise = compileInWorker(
			{ mainPath: "main.tex" },
			new Map([["main.tex", source]]),
			{ manifestUrl: "https://cdn.example.test/manifest.json", manifestSha256 },
			{ Worker, workerUrl: "worker.js", timeoutMs: 5 },
		);
		await assert.rejects(
			promise,
			(error) =>
				error instanceof WorkerCompileError && error.code === "timeout",
		);
		assert.equal(Worker.instances[0].terminated, 1);
	});
});

test("aborted owners do not start workers", async () => {
	const controller = new AbortController();
	controller.abort(new Error("already stopped"));
	const Worker = fakeWorker();
	await assert.rejects(
		request(Worker, { signal: controller.signal }),
		/already stopped/,
	);
	assert.equal(Worker.instances.length, 0);
});

test("preflights all worker input limits before copying or construction", async (t) => {
	const cases = [
		{
			name: "format bytes",
			options: {
				mainPath: "main.tex",
				format: new Uint8Array([1, 2]),
				limits: { oneFileBytes: 1 },
			},
			files: new Map(),
		},
		{
			name: "one user file",
			options: { mainPath: "main.tex", limits: { oneFileBytes: 1 } },
			files: new Map([["main.tex", new Uint8Array([1, 2])]]),
		},
		{
			name: "total user bytes",
			options: { mainPath: "main.tex", limits: { userSourceBytes: 1 } },
			files: new Map([
				["main.tex", new Uint8Array([1])],
				["extra.tex", new Uint8Array([2])],
			]),
		},
		{
			name: "one font resource",
			options: {
				mainPath: "main.tex",
				html: {},
				limits: { oneFileBytes: 1 },
			},
			files: new Map(),
			resourceResponses: [privateFontResponse(new Uint8Array([1, 2]))],
		},
		{
			name: "user file count",
			options: { mainPath: "main.tex", limits: { userFiles: 1 } },
			files: new Map([
				["main.tex", new Uint8Array()],
				["extra.tex", new Uint8Array()],
			]),
		},
	];
	for (const fixture of cases) {
		await t.test(fixture.name, async () => {
			const Worker = fakeWorker();
			const originalLengths = [...fixture.files.values()].map(
				(bytes) => bytes.byteLength,
			);
			await assert.rejects(
				compileInWorker(
					fixture.options,
					fixture.files,
					{
						manifestUrl: "https://cdn.example.test/manifest.json",
						manifestSha256,
						resourceResponses: fixture.resourceResponses,
					},
					{ Worker },
				),
				(error) =>
					error instanceof WorkerCompileError && error.code === "limit",
			);
			assert.equal(Worker.instances.length, 0);
			assert.deepEqual(
				[...fixture.files.values()].map((bytes) => bytes.byteLength),
				originalLengths,
			);
		});
	}
});

test("worker copies typed font resources without detaching caller bytes", async () => {
	const Worker = fakeWorker();
	const woff2 = new Uint8Array([119, 79, 70, 50]);
	const promise = compileInWorker(
		{
			mainPath: "main.tex",
			html: {},
		},
		new Map(),
		{
			manifestUrl: "https://cdn.example.test/manifest.json",
			manifestSha256,
			resourceResponses: [privateFontResponse(woff2)],
		},
		{ Worker },
	);
	const worker = Worker.instances[0];
	assert.equal(worker.transfer.length, 1);
	assert.notEqual(worker.message.resolver.resourceResponses[0].bytes, woff2);
	assert.equal(woff2.byteLength, 4);
	worker.emit("message", {
		data: {
			kind: "complete",
			output: {
				terminal: "done",
				log: new Uint8Array(),
				dvi: new Uint8Array(),
				htmlAssets: [],
				files: [],
			},
		},
	});
	await promise;
});

test("rejects ambiguous inline and manifest-selected formats", async () => {
	const Worker = fakeWorker();
	await assert.rejects(
		compileInWorker(
			{ mainPath: "main.tex", format: new Uint8Array([1]) },
			new Map(),
			{
				manifestUrl: "https://cdn.example.test/manifest.json",
				manifestSha256,
				format: "plain",
			},
			{ Worker },
		),
		/both be provided/,
	);
	assert.equal(Worker.instances.length, 0);
});

test("controller forwards a named manifest format without transferring bytes", async () => {
	const output = {
		terminal: "done",
		log: new Uint8Array(),
		dvi: new Uint8Array(),
		files: [],
	};
	const Worker = fakeWorker((worker) => {
		queueMicrotask(() =>
			worker.emit("message", { data: { kind: "complete", output } }),
		);
	});
	await compileInWorker(
		{ mainPath: "main.tex" },
		new Map([["main.tex", new Uint8Array([1])]]),
		{
			manifestUrl: "https://cdn.example.test/manifest.json",
			manifestSha256,
			format: "plain",
		},
		{ Worker, timeoutMs: 100 },
	);
	const worker = Worker.instances[0];
	assert.equal(worker.message.resolver.format, "plain");
	assert.equal(worker.message.options.format, undefined);
	assert.equal(worker.transfer.length, 1);
});

test("worker runtime selects a compatible manifest format", async () => {
	const format = new Uint8Array([9, 0, 8]);
	const formatPrefetchHints = [
		{
			type: "file",
			domain: "tex",
			kind: "tex",
			name: "plain.tex",
			originalName: "plain.tex",
		},
	];
	let receivedOptions;
	class Session {
		constructor(options) {
			receivedOptions = options;
		}
		addUserFile() {}
		compileAttempt() {
			return {
				kind: "complete",
				output: {
					terminal: "ok",
					log: new Uint8Array(),
					dvi: new Uint8Array(),
					files: [],
				},
			};
		}
		dispose() {}
	}
	const compatibility = [];
	await runCompileMessage(
		{
			kind: "compile",
			options: { mainPath: "main.tex" },
			userFiles: [["main.tex", new Uint8Array()]],
			resolver: { manifestUrl: "unused", format: "plain" },
		},
		{
			bindings: {
				CompilerSession: Session,
				packageVersion: () => "0.1.0",
				formatSchemaVersion: () => 4,
			},
			resolver: {
				async resolve() {
					return [];
				},
				async resolveFormat(name, expected) {
					compatibility.push({ name, expected });
					return format;
				},
				formatPrefetchHints() {
					return formatPrefetchHints;
				},
			},
		},
	);

	assert.equal(receivedOptions.format, format);
	assert.equal(receivedOptions.formatPrefetchHints, formatPrefetchHints);
	assert.deepEqual(compatibility, [
		{
			name: "plain",
			expected: { engineVersion: "0.1.0", formatSchema: 4 },
		},
	]);
});

test("worker runtime resolves an exact application-private typed response before the manifest", async () => {
	const request = {
		type: "font",
		logicalName: "cmr10",
		faceIndex: 0,
		variationInstance: "default",
		variations: [],
		features: [],
		direction: "ltr",
		acceptedContainers: ["woff2"],
	};
	let provided;
	class Session {
		constructor() {
			this.round = 0;
		}
		addUserFile() {}
		advance() {
			if (this.round++ === 0) {
				return {
					kind: "need-resources",
					required: [request],
					probes: [],
					prefetchHints: [],
				};
			}
			return {
				kind: "complete",
				output: {
					terminal: "ok",
					log: new Uint8Array(),
					dvi: new Uint8Array(),
					files: [],
				},
			};
		}
		provideResources(responses) {
			provided = responses;
		}
		dispose() {}
	}
	const resource = {
		...privateFontResponse(new Uint8Array([1, 2, 3])),
		provenance: "fixture license",
	};
	await runCompileMessage(
		{
			kind: "compile",
			options: { mainPath: "main.tex" },
			userFiles: [["main.tex", new Uint8Array()]],
			resolver: { manifestUrl: "unused", resourceResponses: [resource] },
		},
		{
			bindings: { CompilerSession: Session },
			resolver: {
				async resolve() {
					return [{ ...request, type: "font-unavailable" }];
				},
			},
		},
	);
	assert.deepEqual(provided, [resource]);
});

test("removed fontResources option names the typed replacement API", async () => {
	const Worker = fakeWorker();
	await assert.rejects(
		compileInWorker(
			{ mainPath: "main.tex", outputs: ["html"] },
			new Map(),
			{
				manifestUrl: "https://cdn.example.test/manifest.json",
				manifestSha256,
				fontResources: [],
			},
			{ Worker },
		),
		(error) =>
			error instanceof WorkerCompileError &&
			error.code === "removed-option" &&
			/resourceResponses/.test(error.message),
	);
	assert.equal(Worker.instances.length, 0);
});

test("worker runtime uses injected bindings and returns unique output transfers", async () => {
	class Session {
		constructor() {
			this.done = false;
		}
		addUserFile() {}
		compileAttempt() {
			this.done = true;
			return {
				kind: "complete",
				output: {
					terminal: "ok",
					log: new Uint8Array([0]),
					dvi: new Uint8Array([0]),
					files: [],
				},
			};
		}
		dispose() {}
	}
	let resolverOptions;
	const output = await runCompileMessage(
		{
			kind: "compile",
			options: {
				mainPath: "main.tex",
				limits: { resolvedFiles: 7, cachedFileBytes: 11 },
			},
			userFiles: [["main.tex", new Uint8Array([0])]],
			resolver: { manifestUrl: "unused" },
		},
		{
			bindings: { CompilerSession: Session },
			async createResolver(options) {
				resolverOptions = options;
				return {
					async resolve() {
						return [];
					},
				};
			},
		},
	);
	assert.equal(output.terminal, "ok");
	assert.equal(resolverOptions.maxFiles, 7);
	assert.equal(resolverOptions.maxBytes, 11);
	assert.equal(outputTransfers(output).length, 2);

	const shared = new Uint8Array([1, 0]);
	assert.equal(
		outputTransfers({ terminal: "", log: shared, dvi: shared, files: [] })
			.length,
		1,
	);
	assert.equal(
		outputTransfers({
			tex: { terminal: "", log: shared, dvi: shared, files: [] },
			bibliography: { files: [{ bytes: shared }] },
			generatedFiles: [{ bytes: new Uint8Array([2]) }],
		}).length,
		2,
	);
});
