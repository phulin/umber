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
		if (!Array.isArray(clonedOptions.html.fonts)) {
			throw new WorkerCompileError(
				"invalid-html-fonts",
				"html.fonts must be an array",
			);
		}
		let fontBytes = 0;
		const fonts = clonedOptions.html.fonts.map((font, index) => {
			if (!font || typeof font !== "object") {
				throw new WorkerCompileError(
					"invalid-html-fonts",
					`html.fonts[${index}] must be an object`,
				);
			}
			requireBytes(font.woff2, `html font ${index} woff2`);
			if (font.woff2.byteLength > limits.oneFileBytes) {
				throw workerLimitError(
					"one HTML font bytes",
					limits.oneFileBytes,
					font.woff2.byteLength,
				);
			}
			fontBytes += font.woff2.byteLength;
			if (fontBytes > limits.cachedFileBytes) {
				throw workerLimitError(
					"HTML font bytes",
					limits.cachedFileBytes,
					fontBytes,
				);
			}
			const woff2 = font.woff2.slice();
			transfer.push(woff2.buffer);
			return { ...font, encoding: font.encoding?.slice(), woff2 };
		});
		clonedOptions.html = { ...clonedOptions.html, fonts };
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
				concurrency: resolver.concurrency,
				format: resolver.format,
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
