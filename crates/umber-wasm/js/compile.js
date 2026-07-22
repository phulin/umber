const DEFAULT_LIMITS = Object.freeze({
	attempts: 32,
	userFiles: 512,
	resolvedFiles: 512,
	oneFileBytes: 96 * 1024 * 1024,
	cachedFileBytes: 64 * 1024 * 1024,
	userSourceBytes: 16 * 1024 * 1024,
	outputBytes: 64 * 1024 * 1024,
	engineFuel: 100_000_000,
	engineSteps: 10_000_000,
	inputFrames: 100_000,
	journalBytes: 256 * 1024 * 1024,
	effects: 1_000_000,
});

const HARD_LIMITS = Object.freeze({
	attempts: 128,
	userFiles: 4096,
	resolvedFiles: 4096,
	oneFileBytes: 128 * 1024 * 1024,
	cachedFileBytes: 256 * 1024 * 1024,
	userSourceBytes: 64 * 1024 * 1024,
	outputBytes: 256 * 1024 * 1024,
	engineFuel: 1_000_000_000,
	engineSteps: 100_000_000,
	inputFrames: 1_000_000,
	journalBytes: 1024 * 1024 * 1024,
	effects: 10_000_000,
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
			if (attempt?.kind === "complete") {
				const ledger = session.acceptedInputObservations;
				if (ledger !== undefined) {
					attempt.output.acceptedInputObservations = ledger;
				}
				return attempt.output;
			}
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

/** Creates a retained editor session whose hot pass and stabilization are explicit. */
export async function createEditorSession(
	options,
	userFiles,
	resolver,
	signal,
	bindings,
) {
	validateResolver(resolver);
	const limits = validateSessionLimits(options?.limits);
	throwIfAborted(signal);
	const module = bindings ?? (await import("./umber_wasm.js"));
	if (bindings === undefined) await module.default();
	if (typeof module?.EditorSession !== "function") {
		throw new CompileFacadeError(
			"invalid-binding",
			"EditorSession binding is unavailable",
		);
	}
	throwIfAborted(signal);
	const session = new module.EditorSession(options);
	try {
		addUserFiles(session, userFiles, limits);
		return new EditorCompileFacade(session, resolver);
	} catch (error) {
		session.dispose();
		throw error;
	}
}

export class EditorCompileFacade {
	#session;
	#resolver;
	#operation;

	constructor(session, resolver) {
		this.#session = session;
		this.#resolver = resolver;
	}

	get disposed() {
		return this.#session === undefined;
	}

	get status() {
		return this.#requireSession().status;
	}

	get revision() {
		return this.#requireSession().revision;
	}

	get contentHash() {
		return this.#requireSession().contentHash;
	}

	applyPatch(patch) {
		this.#requireSession().applyPatch(patch);
	}

	renderedSourceLocation(page, event, unit, outputId, revision) {
		return this.#requireSession().renderedSourceLocation(
			page,
			event,
			unit,
			outputId,
			revision,
		);
	}

	cancelPendingPatch() {
		const cancelled = this.#requireSession().cancelPendingPatch();
		if (cancelled && this.#operation?.phase === "advance") {
			this.#operation.controller.abort(new EditorOperationCancelled("advance"));
		}
		return cancelled;
	}

	cancelStabilization() {
		const cancelled = this.#requireSession().cancelStabilization();
		if (cancelled && this.#operation?.phase === "stabilization") {
			this.#operation.controller.abort(
				new EditorOperationCancelled("stabilization"),
			);
		}
		return cancelled;
	}

	async advance(signal, onProgress) {
		return this.#drive("advance", signal, onProgress);
	}

	async stabilize(signal, onProgress) {
		return this.#drive("stabilization", signal, onProgress);
	}

	dispose() {
		if (this.#session === undefined) return;
		this.#session.dispose();
		this.#session = undefined;
	}

	async #drive(phase, signal, onProgress) {
		const session = this.#requireSession();
		if (this.#operation !== undefined) {
			throw new CompileFacadeError(
				"operation-pending",
				"an editor operation is already pending",
			);
		}
		const controller = new AbortController();
		const onOwnerAbort = () => controller.abort(abortReason(signal));
		if (signal?.aborted) onOwnerAbort();
		else signal?.addEventListener("abort", onOwnerAbort, { once: true });
		this.#operation = { phase, controller };
		try {
			for (;;) {
				throwIfAborted(signal);
				const attempt =
					phase === "advance" ? session.advance() : session.stabilizeAttempt();
				if (attempt?.kind === "provisional" || attempt?.kind === "stable") {
					const ledger = session.acceptedInputObservations;
					if (ledger !== undefined) {
						attempt.output.acceptedInputObservations = ledger;
					}
					return attempt;
				}
				if (attempt?.kind === "error") {
					throw new CompileFacadeError(
						attempt.diagnostic?.code ?? "compile",
						attempt.diagnostic?.message ?? `${phase} failed`,
						{ diagnostic: attempt.diagnostic },
					);
				}
				if (
					attempt?.kind !== "need-resources" ||
					attempt.phase !== phase ||
					!Array.isArray(attempt.required) ||
					!Array.isArray(attempt.probes) ||
					!Array.isArray(attempt.prefetchHints)
				) {
					throw new CompileFacadeError(
						"invalid-binding",
						`${phase} returned an invalid result`,
					);
				}
				onProgress?.(attempt);
				const responses = await resolveBatch(
					this.#resolver,
					attempt,
					controller.signal,
				);
				throwIfAborted(signal);
				session.provideResources(responses);
			}
		} catch (error) {
			if (signal?.aborted) {
				this.dispose();
				throw abortReason(signal);
			}
			if (controller.signal.reason instanceof EditorOperationCancelled) {
				return {
					kind: "cancelled",
					phase,
					cancelled: true,
					status: session.status,
				};
			}
			throw error;
		} finally {
			signal?.removeEventListener("abort", onOwnerAbort);
			if (this.#operation?.controller === controller)
				this.#operation = undefined;
		}
	}

	#requireSession() {
		if (this.#session === undefined) {
			throw new CompileFacadeError(
				"disposed",
				"editor session has been disposed",
			);
		}
		return this.#session;
	}
}

class EditorOperationCancelled extends Error {
	constructor(phase) {
		super(`${phase} was cancelled`);
	}
}

async function resolveBatch(resolver, attempt, signal) {
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
			{ cause: error },
		);
	}
	if (!downloads || typeof downloads[Symbol.iterator] !== "function") {
		throw new CompileFacadeError(
			"invalid-resolver",
			"resolver must return an iterable",
		);
	}
	return [...downloads];
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
