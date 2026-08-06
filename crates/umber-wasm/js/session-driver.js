/** Internal driver for every authored facade over a retained WASM session. */
export class SessionDriver {
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

	get session() {
		if (this.#session === undefined) {
			throw new SessionDriverError("disposed", "session has been disposed");
		}
		return this.#session;
	}

	async drive({
		phase,
		attempt,
		isComplete,
		signal,
		onProgress,
		attemptLimit,
		pendingMessage = "a session operation is already pending",
	}) {
		const session = this.session;
		if (this.#operation !== undefined) {
			throw new SessionDriverError("operation-pending", pendingMessage);
		}
		const controller = new AbortController();
		const onOwnerAbort = () => controller.abort(abortReason(signal));
		if (signal?.aborted) onOwnerAbort();
		else signal?.addEventListener("abort", onOwnerAbort, { once: true });
		this.#operation = { phase, controller };

		try {
			for (
				let round = 0;
				attemptLimit === undefined || round < attemptLimit;
				round += 1
			) {
				throwIfAborted(controller.signal);
				const result = attempt(session);
				if (isComplete(result)) return attachLedger(session, result);
				if (result?.kind === "error") throw diagnosticError(result, phase);
				validateResourceWait(result, phase);
				onProgress?.(result);
				const responses = await resolveBatch(
					this.#resolver,
					result,
					controller.signal,
				);
				throwIfAborted(controller.signal);
				try {
					session.provideResources(responses);
				} catch (error) {
					throw new SessionDriverError(
						error?.code ?? "resource",
						errorMessage(error),
						{ cause: error },
					);
				}
			}
			throw new SessionDriverError(
				"attempt-limit",
				`compile attempt limit ${attemptLimit} reached`,
			);
		} catch (error) {
			if (controller.signal.reason instanceof SessionOperationCancelled) {
				return {
					kind: "cancelled",
					phase,
					cancelled: true,
					status: session.status,
				};
			}
			if (signal?.aborted) {
				this.dispose();
				throw abortReason(signal);
			}
			throw error;
		} finally {
			signal?.removeEventListener("abort", onOwnerAbort);
			if (this.#operation?.controller === controller) {
				this.#operation = undefined;
			}
		}
	}

	cancel(phase) {
		if (this.#operation?.phase !== phase) return;
		this.#operation.controller.abort(new SessionOperationCancelled(phase));
	}

	dispose() {
		if (this.#session === undefined) return;
		this.#operation?.controller.abort(
			new SessionDriverError("disposed", "session has been disposed"),
		);
		this.#session.dispose();
		this.#session = undefined;
	}
}

export class SessionDriverError extends Error {
	constructor(code, message, options = {}) {
		super(message, { cause: options.cause });
		this.name = "CompileFacadeError";
		this.code = code;
		if (options.diagnostic !== undefined) this.diagnostic = options.diagnostic;
	}
}

class SessionOperationCancelled extends Error {
	constructor(phase) {
		super(`${phase} was cancelled`);
	}
}

function attachLedger(session, result) {
	const ledger = session.acceptedInputObservations;
	if (ledger !== undefined) result.output.acceptedInputObservations = ledger;
	return result;
}

function diagnosticError(result, phase) {
	return new SessionDriverError(
		result.diagnostic?.code ?? "compile",
		result.diagnostic?.message ?? `${phase} failed`,
		{ diagnostic: result.diagnostic },
	);
}

function validateResourceWait(result, phase) {
	if (
		result?.kind !== "need-resources" ||
		!Array.isArray(result.required) ||
		!Array.isArray(result.probes) ||
		!Array.isArray(result.prefetchHints) ||
		(phase !== "compile" && result.phase !== phase)
	) {
		throw new SessionDriverError(
			"invalid-binding",
			`${phase === "compile" ? "compileAttempt" : phase} returned an invalid result`,
		);
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
		if (signal.aborted) throw signal.reason;
		throw new SessionDriverError(
			"resolve",
			`file resolution failed: ${errorMessage(error)}`,
			{ cause: error },
		);
	}
	if (!downloads || typeof downloads[Symbol.iterator] !== "function") {
		throw new SessionDriverError(
			"invalid-resolver",
			"resolver must return an iterable",
		);
	}
	return [...downloads];
}

function throwIfAborted(signal) {
	if (signal.aborted) throw signal.reason;
}

function abortReason(signal) {
	return (
		signal.reason ?? new DOMException("The operation was aborted", "AbortError")
	);
}

function errorMessage(error) {
	return error instanceof Error ? error.message : String(error);
}
