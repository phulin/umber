const KEY_PATTERN = /^(tex|tfm|bib-aux|classic-bib|bst):(.+)$/;
const DIGEST_PATTERN = /^[0-9a-f]{64}$/;
const FORMAT_NAME_PATTERN = /^[A-Za-z0-9._-]+$/;
const MAX_OBJECT_BYTES = 128 * 1024 * 1024;
const MAX_SHARD_BITS = 16;

export class ManifestResolverError extends Error {
	constructor(code, message, options) {
		super(message, options);
		this.name = "ManifestResolverError";
		this.code = code;
	}
}

export function validateRootManifest(value) {
	if (!isRecord(value) || value.schema !== 2) {
		throw invalidManifest("root manifest schema 2 is required");
	}
	const distribution = validateDistribution(value.distribution);
	const objectsBaseUrl = validateObjectsBaseUrl(value.objectsBaseUrl);
	if (
		!Number.isInteger(value.shardBits) ||
		value.shardBits < 0 ||
		value.shardBits > MAX_SHARD_BITS
	) {
		throw invalidManifest(`shardBits must be between 0 and ${MAX_SHARD_BITS}`);
	}
	const expectedCount = 2 ** value.shardBits;
	if (
		value.shardCount !== expectedCount ||
		!Array.isArray(value.shards) ||
		value.shards.length !== expectedCount ||
		value.shards.some((digest) => !DIGEST_PATTERN.test(digest)) ||
		new Set(value.shards).size !== value.shards.length
	) {
		throw invalidManifest("root manifest shard metadata is inconsistent");
	}
	const formats = Object.create(null);
	const hashLengths = new Map();
	if (!isRecord(value.formats ?? {}))
		throw invalidManifest("formats must be an object");
	for (const [name, entry] of Object.entries(value.formats ?? {})) {
		if (!isFormatName(name) || !isRecord(entry)) {
			throw invalidManifest(`invalid format entry for ${name}`);
		}
		validateObjectEntry(entry, `format ${name}`, hashLengths);
		if (
			entry.engine !== "umber" ||
			typeof entry.engineVersion !== "string" ||
			entry.engineVersion.length === 0 ||
			!Number.isSafeInteger(entry.formatSchema) ||
			entry.formatSchema < 1 ||
			typeof entry.sourceDistribution !== "string" ||
			entry.sourceDistribution.length === 0 ||
			!DIGEST_PATTERN.test(entry.sourceManifestSha256) ||
			!Number.isSafeInteger(entry.sourceDateEpoch) ||
			entry.sourceDateEpoch < 0
		) {
			throw invalidManifest(
				`invalid compatibility metadata for format ${name}`,
			);
		}
		formats[name] = Object.freeze({ ...entry });
	}
	return Object.freeze({
		schema: 2,
		distribution,
		objectsBaseUrl,
		shardBits: value.shardBits,
		shardCount: value.shardCount,
		shards: Object.freeze([...value.shards]),
		formats: Object.freeze(formats),
	});
}

export function validateIndexShard(value, root, index) {
	if (
		!isRecord(value) ||
		value.schema !== 1 ||
		value.distribution !== root.distribution ||
		value.index !== index ||
		!isRecord(value.files)
	) {
		throw invalidManifest(
			`index shard ${index} identity does not match root manifest`,
		);
	}
	const files = Object.create(null);
	const hashLengths = new Map();
	const pathObjects = new Map();
	for (const [key, entry] of Object.entries(value.files)) {
		validateKey(key);
		if (!isRecord(entry)) throw invalidManifest(`invalid entry for ${key}`);
		validateFileEntry(entry, key, hashLengths, pathObjects);
		const dependencies = entry.dependencies ?? [];
		if (!Array.isArray(dependencies))
			throw invalidManifest(`invalid dependencies for ${key}`);
		let previous;
		const validatedDependencies = dependencies.map((dependency) => {
			if (!isRecord(dependency))
				throw invalidManifest(`invalid dependency from ${key}`);
			validateKey(dependency.key);
			if (previous !== undefined && previous >= dependency.key) {
				throw invalidManifest(
					`dependencies for ${key} are not strictly sorted`,
				);
			}
			previous = dependency.key;
			validateFileEntry(dependency, dependency.key, hashLengths, pathObjects);
			return Object.freeze({ ...dependency });
		});
		files[key] = Object.freeze({
			...entry,
			dependencies: Object.freeze(validatedDependencies),
		});
	}
	return Object.freeze({
		schema: 1,
		distribution: root.distribution,
		index,
		files: Object.freeze(files),
	});
}

