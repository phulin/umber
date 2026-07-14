const DEFAULT_LIMITS = Object.freeze({
	attempts: 32,
	userFiles: 512,
	resolvedFiles: 512,
	oneFileBytes: 16 * 1024 * 1024,
	cachedFileBytes: 64 * 1024 * 1024,
	userSourceBytes: 16 * 1024 * 1024,
	outputBytes: 64 * 1024 * 1024,
});

const HARD_LIMITS = Object.freeze({
	attempts: 128,
	userFiles: 4096,
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
	const limits = validateSessionLimits(options?.limits);
	throwIfAborted(signal);
	const CompilerSession = await compilerClass(bindings);
	throwIfAborted(signal);
	const session = new CompilerSession(options);
	const provided = new Map();
	const providedPaths = new Map();
	let providedBytes = 0;
	try {
		addUserFiles(session, userFiles, limits);
		addHtmlFonts(session, options?.html?.fonts);
		for (let round = 0; round < limits.attempts; round += 1) {
			throwIfAborted(signal);
			const attempt =
				typeof session.advance === "function"
					? session.advance()
					: session.compileAttempt();
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
			if (
				attempt?.kind !== "need-resources" ||
				!Array.isArray(attempt.required) ||
				!Array.isArray(attempt.prefetchHints)
			) {
				throw new CompileFacadeError(
					"invalid-binding",
					"compileAttempt returned an invalid result",
				);
			}
			const requested = new Set(attempt.required.map(resourceKey));
			if (requested.size === 0) {
				throw new CompileFacadeError(
					"no-progress",
					"compile requested no resources",
				);
			}
			throwIfAborted(signal);
			let downloads;
			try {
				downloads = await resolver.resolve(attempt.required, {
					signal,
					prefetchHints: attempt.prefetchHints,
				});
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
			const accepted = [];
			for (const download of downloads) {
				const validated = validateResourceResponse(download, limits);
				const key = resourceKey(validated);
				const previous = provided.get(key);
				if (
					previous !== undefined &&
					!equalResourceResponse(previous, validated)
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
					const previousPath =
						validated.type === "file"
							? providedPaths.get(validated.virtualPath)
							: undefined;
					if (validated.type === "file" && previousPath !== undefined &&
						!equalBytes(previousPath, validated.bytes)) {
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
					accepted.push(validated);
					provided.set(key, validated);
					if (validated.type === "file") {
						providedPaths.set(validated.virtualPath, validated.bytes);
					}
					if (requested.has(key)) progressed = true;
				}
			}
			if (accepted.length > 0) session.provideResources(accepted);
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

function addHtmlFonts(session, fonts) {
	if (fonts === undefined) return;
	if (!Array.isArray(fonts)) {
		throw new CompileFacadeError(
			"invalid-html-fonts",
			"html.fonts must be an array",
		);
	}
	for (const font of fonts) session.addHtmlFont(font);
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
	let count = 0;
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
		count += 1;
		if (count > limits.userFiles) {
			throw limitError("user files", limits.userFiles, count);
		}
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

function validateResourceResponse(download, limits) {
	if (!download || typeof download !== "object") {
		throw new CompileFacadeError(
			"invalid-resolver",
			"resolver returned a non-object download",
		);
	}
	const response = normalizeResourceResponse(download);
	resourceKey(response);
	if (response.type === "file" && typeof response.virtualPath !== "string") {
		throw new CompileFacadeError(
			"invalid-resolver",
			"resolved virtualPath must be a string",
		);
	}
	requireBytes(response.bytes, "resolved bytes");
	if (response.bytes.byteLength > limits.oneFileBytes) {
		throw limitError(
			"one resolved file bytes",
			limits.oneFileBytes,
			response.bytes.byteLength,
		);
	}
	if (response.type === "font") {
		if (response.container !== "woff2") {
			throw new CompileFacadeError("invalid-resolver", "WASM fonts must use WOFF2");
		}
		for (const field of ["objectSha256", "programIdentity"]) {
			if (response[field] !== undefined && !/^[0-9a-f]{64}$/.test(response[field])) {
				throw new CompileFacadeError("invalid-resolver", `${field} must be 64 lowercase hex digits`);
			}
		}
		if (response.provenance !== undefined && typeof response.provenance !== "string") {
			throw new CompileFacadeError("invalid-resolver", "font provenance must be a string");
		}
	}
	return response;
}

function normalizeResourceResponse(response) {
	if (response.type === "file" || response.type === "font") return response;
	if (response.request !== undefined) {
		return { type: "file", ...response.request, virtualPath: response.virtualPath, bytes: response.bytes };
	}
	return response;
}

function resourceKey(request) {
	if (request?.type === "font") {
		if (
			typeof request.logicalName !== "string" ||
			request.logicalName.length === 0 ||
			!Number.isSafeInteger(request.faceIndex) ||
			request.faceIndex < 0 ||
			!Array.isArray(request.variations) ||
			!Array.isArray(request.features)
		) {
			throw new CompileFacadeError("invalid-resolver", "invalid font request key");
		}
		const variations = request.variations.map(({ tag, value }) => `${tag}:${value}`).join(",");
		const features = request.features.map(({ tag, enabled }) => `${tag}:${enabled}`).join(",");
		return `font:${request.logicalName}:${request.faceIndex}:${variations}:${features}`;
	}
	if (
		!request ||
		request.type !== "file" ||
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

function equalResourceResponse(left, right) {
	if (left.type !== right.type || !equalBytes(left.bytes, right.bytes)) return false;
	if (left.type === "file") return left.virtualPath === right.virtualPath;
	return left.container === right.container &&
		left.objectSha256 === right.objectSha256 &&
		left.programIdentity === right.programIdentity &&
		left.provenance === right.provenance;
}

function validateResolver(resolver) {
	if (!resolver || typeof resolver.resolve !== "function") {
		throw new CompileFacadeError(
			"invalid-options",
			"resolver.resolve is required",
		);
	}
}

export function validateSessionLimits(partial = {}) {
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
