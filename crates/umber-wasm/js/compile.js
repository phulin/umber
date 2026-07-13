const DEFAULT_LIMITS = Object.freeze({
	attempts: 32,
	resolvedFiles: 512,
	oneFileBytes: 16 * 1024 * 1024,
	cachedFileBytes: 64 * 1024 * 1024,
	userSourceBytes: 16 * 1024 * 1024,
	outputBytes: 64 * 1024 * 1024,
});

const HARD_LIMITS = Object.freeze({
	attempts: 128,
	resolvedFiles: 4096,
	oneFileBytes: 64 * 1024 * 1024,
	cachedFileBytes: 256 * 1024 * 1024,
	userSourceBytes: 64 * 1024 * 1024,
	outputBytes: 256 * 1024 * 1024,
});

export class CompileFacadeError extends Error {
	constructor(code, message, options = {}) {
		super(message, { cause: options.cause });
		this.name = "CompileFacadeError";
		this.code = code;
		if (options.diagnostic !== undefined) this.diagnostic = options.diagnostic;
	}
}

export async function compile(options, userFiles, resolver, signal, bindings) {
	validateResolver(resolver);
	const limits = validateLimits(options?.limits);
	throwIfAborted(signal);
	const CompilerSession = await compilerClass(bindings);
	throwIfAborted(signal);
	const session = new CompilerSession(options);
	const provided = new Map();
	const providedPaths = new Map();
	let providedBytes = 0;
	try {
		addUserFiles(session, userFiles, limits);
		for (let round = 0; round < limits.attempts; round += 1) {
			throwIfAborted(signal);
			const attempt = session.compileAttempt();
			if (attempt?.kind === "complete") return attempt.output;
			if (attempt?.kind === "error") {
				throw new CompileFacadeError(
					"compile",
					attempt.diagnostic?.message ?? "compile failed",
					{
						diagnostic: attempt.diagnostic,
					},
				);
			}
			if (attempt?.kind !== "need-files" || !Array.isArray(attempt.files)) {
				throw new CompileFacadeError(
					"invalid-binding",
					"compileAttempt returned an invalid result",
				);
			}
			const requested = new Set(attempt.files.map(requestKey));
			if (requested.size === 0) {
				throw new CompileFacadeError(
					"no-progress",
					"compile requested no files",
				);
			}
			throwIfAborted(signal);
			let downloads;
			try {
				downloads = await resolver.resolve(attempt.files, signal);
			} catch (error) {
				if (signal?.aborted) throw abortReason(signal);
				throw new CompileFacadeError(
					"resolve",
					`file resolution failed: ${errorMessage(error)}`,
					{
						cause: error,
					},
				);
			}
			throwIfAborted(signal);
			if (!downloads || typeof downloads[Symbol.iterator] !== "function") {
				throw new CompileFacadeError(
					"invalid-resolver",
					"resolver must return an iterable",
				);
			}
			let progressed = false;
			for (const download of downloads) {
				const validated = validateDownload(download, limits);
				const key = requestKey(validated.request);
				const previous = provided.get(key);
				if (
					previous !== undefined &&
					(previous.virtualPath !== validated.virtualPath ||
						!equalBytes(previous.bytes, validated.bytes))
				) {
					throw new CompileFacadeError(
						"conflicting-download",
						`${key} resolved to conflicting bytes`,
					);
				}
				if (previous === undefined) {
					if (provided.size + 1 > limits.resolvedFiles) {
						throw limitError(
							"resolved files",
							limits.resolvedFiles,
							provided.size + 1,
						);
					}
					const previousPath = providedPaths.get(validated.virtualPath);
					if (
						previousPath !== undefined &&
						!equalBytes(previousPath, validated.bytes)
					) {
						throw new CompileFacadeError(
							"conflicting-download",
							`${validated.virtualPath} resolved to conflicting bytes`,
						);
					}
					if (previousPath === undefined) {
						providedBytes = checkedAdd(
							providedBytes,
							validated.bytes.byteLength,
						);
						if (providedBytes > limits.cachedFileBytes) {
							throw limitError(
								"cached file bytes",
								limits.cachedFileBytes,
								providedBytes,
							);
						}
					}
					session.provideResolvedFile(
						validated.request,
						validated.virtualPath,
						validated.bytes,
					);
					provided.set(key, {
						virtualPath: validated.virtualPath,
						bytes: validated.bytes,
					});
					providedPaths.set(validated.virtualPath, validated.bytes);
					if (requested.has(key)) progressed = true;
				}
			}
			if (!progressed) {
				throw new CompileFacadeError(
					"no-progress",
					`resolver did not provide any of: ${[...requested].join(", ")}`,
				);
			}
		}
		throw new CompileFacadeError(
			"attempt-limit",
			`compile attempt limit ${limits.attempts} reached`,
		);
	} finally {
		session.dispose();
	}
}

