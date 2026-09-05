import { encodeRequest, resourceDomain } from "./manifest-schema.js";

/** Shared names for the three states used by catalog/VFS adapters. */
export const ResourceReadiness = Object.freeze({
	Ready: "ready",
	ExistsNotReady: "exists-not-ready",
	Absent: "absent",
});

export const PREFETCH_POLICY_VERSION = "literal-groups-v1";

export function makePrefetchIdentity({
	engine = "tex82",
	format = "none",
	options = "{}",
	distribution,
	searchPolicy = PREFETCH_POLICY_VERSION,
} = {}) {
	const values = { engine, format, options, distribution, searchPolicy };
	for (const [name, value] of Object.entries(values)) {
		if (typeof value !== "string" || value.length === 0)
			throw new TypeError(`${name} must be a non-empty string`);
	}
	return Object.freeze({
		...values,
		canonicalKey: [
			values.engine,
			values.format,
			values.options,
			values.distribution,
			values.searchPolicy,
		].map(escapeIdentity).join("|"),
	});
}

/** Accepted lookup history for one source-independent execution identity. */
export class LookupManifest {
	constructor(identity, records = []) {
		this.identity = identity;
		this.records = [];
		for (const record of records) this.record(record);
	}

	record(record) {
		if (!record || typeof record !== "object") return false;
		const duplicate = this.records.find(
			(existing) =>
				existing.requestKey === record.requestKey &&
				existing.resourceKind === record.resourceKind &&
				existing.outcome?.kind === record.outcome?.kind,
		);
		if (duplicate) {
			if (roleRank(record.role) > roleRank(duplicate.role))
				duplicate.role = record.role;
			return false;
		}
		this.records.push(structuredRecord(record));
		return true;
	}

	resolvedRequests() {
		return this.records
			.filter((record) => record.outcome?.kind === "resolved")
			.map((record) => {
				const name = record.requestKey.slice(record.requestKey.indexOf(":") + 1);
				return {
					type: "file",
					domain: resourceDomain(record.resourceKind),
					kind: record.resourceKind,
					name,
					originalName: record.originalSpelling,
				};
			});
	}

	encode() {
		return new TextEncoder().encode(
			JSON.stringify({
				schema: 1,
				identity: {
					engine: this.identity.engine,
					format: this.identity.format,
					options: this.identity.options,
					distribution: this.identity.distribution,
					searchPolicy: this.identity.searchPolicy,
				},
				records: this.records,
			}) + "\n",
		);
	}

	static decode(bytes, identity) {
		if (!(bytes instanceof Uint8Array)) return undefined;
		let value;
		try {
			value = JSON.parse(new TextDecoder("utf-8", { fatal: true }).decode(bytes));
		} catch {
			return undefined;
		}
		if (
			value?.schema !== 1 ||
			JSON.stringify(value.identity) !==
				JSON.stringify({
					engine: identity.engine,
					format: identity.format,
					options: identity.options,
					distribution: identity.distribution,
					searchPolicy: identity.searchPolicy,
				}) ||
			!Array.isArray(value.records)
		)
			return undefined;
		return new LookupManifest(identity, value.records);
	}
}

export function prefetchManifestCacheKey(identity) {
	return stableHex64(identity.canonicalKey);
}

const DEFAULT_MAX_HINTS = 256;
const DEFAULT_MAX_NAME_BYTES = 1024;

/**
 * Distinguishes admitted bytes from catalog evidence.  Undefined means that
 * no authoritative semantic answer exists yet (for example, a transport or
 * access failure); callers must not persist that state as absence.
 */
export function classifyReadiness({
	exists = undefined,
	payloadAdmitted = false,
	authoritativeAbsent = false,
} = {}) {
	if (authoritativeAbsent) return ResourceReadiness.Absent;
	if (payloadAdmitted) return ResourceReadiness.Ready;
	if (exists === true) return ResourceReadiness.ExistsNotReady;
	return undefined;
}

/**
 * Lexically extracts only literal LaTeX lookup arguments.  This intentionally
 * mirrors the native scanner: comments, malformed delimiters, control
 * sequences, and dynamic arguments are ignored without interpreting TeX.
 */
