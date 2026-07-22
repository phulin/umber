import { validateSessionLimits } from "./compile.js";

const DEFAULT_TIMEOUT_MS = 10_000;
const MAX_TIMEOUT_MS = 60_000;

export class WorkerCompileError extends Error {
	constructor(code, message, options = {}) {
		super(message, { cause: options.cause });
		this.name = "WorkerCompileError";
		this.code = code;
		if (options.diagnostic !== undefined) this.diagnostic = options.diagnostic;
	}
}

export async function compileInWorker(
	options,
	userFiles,
	resolver,
	control = {},
) {
	if (control.signal?.aborted)
		return Promise.reject(abortReason(control.signal));
	const timeoutMs = validateTimeout(control.timeoutMs ?? DEFAULT_TIMEOUT_MS);
	const WorkerClass = control.Worker ?? globalThis.Worker;
	if (typeof WorkerClass !== "function") {
		return Promise.reject(
			new WorkerCompileError("worker-unavailable", "Worker is unavailable"),
		);
	}
	const workerUrl =
		control.workerUrl ?? new URL("./worker-entry.js", import.meta.url);
	let worker;
	let prepared;
	try {
		prepared = prepareMessage(options, userFiles, resolver, control.wasmUrl);
		worker = new WorkerClass(workerUrl, {
			type: "module",
			name: "umber-compile",
		});
	} catch (error) {
		worker?.terminate();
		return Promise.reject(error);
	}

	return new Promise((resolve, reject) => {
		let settled = false;
		const finish = (callback, value) => {
			if (settled) return;
			settled = true;
			clearTimeout(timer);
			control.signal?.removeEventListener("abort", onAbort);
			worker.removeEventListener("message", onMessage);
			worker.removeEventListener("error", onWorkerError);
			worker.removeEventListener("messageerror", onMessageError);
			worker.terminate();
			callback(value);
		};
		const onMessage = (event) => {
			const message = event.data;
			if (message?.kind === "complete") {
				finish(resolve, message.output);
			} else if (message?.kind === "error") {
				finish(
					reject,
					new WorkerCompileError(
						message.error?.code ?? "worker",
						message.error?.message ?? "worker compilation failed",
						{
							diagnostic: message.error?.diagnostic,
						},
					),
				);
			} else {
				finish(
					reject,
					new WorkerCompileError(
						"worker-protocol",
						"worker returned an invalid message",
					),
				);
			}
		};
		const onWorkerError = (event) => {
			finish(
				reject,
				new WorkerCompileError(
					"worker",
					event.message ?? "worker execution failed",
					{
						cause: event.error,
					},
				),
			);
		};
		const onMessageError = () => {
			finish(
				reject,
				new WorkerCompileError(
					"worker-protocol",
					"worker response could not be cloned",
				),
			);
		};
		const onAbort = () => finish(reject, abortReason(control.signal));
		const timer = setTimeout(
			() =>
				finish(
					reject,
					new WorkerCompileError("timeout", `worker exceeded ${timeoutMs} ms`),
				),
			timeoutMs,
		);

		worker.addEventListener("message", onMessage);
		worker.addEventListener("error", onWorkerError);
		worker.addEventListener("messageerror", onMessageError);
		control.signal?.addEventListener("abort", onAbort, { once: true });
		try {
			worker.postMessage(prepared.message, prepared.transfer);
		} catch (error) {
			finish(
				reject,
				new WorkerCompileError(
					"worker-protocol",
					"compile request could not be cloned",
					{
						cause: error,
					},
				),
			);
		}
	});
}