async function compilerClass(bindings) {
	const module = bindings ?? (await import("./umber_wasm.js"));
	if (bindings === undefined) await module.default();
	if (typeof module?.CompilerSession !== "function") {
		throw new CompileFacadeError(
			"invalid-binding",
			"CompilerSession binding is unavailable",
		);
	}
	return module.CompilerSession;
}

function addUserFiles(session, userFiles, limits) {
	if (!userFiles || typeof userFiles[Symbol.iterator] !== "function") {
		throw new CompileFacadeError(
			"invalid-options",
			"userFiles must be an iterable map",
		);
	}
	let total = 0;
	for (const item of userFiles) {
		if (
			!Array.isArray(item) ||
			item.length !== 2 ||
			typeof item[0] !== "string"
		) {
			throw new CompileFacadeError(
				"invalid-options",
				"userFiles entries must be [path, Uint8Array]",
			);
		}
		const [path, bytes] = item;
		requireBytes(bytes, `user file ${path}`);
		if (bytes.byteLength > limits.oneFileBytes) {
			throw limitError(
				"one user file bytes",
				limits.oneFileBytes,
				bytes.byteLength,
			);
		}
		total = checkedAdd(total, bytes.byteLength);
		if (total > limits.userSourceBytes) {
			throw limitError("user source bytes", limits.userSourceBytes, total);
		}
		session.addUserFile(path, bytes);
	}
}

function validateDownload(download, limits) {
	if (!download || typeof download !== "object") {
		throw new CompileFacadeError(
			"invalid-resolver",
			"resolver returned a non-object download",
		);
	}
	requestKey(download.request);
	if (typeof download.virtualPath !== "string") {
		throw new CompileFacadeError(
			"invalid-resolver",
			"resolved virtualPath must be a string",
		);
	}
	requireBytes(download.bytes, "resolved bytes");
	if (download.bytes.byteLength > limits.oneFileBytes) {
		throw limitError(
			"one resolved file bytes",
			limits.oneFileBytes,
			download.bytes.byteLength,
		);
	}
	return download;
}

function requestKey(request) {
	if (
		!request ||
		(request.kind !== "tex" && request.kind !== "tfm") ||
		typeof request.name !== "string" ||
		request.name.length === 0
	) {
		throw new CompileFacadeError(
			"invalid-resolver",
			"invalid file request key",
		);
	}
	return `${request.kind}:${request.name}`;
}

function validateResolver(resolver) {
	if (!resolver || typeof resolver.resolve !== "function") {
		throw new CompileFacadeError(
			"invalid-options",
			"resolver.resolve is required",
		);
	}
}

function validateLimits(partial = {}) {
	const limits = { ...DEFAULT_LIMITS, ...(partial ?? {}) };
	for (const [name, hard] of Object.entries(HARD_LIMITS)) {
		const value = limits[name];
		if (!Number.isSafeInteger(value) || value < 0 || value > hard) {
			throw new CompileFacadeError(
				"invalid-options",
				`${name} must be an integer from 0 through ${hard}`,
			);
		}
	}
	return limits;
}

function requireBytes(value, label) {
	if (!(value instanceof Uint8Array)) {
		throw new CompileFacadeError(
			"invalid-options",
			`${label} must be a Uint8Array`,
		);
	}
}

function checkedAdd(left, right) {
	const total = left + right;
	if (!Number.isSafeInteger(total)) {
		throw new CompileFacadeError(
			"limit",
			"byte accounting exceeded JavaScript safe integers",
		);
	}
	return total;
}

function limitError(resource, limit, attempted) {
	return new CompileFacadeError(
		"limit",
		`${resource} requires ${attempted}, exceeding limit ${limit}`,
	);
}

function throwIfAborted(signal) {
	if (signal?.aborted) throw abortReason(signal);
}

function abortReason(signal) {
	return (
		signal.reason ?? new DOMException("The operation was aborted", "AbortError")
	);
}

function errorMessage(error) {
	return error instanceof Error ? error.message : String(error);
}

function equalBytes(left, right) {
	if (left.byteLength !== right.byteLength) return false;
	return left.every((byte, index) => byte === right[index]);
}
