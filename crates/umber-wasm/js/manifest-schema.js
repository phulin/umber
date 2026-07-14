const KEY_PATTERN = /^(tex|tfm):(.+)$/;
const DIGEST_PATTERN = /^[0-9a-f]{64}$/;
const FORMAT_NAME_PATTERN = /^[A-Za-z0-9._-]+$/;
const MAX_OBJECT_BYTES = 64 * 1024 * 1024;

export class ManifestResolverError extends Error {
	constructor(code, message, options) {
		super(message, options);
		this.name = "ManifestResolverError";
		this.code = code;
	}
}

export function validateManifest(value) {
	if (!isRecord(value) || value.schema !== 1 || !isRecord(value.files)) {
		throw new ManifestResolverError(
			"invalid-manifest",
			"manifest schema 1 is required",
		);
	}
	if (
		typeof value.distribution !== "string" ||
		value.distribution.length === 0
	) {
		throw new ManifestResolverError(
			"invalid-manifest",
			"distribution is required",
		);
	}
	let objectsBaseUrl;
	try {
		objectsBaseUrl = new URL(value.objectsBaseUrl).href;
	} catch (error) {
		throw new ManifestResolverError(
			"invalid-manifest",
			"objectsBaseUrl is invalid",
			{ cause: error },
		);
	}
	if (!objectsBaseUrl.endsWith("/")) {
		throw new ManifestResolverError(
			"invalid-manifest",
			"objectsBaseUrl must end with '/'",
		);
	}

	const files = Object.create(null);
	const fonts = Object.create(null);
	const formats = Object.create(null);
	const hashLengths = new Map();
	const pathObjects = new Map();
	for (const [key, entry] of Object.entries(value.files)) {
		validateKey(key);
		if (!isRecord(entry) || !DIGEST_PATTERN.test(entry.sha256)) {
			throw invalidManifest(`invalid entry for ${key}`);
		}
		validateObjectEntry(entry, key, hashLengths);
		if (!isCanonicalPath(entry.virtualPath, "/texlive/")) {
			throw invalidManifest(`invalid virtual path for ${key}`);
		}
		const dependencies = entry.dependencies ?? [];
		if (!Array.isArray(dependencies)) {
			throw invalidManifest(`invalid dependencies for ${key}`);
		}
		for (const dependency of dependencies) validateKey(dependency);
		const previousObject = pathObjects.get(entry.virtualPath);
		if (previousObject !== undefined && previousObject !== entry.sha256) {
			throw invalidManifest(
				`virtual path ${entry.virtualPath} has conflicting objects`,
			);
		}
		pathObjects.set(entry.virtualPath, entry.sha256);
		files[key] = Object.freeze({
			...entry,
			dependencies: Object.freeze([...dependencies]),
		});
	}
	const manifestFonts = value.fonts ?? {};
	if (!isRecord(manifestFonts)) {
		throw invalidManifest("fonts must be an object");
	}
	for (const [logicalName, entry] of Object.entries(manifestFonts)) {
		if (
			logicalName.length === 0 ||
			[...logicalName].some((character) => /\p{Cc}/u.test(character)) ||
			!isRecord(entry)
		) {
			throw invalidManifest(`invalid font entry for ${logicalName}`);
		}
		validateObjectEntry(entry, `font ${logicalName}`, hashLengths);
		if (
			entry.container !== "woff2" ||
			(entry.provenance !== undefined && typeof entry.provenance !== "string")
		) {
			throw invalidManifest(`invalid font metadata for ${logicalName}`);
		}
		fonts[logicalName] = Object.freeze({ ...entry });
	}

	const manifestFormats = value.formats ?? {};
	if (!isRecord(manifestFormats)) {
		throw invalidManifest("formats must be an object");
	}
	for (const [name, entry] of Object.entries(manifestFormats)) {
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
	for (const [key, entry] of Object.entries(files)) {
		for (const dependency of entry.dependencies) {
			if (files[dependency] === undefined) {
				throw invalidManifest(`dependency ${dependency} from ${key} is absent`);
			}
		}
	}
	return Object.freeze({
		schema: 1,
		distribution: value.distribution,
		objectsBaseUrl,
		files: Object.freeze(files),
		fonts: Object.freeze(fonts),
		formats: Object.freeze(formats),
	});
}

export function encodeRequest(request) {
	if (
		!isRecord(request) ||
		(request.kind !== "tex" && request.kind !== "tfm")
	) {
		throw new ManifestResolverError(
			"invalid-request",
			"request kind must be tex or tfm",
		);
	}
	const key = `${request.kind}:${request.name}`;
	validateKey(key);
	return key;
}

export function decodeKey(key) {
	const match = KEY_PATTERN.exec(key);
	return { kind: match[1], name: match[2] };
}

export function isFormatName(name) {
	return typeof name === "string" && FORMAT_NAME_PATTERN.test(name);
}

function validateObjectEntry(entry, label, hashLengths) {
	if (!DIGEST_PATTERN.test(entry.sha256)) {
		throw invalidManifest(`invalid digest for ${label}`);
	}
	if (entry.object !== `sha256-${entry.sha256}`) {
		throw invalidManifest(`invalid object name for ${label}`);
	}
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

function validateKey(key) {
	if (typeof key !== "string") {
		throw invalidManifest(`invalid lookup key ${String(key)}`);
	}
	const match = KEY_PATTERN.exec(key);
	if (match === null || !isCanonicalPath(match[2], "")) {
		throw invalidManifest(`invalid lookup key ${key}`);
	}
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
	) {
		return false;
	}
	return suffix
		.split("/")
		.every(
			(component) =>
				component !== "" && component !== "." && component !== "..",
		);
}