/** Creates a retained editor session inside one dedicated module worker. */
export async function createEditorSessionInWorker(
	options,
	userFiles,
	resolver,
	control = {},
) {
	if (control.signal?.aborted) throw abortReason(control.signal);
	const timeoutMs = validateTimeout(control.timeoutMs ?? DEFAULT_TIMEOUT_MS);
	const WorkerClass = control.Worker ?? globalThis.Worker;
	if (typeof WorkerClass !== "function") {
		throw new WorkerCompileError("worker-unavailable", "Worker is unavailable");
	}
	const workerUrl =
		control.workerUrl ?? new URL("./worker-entry.js", import.meta.url);
	const prepared = prepareMessage(
		options,
		userFiles,
		resolver,
		control.wasmUrl,
	);
	prepared.message.kind = "editor-create";
	prepared.message.id = 0;
	const worker = new WorkerClass(workerUrl, {
		type: "module",
		name: "umber-editor",
	});
	const facade = new EditorWorkerFacade(worker, timeoutMs, control.signal);
	try {
		await facade.initialize(prepared.message, prepared.transfer);
		return facade;
	} catch (error) {
		facade.terminate();
		throw error;
	}
}

export class EditorWorkerFacade {
	#worker;
	#timeoutMs;
	#signal;
	#nextId = 1;
	#pending = new Set();
	#status;
	#onOwnerAbort;

