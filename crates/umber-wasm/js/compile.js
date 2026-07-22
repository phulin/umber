const DEFAULT_LIMITS = Object.freeze({
	attempts: 32,
	userFiles: 512,
	resolvedFiles: 512,
	oneFileBytes: 96 * 1024 * 1024,
	cachedFileBytes: 64 * 1024 * 1024,
	userSourceBytes: 16 * 1024 * 1024,
	outputBytes: 64 * 1024 * 1024,
});

const HARD_LIMITS = Object.freeze({
	attempts: 128,
	userFiles: 4096,
	resolvedFiles: 4096,
	oneFileBytes: 128 * 1024 * 1024,
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
	const Session = await sessionClass(
		bindings,
		options?.bibliography !== undefined,
	);
	throwIfAborted(signal);
	const session = new Session(options);
	try {
		addUserFiles(session, userFiles, limits);
		for (let round = 0; round < limits.attempts; round += 1) {
			throwIfAborted(signal);
			const attempt =
				typeof session.advance === "function"
					? session.advance()
					: session.compileAttempt();
			if (attempt?.kind === "complete") return attempt.output;
			if (attempt?.kind === "error") {
				throw new CompileFacadeError(
					attempt.diagnostic?.code ?? "compile",
					attempt.diagnostic?.message ?? "compile failed",
					{
						diagnostic: attempt.diagnostic,
					},
				);
			}
			if (
				attempt?.kind !== "need-resources" ||
				!Array.isArray(attempt.required) ||
				!Array.isArray(attempt.probes) ||
				!Array.isArray(attempt.prefetchHints)
			) {
				throw new CompileFacadeError(
					"invalid-binding",
					"compileAttempt returned an invalid result",
				);
			}
			throwIfAborted(signal);
			let downloads;
			try {
				downloads = await resolver.resolve(attempt.required, {
					signal,
					probes: attempt.probes,
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
			const responses = [...downloads];
			try {
				session.provideResources(responses);
			} catch (error) {
				throw new CompileFacadeError(
					error?.code ?? "resource",
					errorMessage(error),
					{ cause: error },
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

async function sessionClass(bindings, project) {
	const module = bindings ?? (await import("./umber_wasm.js"));
	if (bindings === undefined) await module.default();
	const Session = project ? module?.ProjectSession : module?.CompilerSession;
	if (typeof Session !== "function") {
		throw new CompileFacadeError(
			"invalid-binding",
			`${project ? "ProjectSession" : "CompilerSession"} binding is unavailable`,
		);
	}
	return Session;
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