export function extractLiteralHints(source, limits = {}) {
	if (typeof source !== "string") return [];
	const maxHints = limits.maxHints ?? DEFAULT_MAX_HINTS;
	const maxNameBytes = limits.maxNameBytes ?? DEFAULT_MAX_NAME_BYTES;
	if (!Number.isSafeInteger(maxHints) || maxHints <= 0) return [];
	if (!Number.isSafeInteger(maxNameBytes) || maxNameBytes <= 0) return [];
	const hints = [];
	let index = 0;
	while (index < source.length && hints.length < maxHints) {
		const slash = source.indexOf("\\", index);
		if (slash < 0) break;
		if (inComment(source, slash)) {
			const newline = source.indexOf("\n", slash);
			index = newline < 0 ? source.length : newline;
			continue;
		}
		index = slash + 1;
		if (!/[A-Za-z]/.test(source[index] ?? "")) continue;
		const commandStart = index;
		while (/[A-Za-z]/.test(source[index] ?? "")) index += 1;
		const command = source.slice(commandStart, index);
		const kind =
			command === "documentclass"
				? "documentclass"
				: command === "usepackage" || command === "RequirePackage"
					? "package"
					: command === "input" || command === "include"
						? "input"
						: command === "includegraphics"
							? "includegraphics"
							: undefined;
		if (kind === undefined) continue;
		let cursor = skipHorizontalSpace(source, index);
		if (
			(kind === "documentclass" ||
				kind === "package" ||
				kind === "includegraphics") &&
			source[cursor] === "["
		) {
			const end = balanced(source, cursor, "[", "]");
			if (end === undefined) continue;
			cursor = skipHorizontalSpace(source, end);
		}
		const argument = literalArgument(source, cursor, maxNameBytes);
		if (argument === undefined) continue;
		for (const spelling of argument.value.split(",")) {
			const name = spelling.trim();
			if (
				name.length === 0 ||
				new TextEncoder().encode(name).byteLength > maxNameBytes ||
				/[\u0000-\u001f\u007f]/.test(name)
			)
				continue;
			hints.push({
				kind,
				originalSpelling: name,
				name,
				byteOffset: slash,
			});
			if (hints.length >= maxHints) break;
		}
		index = argument.end;
	}
	return hints;
}

/** Converts a lexical hint to the distribution-facing typed request. */
export function literalHintRequest(hint) {
	if (!hint || typeof hint.name !== "string") return undefined;
	const kind = hint.kind === "includegraphics" ? "image" : "tex";
	return {
		type: "file",
		domain: resourceDomain(kind),
		kind,
		name: hint.name,
		originalName: hint.originalSpelling ?? hint.name,
	};
}

/**
 * Catalog key identity for a typed request.  The semantic kind is retained so
 * an image and a TeX input with the same spelling never alias in the VFS.
 */
export function typedRequestIdentity(request) {
	if (request?.type === "file") {
		return JSON.stringify([
			"file",
			request.domain ?? resourceDomain(request.kind),
			request.kind,
			request.name,
		]);
	}
	try {
		return `${request?.type ?? "unknown"}:${encodeRequest(request)}`;
	} catch {
		return JSON.stringify(request);
	}
}

function skipHorizontalSpace(source, index) {
	while (index < source.length && /[ \t\r\n]/.test(source[index])) index += 1;
	return index;
}

function balanced(source, start, open, close) {
	let depth = 0;
	for (let index = start; index < source.length; index += 1) {
		if (source[index] === "%" && !isEscaped(source, index)) {
			const newline = source.indexOf("\n", index);
			if (newline < 0) return undefined;
			index = newline;
			continue;
		}
		if (source[index] === open) depth += 1;
		else if (source[index] === close && --depth === 0) return index + 1;
		if (depth < 0) return undefined;
	}
	return undefined;
}

function literalArgument(source, start, maxNameBytes) {
	if (source[start] === "{") {
		const end = balanced(source, start, "{", "}");
		if (end === undefined) return undefined;
		const value = source.slice(start + 1, end - 1);
		return new TextEncoder().encode(value).byteLength <= maxNameBytes
			? { value, end }
			: undefined;
	}
	let end = start;
	while (
		end < source.length &&
		!/[\s%\\]/.test(source[end])
	)
		end += 1;
	const value = source.slice(start, end);
	return value.length > 0 && new TextEncoder().encode(value).byteLength <= maxNameBytes
		? { value, end }
		: undefined;
}

function isEscaped(source, index) {
	let count = 0;
	for (let cursor = index - 1; cursor >= 0 && source[cursor] === "\\"; cursor -= 1)
		count += 1;
	return count % 2 === 1;
}

function inComment(source, index) {
	const lineStart = source.lastIndexOf("\n", index - 1) + 1;
	for (let cursor = lineStart; cursor < index; cursor += 1)
		if (source[cursor] === "%" && !isEscaped(source, cursor)) return true;
	return false;
}

function roleRank(role) {
	return role === "required" ? 3 : role === "probe" ? 2 : 1;
}

function structuredRecord(record) {
	return {
		originalSpelling: String(record.originalSpelling ?? ""),
		requestKey: String(record.requestKey ?? ""),
		resourceKind: String(record.resourceKind ?? "tex"),
		searchContext: String(record.searchContext ?? "distribution"),
		role: record.role === "required" || record.role === "probe" ? record.role : "hint",
		outcome:
			record.outcome?.kind === "resolved"
				? {
					kind: "resolved",
					manifestKey: String(record.outcome.manifestKey ?? record.requestKey),
					virtualPath: record.outcome.virtualPath,
					object: String(record.outcome.object ?? ""),
					ahash64: String(record.outcome.ahash64 ?? ""),
					bytes: Number(record.outcome.bytes ?? 0),
				}
				: { kind: "absent", scope: String(record.outcome?.scope ?? "") },
	};
}

function escapeIdentity(value) {
	return encodeURIComponent(value);
}

function stableHex64(value) {
	let hash = 0xcbf29ce484222325n;
	for (const byte of new TextEncoder().encode(value)) {
		hash ^= BigInt(byte);
		hash = BigInt.asUintN(64, hash * 0x100000001b3n);
	}
	return hash.toString(16).padStart(16, "0");
}