	constructor(worker, timeoutMs, signal) {
		this.#worker = worker;
		this.#timeoutMs = timeoutMs;
		this.#signal = signal;
		this.#onOwnerAbort = () => this.terminate();
		signal?.addEventListener("abort", this.#onOwnerAbort, { once: true });
	}

	get disposed() {
		return this.#worker === undefined;
	}

	get status() {
		return this.#status;
	}

	async initialize(message, transfer) {
		await this.#request(message, transfer, "editor-ready");
	}

	async advance(onProgress) {
		return this.#operation("editor-advance", {}, onProgress);
	}

	async stabilize(onProgress) {
		return this.#operation("editor-stabilize", {}, onProgress);
	}

	async applyPatch(patch) {
		return this.#operation("editor-apply-patch", { patch });
	}

	async renderedSourceLocation(page, event, unit, outputId, revision) {
		const result = await this.#operation("editor-rendered-source", {
			page,
			event,
			unit,
			outputId,
			revision,
		});
		return result.location;
	}

	async cancelStabilization() {
		return this.#operation("editor-cancel-stabilization");
	}

	async cancelPendingPatch() {
		return this.#operation("editor-cancel-pending-patch");
	}

	async dispose() {
		if (this.#worker === undefined) return;
		try {
			await this.#operation("editor-dispose");
		} finally {
			this.terminate();
		}
	}

	terminate() {
		this.#signal?.removeEventListener("abort", this.#onOwnerAbort);
		this.#worker?.terminate();
		this.#worker = undefined;
	}

	async #operation(kind, fields = {}, onProgress) {
		const concurrentCancellation =
			kind === "editor-cancel-stabilization" ||
			kind === "editor-cancel-pending-patch";
		const result = await this.#request(
			{
				kind,
				id: this.#nextId++,
				...fields,
			},
			[],
			"editor-result",
			onProgress,
			concurrentCancellation,
		);
		if (result?.status !== undefined) this.#status = result.status;
		else if (result?.kind === "provisional" || result?.kind === "stable") {
			const { output: _output, ...status } = result;
			this.#status = status;
		}
		return result;
	}

	#request(
		message,
		transfer = [],
		expectedKind = "editor-result",
		onProgress,
		allowConcurrent = false,
	) {
		if (this.#worker === undefined) {
			return Promise.reject(
				new WorkerCompileError("disposed", "editor worker has been disposed"),
			);
		}
		if (this.#pending.size > 0 && !allowConcurrent) {
			return Promise.reject(
				new WorkerCompileError(
					"worker-protocol",
					"an editor worker operation is already pending",
				),
			);
		}
		return new Promise((resolve, reject) => {
			const worker = this.#worker;
			const finish = (callback, value, release = false) => {
				if (!this.#pending.has(message.id)) return;
				clearTimeout(timer);
				this.#signal?.removeEventListener("abort", onAbort);
				worker.removeEventListener("message", onMessage);
				worker.removeEventListener("error", onError);
				worker.removeEventListener("messageerror", onMessageError);
				this.#pending.delete(message.id);
				if (release) this.terminate();
				callback(value);
			};
			const onMessage = (event) => {
				if (event.data?.id !== message.id) return;
				if (event.data.kind === "editor-progress") {
					if (event.data.result?.status !== undefined) {
						this.#status = event.data.result.status;
					}
					onProgress?.(event.data.result);
				} else if (event.data.kind === expectedKind) {
					finish(resolve, event.data.result);
				} else if (event.data.kind === "editor-error") {
					finish(
						reject,
						new WorkerCompileError(
							event.data.error?.code ?? "worker",
							event.data.error?.message ?? "editor worker failed",
							{ diagnostic: event.data.error?.diagnostic },
						),
					);
				}
			};
			const onError = (event) =>
				finish(
					reject,
					new WorkerCompileError("worker", event.message ?? "worker failed"),
					true,
				);
			const onMessageError = () =>
				finish(
					reject,
					new WorkerCompileError(
						"worker-protocol",
						"worker response could not be cloned",
					),
					true,
				);
			const onAbort = () => finish(reject, abortReason(this.#signal), true);
			const timer = setTimeout(
				() =>
					finish(
						reject,
						new WorkerCompileError(
							"timeout",
							`worker exceeded ${this.#timeoutMs} ms`,
						),
						true,
					),
				this.#timeoutMs,
			);
			this.#pending.add(message.id);
			worker.addEventListener("message", onMessage);
			worker.addEventListener("error", onError);
			worker.addEventListener("messageerror", onMessageError);
			this.#signal?.addEventListener("abort", onAbort, { once: true });
			try {
				worker.postMessage(message, transfer);
			} catch (error) {
				finish(
					reject,
					new WorkerCompileError(
						"worker-protocol",
						"editor request could not be cloned",
						{ cause: error },
					),
					true,
				);
			}
		});
	}
}

function prepareMessage(options, userFiles, resolver, wasmUrl) {
	if (!options || typeof options !== "object") {
		throw new WorkerCompileError(
			"invalid-options",
			"session options are required",
		);
	}
	const clonedOptions = { ...options };
	if (!userFiles || typeof userFiles[Symbol.iterator] !== "function") {
		throw new WorkerCompileError(
			"invalid-options",
			"userFiles must be an iterable map",
		);
	}
	if (!resolver || typeof resolver.manifestUrl !== "string") {
		throw new WorkerCompileError(
			"invalid-options",
			"resolver.manifestUrl is required",
		);
	}
	if (typeof resolver.manifestSha256 !== "string") {
		throw new WorkerCompileError(
			"invalid-options",
			"resolver.manifestSha256 is required",
		);
	}
	if (clonedOptions.format !== undefined && resolver.format !== undefined) {
		throw new WorkerCompileError(
			"invalid-options",
			"options.format and resolver.format cannot both be provided",
		);
	}
	if (
		resolver.format !== undefined &&
		(typeof resolver.format !== "string" || resolver.format.length === 0)
	) {
		throw new WorkerCompileError(
			"invalid-options",
			"resolver.format must be a nonempty string",
		);
	}
	const transfer = [];
	let limits;
	try {
		limits = validateSessionLimits(clonedOptions.limits);
	} catch (error) {
		throw new WorkerCompileError(
			error?.code ?? "invalid-options",
			error instanceof Error ? error.message : String(error),
			{ cause: error },
		);
	}
	clonedOptions.limits = limits;
	if (clonedOptions.html !== undefined) {
		if (!clonedOptions.html || typeof clonedOptions.html !== "object") {
			throw new WorkerCompileError("invalid-options", "html must be an object");
		}
	}
	if (resolver.fontResources !== undefined) {
		throw new WorkerCompileError(
			"removed-option",
			"resolver.fontResources was removed; use resolver.resourceResponses with complete typed request/response keys, or add a provider through CompositeResourceResolver",
		);
	}
	let resourceResponses;
	if (resolver.resourceResponses !== undefined) {
		if (!Array.isArray(resolver.resourceResponses)) {
			throw new WorkerCompileError(
				"invalid-resource-responses",
				"resolver.resourceResponses must be an array",
			);
		}
		let resourceBytes = 0;
		resourceResponses = resolver.resourceResponses.map((response, index) => {
			if (response?.bytes !== undefined)
				requireBytes(response.bytes, `resource response ${index} bytes`);
			if ((response?.bytes?.byteLength ?? 0) > limits.oneFileBytes)
				throw workerLimitError(
					"one resource response bytes",
					limits.oneFileBytes,
					response.bytes.byteLength,
				);
			resourceBytes += response?.bytes?.byteLength ?? 0;
			if (resourceBytes > limits.cachedFileBytes)
				throw workerLimitError(
					"resource response bytes",
					limits.cachedFileBytes,
					resourceBytes,
				);
			const bytes = response?.bytes?.slice();
			if (bytes !== undefined) transfer.push(bytes.buffer);
			return {
				...response,
				...(bytes === undefined ? {} : { bytes }),
				legacyMapping: response.legacyMapping && {
					...response.legacyMapping,
					encoding: response.legacyMapping.encoding.slice(),
				},
				unicodeMap: response.unicodeMap?.slice(),
			};
		});
	}
	if (clonedOptions.format !== undefined) {
		requireBytes(clonedOptions.format, "format");
		if (clonedOptions.format.byteLength > limits.oneFileBytes) {
			throw workerLimitError(
				"format image bytes",
				limits.oneFileBytes,
				clonedOptions.format.byteLength,
			);
		}
		clonedOptions.format = clonedOptions.format.slice();
		transfer.push(clonedOptions.format.buffer);
	}
	const sourceFiles = [];
	let sourceBytes = 0;
	for (const [path, bytes] of userFiles) {
		if (typeof path !== "string") {
			throw new WorkerCompileError(
				"invalid-options",
				"user file paths must be strings",
			);
		}
		requireBytes(bytes, `user file ${path}`);
		if (sourceFiles.length + 1 > limits.userFiles) {
			throw workerLimitError(
				"user files",
				limits.userFiles,
				sourceFiles.length + 1,
			);
		}
		if (bytes.byteLength > limits.oneFileBytes) {
			throw workerLimitError(
				"one user file bytes",
				limits.oneFileBytes,
				bytes.byteLength,
			);
		}
		sourceBytes += bytes.byteLength;
		if (sourceBytes > limits.userSourceBytes) {
			throw workerLimitError(
				"user source bytes",
				limits.userSourceBytes,
				sourceBytes,
			);
		}
		sourceFiles.push([path, bytes]);
	}
	const files = sourceFiles.map(([path, bytes]) => {
		const copy = bytes.slice();
		transfer.push(copy.buffer);
		return [path, copy];
	});
	return {
		message: {
			kind: "compile",
			options: clonedOptions,
			userFiles: files,
			resolver: {
				manifestUrl: resolver.manifestUrl,
				manifestSha256: resolver.manifestSha256,
				persistentCache: resolver.persistentCache,
				offline: resolver.offline,
				concurrency: resolver.concurrency,
				format: resolver.format,
				resourceResponses,
			},
			wasmUrl,
		},
		transfer,
	};
}

function workerLimitError(resource, limit, attempted) {
	return new WorkerCompileError(
		"limit",
		`${resource} requires ${attempted}, exceeding limit ${limit}`,
	);
}

function validateTimeout(value) {
	if (!Number.isSafeInteger(value) || value < 1 || value > MAX_TIMEOUT_MS) {
		throw new WorkerCompileError(
			"invalid-options",
			`timeoutMs must be an integer from 1 through ${MAX_TIMEOUT_MS}`,
		);
	}
	return value;
}

function requireBytes(value, label) {
	if (!(value instanceof Uint8Array)) {
		throw new WorkerCompileError(
			"invalid-options",
			`${label} must be a Uint8Array`,
		);
	}
}

function abortReason(signal) {
	return (
		signal.reason ?? new DOMException("The operation was aborted", "AbortError")
	);
}