export async function shardIndex(key, shardBits, crypto) {
	validateKey(key);
	if (
		!Number.isInteger(shardBits) ||
		shardBits < 0 ||
		shardBits > MAX_SHARD_BITS
	) {
		throw invalidManifest(`shardBits must be between 0 and ${MAX_SHARD_BITS}`);
	}
	if (shardBits === 0) return 0;
	if (!crypto?.subtle) {
		throw new ManifestResolverError(
			"invalid-options",
			"Web Crypto SubtleCrypto is required",
		);
	}
	const digest = new Uint8Array(
		await crypto.subtle.digest("SHA-256", new TextEncoder().encode(key)),
	);
	const prefix = (digest[0] << 8) | digest[1];
	return prefix >>> (16 - shardBits);
}

export function encodeRequest(request) {
	if (
		!isRecord(request) ||
		!["tex", "tfm", "bib-aux", "classic-bib-data", "bib-style"].includes(
			request.kind,
		)
	) {
		throw new ManifestResolverError(
			"invalid-request",
			"request kind is not supported by the distribution resolver",
		);
	}
	const kind = {
		"classic-bib-data": "classic-bib",
		"bib-style": "bst",
	}[request.kind] ?? request.kind;
	const key = `${kind}:${request.name}`;
	validateKey(key);
	return key;
}

export function decodeKey(key) {
	const match = KEY_PATTERN.exec(key);
	const kind = {
		"classic-bib": "classic-bib-data",
		bst: "bib-style",
	}[match[1]] ?? match[1];
	return { kind, name: match[2] };
}

export function resourceDomain(kind) {
	return kind === "tex" || kind === "tfm" ? "tex" : "bibliography";
}

export function isFormatName(name) {
	return typeof name === "string" && FORMAT_NAME_PATTERN.test(name);
}

export function fontRequestIdentity(request) {
	if (
		typeof request.logicalName !== "string" ||
		request.logicalName.length === 0 ||
		!Number.isSafeInteger(request.faceIndex) ||
		request.faceIndex < 0 ||
		!Array.isArray(request.variations) ||
		!Array.isArray(request.features)
	) {
		throw new ManifestResolverError("invalid-request", "invalid font request");
	}
	return `font:${request.logicalName}:${request.faceIndex}:${JSON.stringify(request.variations)}:${JSON.stringify(request.features)}`;
}

function validateFileEntry(entry, label, hashLengths, pathObjects) {
	validateObjectEntry(entry, label, hashLengths);
	if (!isCanonicalPath(entry.virtualPath, "/texlive/")) {
		throw invalidManifest(`invalid virtual path for ${label}`);
	}
	const previousObject = pathObjects.get(entry.virtualPath);
	if (previousObject !== undefined && previousObject !== entry.sha256) {
		throw invalidManifest(
			`virtual path ${entry.virtualPath} has conflicting objects`,
		);
	}
	pathObjects.set(entry.virtualPath, entry.sha256);
}

function validateObjectEntry(entry, label, hashLengths) {
	if (!DIGEST_PATTERN.test(entry.sha256))
		throw invalidManifest(`invalid digest for ${label}`);
	if (entry.object !== `sha256-${entry.sha256}`)
		throw invalidManifest(`invalid object name for ${label}`);
	if (
		!Number.isSafeInteger(entry.bytes) ||
		entry.bytes < 0 ||
		entry.bytes > MAX_OBJECT_BYTES
	) {
		throw invalidManifest(`invalid byte length for ${label}`);
	}
	const previousLength = hashLengths.get(entry.sha256);
	if (previousLength !== undefined && previousLength !== entry.bytes) {
		throw invalidManifest(
			`inconsistent byte lengths for digest ${entry.sha256}`,
		);
	}
	hashLengths.set(entry.sha256, entry.bytes);
}

function validateDistribution(value) {
	if (typeof value !== "string" || value.length === 0)
		throw invalidManifest("distribution is required");
	return value;
}

function validateObjectsBaseUrl(value) {
	let url;
	try {
		url = new URL(value).href;
	} catch (error) {
		throw new ManifestResolverError(
			"invalid-manifest",
			"objectsBaseUrl is invalid",
			{ cause: error },
		);
	}
	if (!url.endsWith("/"))
		throw invalidManifest("objectsBaseUrl must end with '/'");
	return url;
}

function validateKey(key) {
	if (typeof key !== "string")
		throw invalidManifest(`invalid lookup key ${String(key)}`);
	const match = KEY_PATTERN.exec(key);
	if (match === null || !isCanonicalPath(match[2], ""))
		throw invalidManifest(`invalid lookup key ${key}`);
}

function invalidManifest(message) {
	return new ManifestResolverError("invalid-manifest", message);
}

function isRecord(value) {
	return value !== null && typeof value === "object" && !Array.isArray(value);
}

function isCanonicalPath(value, prefix) {
	if (typeof value !== "string" || !value.startsWith(prefix)) return false;
	const suffix = value.slice(prefix.length);
	if (
		suffix.length === 0 ||
		suffix.includes("\\") ||
		suffix.includes("\0") ||
		suffix.includes(":")
	)
		return false;
	return suffix
		.split("/")
		.every(
			(component) =>
				component !== "" && component !== "." && component !== "..",
		);
}
