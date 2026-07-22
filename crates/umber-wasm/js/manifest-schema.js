const KEY_PATTERN = /^(tex|tfm|bib-aux|classic-bib|bst):(.+)$/;
const DIGEST_PATTERN = /^[0-9a-f]{64}$/;
const FORMAT_NAME_PATTERN = /^[A-Za-z0-9._-]+$/;
const MAX_OBJECT_BYTES = 128 * 1024 * 1024;
const MAX_SHARD_BITS = 16;
const MAX_FORMAT_INPUTS = 256;
const MAX_REQUEST_KEY_BYTES = 1024;
const MAX_FONT_KEY_BYTES = 4096;
const MAX_METADATA_BYTES = 4096;
const MAX_LICENSE_BYTES = 1024 * 1024;
const MAX_UNICODE_MAPPING_BYTES = 64;
const MAX_RECORDS_PER_SHARD = 4096;

export class ManifestResolverError extends Error {
	constructor(code, message, options) {
		super(message, options);
		this.name = "ManifestResolverError";
		this.code = code;
	}
}

export function parseManifestJson(text) {
	try {
		rejectDuplicateObjectKeys(text);
		return JSON.parse(text);
	} catch (error) {
		if (error instanceof ManifestResolverError) throw error;
		throw new ManifestResolverError(
			"invalid-manifest",
			"manifest is not strict JSON",
			{ cause: error },
		);
	}
}

export function validateRootManifest(value) {
	if (!isRecord(value) || ![2, 3, 4].includes(value.schema)) {
		throw invalidManifest("root manifest schema 2, 3, or 4 is required");
	}
	if (value.schema === 4)
		exactKeys(
			value,
			[
				"schema",
				"distribution",
				"objectsBaseUrl",
				"shardBits",
				"shardCount",
				"shards",
				"formats",
			],
			"HTML root manifest",
			["formats"],
		);
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
		if (value.schema === 2 && entry.inputClosure !== undefined) {
			throw invalidManifest(
				"format input closures require root manifest schema 3",
			);
		}
		const inputClosure =
			entry.inputClosure === undefined
				? undefined
				: validateFormatInputClosure(entry.inputClosure, name);
		formats[name] = Object.freeze({ ...entry, inputClosure });
	}
	return Object.freeze({
		schema: value.schema,
		distribution,
		objectsBaseUrl,
		shardBits: value.shardBits,
		shardCount: value.shardCount,
		shards: Object.freeze([...value.shards]),
		formats: Object.freeze(formats),
	});
}

function validateFormatInputClosure(value, formatName) {
	if (
		!isRecord(value) ||
		value.schema !== 1 ||
		!Array.isArray(value.keys) ||
		value.keys.length === 0 ||
		value.keys.length > MAX_FORMAT_INPUTS
	) {
		throw invalidManifest(`invalid input closure for format ${formatName}`);
	}
	let previous;
	const keys = value.keys.map((key) => {
		validateKey(key);
		if (new TextEncoder().encode(key).byteLength > MAX_REQUEST_KEY_BYTES) {
			throw invalidManifest(
				`input closure key for format ${formatName} is too long`,
			);
		}
		if (previous !== undefined && previous >= key) {
			throw invalidManifest(
				`input closure for format ${formatName} is not strictly sorted`,
			);
		}
		previous = key;
		return key;
	});
	return Object.freeze({ schema: 1, keys: Object.freeze(keys) });
}

export function validateIndexShard(value, root, index) {
	const expectedSchema = root.schema === 4 ? 2 : 1;
	if (
		!isRecord(value) ||
		value.schema !== expectedSchema ||
		value.distribution !== root.distribution ||
		value.index !== index ||
		!isRecord(value.files)
	) {
		throw invalidManifest(
			`index shard ${index} identity does not match root manifest`,
		);
	}
	if (expectedSchema === 2)
		exactKeys(
			value,
			["schema", "distribution", "index", "files", "fonts", "legacyMappings"],
			`HTML index shard ${index}`,
			["fonts", "legacyMappings"],
		);
	const files = Object.create(null);
	const fonts = Object.create(null);
	const legacyMappings = Object.create(null);
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
	if (!isRecord(value.fonts ?? {}))
		throw invalidManifest("fonts must be an object");
	if (Object.keys(value.fonts ?? {}).length > MAX_RECORDS_PER_SHARD)
		throw invalidManifest("font shard contains too many records");
	for (const [key, entry] of Object.entries(value.fonts ?? {})) {
		const request = parseFontRequestIdentity(key);
		validateFontRecord(entry, key, hashLengths);
		fonts[key] = Object.freeze({ ...entry, request });
	}
	if (!isRecord(value.legacyMappings ?? {}))
		throw invalidManifest("legacyMappings must be an object");
	if (Object.keys(value.legacyMappings ?? {}).length > MAX_RECORDS_PER_SHARD)
		throw invalidManifest("legacy mapping shard contains too many records");
	for (const [key, entry] of Object.entries(value.legacyMappings ?? {})) {
		const request = parseLegacyMappingIdentity(key);
		validateLegacyMappingRecord(entry, key, request, hashLengths);
		legacyMappings[key] = Object.freeze({
			...entry,
			request,
			fontRequest: parseFontRequestIdentity(entry.fontKey),
			unicodeMap: Object.freeze([...entry.unicodeMap]),
		});
	}
	return Object.freeze({
		schema: expectedSchema,
		distribution: root.distribution,
		index,
		files: Object.freeze(files),
		fonts: Object.freeze(fonts),
		legacyMappings: Object.freeze(legacyMappings),
	});
}

export function serializeIndexShard(shard) {
	const fonts = Object.fromEntries(
		Object.entries(shard.fonts).map(([key, entry]) => {
			const { request: _request, ...record } = entry;
			return [key, record];
		}),
	);
	const legacyMappings = Object.fromEntries(
		Object.entries(shard.legacyMappings).map(([key, entry]) => {
			const { request: _request, fontRequest: _fontRequest, ...record } = entry;
			return [key, record];
		}),
	);
	return `${JSON.stringify({
		schema: shard.schema,
		distribution: shard.distribution,
		index: shard.index,
		files: shard.files,
		...(Object.keys(fonts).length === 0 ? {} : { fonts }),
		...(Object.keys(legacyMappings).length === 0 ? {} : { legacyMappings }),
	})}\n`;
}

export async function shardIndex(key, shardBits, crypto, typed = false) {
	if (typed) validateShardKey(key);
	else validateKey(key);
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

function validateShardKey(key) {
	if (typeof key === "string" && key.startsWith("font:"))
		parseFontRequestIdentity(key);
	else if (typeof key === "string" && key.startsWith("legacy-mapping:"))
		parseLegacyMappingIdentity(key);
	else validateKey(key);
}

export function encodeRequest(request) {
	if (
		!isRecord(request) ||
		![
			"tex",
			"tfm",
			"vf",
			"font-map",
			"font-encoding",
			"font-program",
			"bib-aux",
			"classic-bib-data",
			"bib-style",
		].includes(request.kind)
	) {
		throw new ManifestResolverError(
			"invalid-request",
			"request kind is not supported by the distribution resolver",
		);
	}
	const kind =
		{
			vf: "tex",
			"font-map": "tex",
			"font-encoding": "tex",
			"font-program": "tex",
			"classic-bib-data": "classic-bib",
			"bib-style": "bst",
		}[request.kind] ?? request.kind;
	const key = `${kind}:${request.name}`;
	validateKey(key);
	return key;
}

export function decodeKey(key) {
	const match = KEY_PATTERN.exec(key);
	const kind =
		{
			"classic-bib": "classic-bib-data",
			bst: "bib-style",
		}[match[1]] ?? match[1];
	return { kind, name: match[2] };
}

export function resourceDomain(kind) {
	return [
		"tex",
		"tfm",
		"vf",
		"font-map",
		"font-encoding",
		"font-program",
	].includes(kind)
		? "tex"
		: "bibliography";
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
	const logicalName = validateBoundedText(
		request.logicalName,
		1024,
		"logicalName",
	);
	if (request.faceIndex >= 64)
		throw invalidRequest("font face index must be below 64");
	const variations = canonicalSettings(request.variations, true, "variation");
	const features = canonicalSettings(request.features, false, "feature");
	const instance =
		request.variationInstance ??
		(variations.length === 0 ? "default" : "coordinates");
	let encodedInstance;
	if (instance === "default") encodedInstance = "d";
	else if (instance === "coordinates") encodedInstance = "c";
	else if (
		isRecord(instance) &&
		Number.isSafeInteger(instance.namedNameId) &&
		instance.namedNameId >= 0 &&
		instance.namedNameId <= 65535
	)
		encodedInstance = `n${instance.namedNameId}`;
	else throw invalidRequest("invalid variation instance");
	if (encodedInstance !== "c" && variations.length !== 0)
		throw invalidRequest("only coordinate variation instances may carry axes");
	const direction = request.direction ?? "ltr";
	if (direction !== "ltr" && direction !== "rtl")
		throw invalidRequest("invalid writing direction");
	const script =
		request.script === undefined || request.script === null
			? "-"
			: hex(tagBytes(request.script));
	let language = "-";
	if (request.language !== undefined && request.language !== null) {
		const canonical = request.language.toLowerCase();
		if (!/^[a-z0-9]+(?:-[a-z0-9]+)*$/.test(canonical) || canonical.length > 63)
			throw invalidRequest("invalid font language");
		language = hex(new TextEncoder().encode(canonical));
	}
	const axes = variations
		.map(({ tag, value }) => `${hex(tagBytes(tag))}=${unsignedHex(value)}`)
		.join(",");
	const featureKey = features
		.map(({ tag, value }) => `${hex(tagBytes(tag))}=${unsignedHex(value)}`)
		.join(",");
	return `font:1:${hex(new TextEncoder().encode(logicalName))}:${request.faceIndex}:${encodedInstance}:${axes}:${featureKey}:${direction}:${script}:${language}`;
}

export function legacyMappingRequestIdentity(request) {
	if (
		!isRecord(request) ||
		!DIGEST_PATTERN.test(request.tfmSha256) ||
		request.layoutPolicyVersion !== 1 ||
		!["html-layout", "html-paint"].includes(request.purpose)
	)
		throw invalidRequest("invalid legacy mapping request");
	const encoding =
		request.encodingCatalog === undefined || request.encodingCatalog === null
			? "-"
			: hex(
					new TextEncoder().encode(
						validateBoundedText(
							request.encodingCatalog,
							128,
							"encodingCatalog",
						),
					),
				);
	return `legacy-mapping:1:${request.tfmSha256}:1:${request.purpose}:${encoding}`;
}

export function parseFontRequestIdentity(key) {
	if (
		typeof key !== "string" ||
		new TextEncoder().encode(key).byteLength > MAX_FONT_KEY_BYTES
	)
		throw invalidManifest("invalid canonical font request key");
	const parts = key.split(":");
	if (parts.length !== 10 || parts[0] !== "font" || parts[1] !== "1")
		throw invalidManifest("invalid canonical font request key");
	const logicalName = decodeUtf8Hex(parts[2], "font logical name");
	const faceIndex = canonicalDecimal(parts[3], "font face index");
	let variationInstance;
	if (parts[4] === "d") variationInstance = "default";
	else if (parts[4] === "c") variationInstance = "coordinates";
	else if (/^n(?:0|[1-9][0-9]*)$/.test(parts[4]))
		variationInstance = { namedNameId: Number(parts[4].slice(1)) };
	else throw invalidManifest("invalid variation instance in font request key");
	const variations = decodeSettings(parts[5], true);
	const features = decodeSettings(parts[6], false);
	const script =
		parts[8] === "-"
			? undefined
			: new TextDecoder().decode(unhex(parts[8], "font script"));
	const language =
		parts[9] === "-" ? undefined : decodeUtf8Hex(parts[9], "font language");
	const request = {
		logicalName,
		faceIndex,
		variationInstance,
		variations,
		features,
		direction: parts[7],
		...(script === undefined ? {} : { script }),
		...(language === undefined ? {} : { language }),
	};
	if (fontRequestIdentity(request) !== key)
		throw invalidManifest("noncanonical font request key");
	return Object.freeze(request);
}

export function parseLegacyMappingIdentity(key) {
	if (
		typeof key !== "string" ||
		new TextEncoder().encode(key).byteLength > MAX_FONT_KEY_BYTES
	)
		throw invalidManifest("invalid canonical legacy mapping request key");
	const parts = typeof key === "string" ? key.split(":") : [];
	if (parts.length !== 6 || parts[0] !== "legacy-mapping" || parts[1] !== "1")
		throw invalidManifest("invalid canonical legacy mapping request key");
	const request = {
		tfmSha256: parts[2],
		layoutPolicyVersion: canonicalDecimal(parts[3], "layout policy version"),
		purpose: parts[4],
		...(parts[5] === "-"
			? {}
			: { encodingCatalog: decodeUtf8Hex(parts[5], "encoding catalog") }),
	};
	if (legacyMappingRequestIdentity(request) !== key)
		throw invalidManifest("noncanonical legacy mapping request key");
	return Object.freeze(request);
}

function validateFontRecord(entry, key, hashLengths) {
	exactKeys(
		entry,
		[
			"schema",
			"object",
			"sha256",
			"bytes",
			"container",
			"programIdentity",
			"featurePolicyVersion",
			"provenance",
			"license",
		],
		key,
		["programIdentity"],
	);
	if (entry.schema !== 1)
		throw invalidManifest(
			`unsupported font record schema ${String(entry.schema)}`,
		);
	validateObjectEntry(entry, `font ${key}`, hashLengths);
	if (entry.container !== "woff2" || entry.featurePolicyVersion !== 1)
		throw invalidManifest(`unsupported font record policy for ${key}`);
	if (
		entry.programIdentity !== undefined &&
		!DIGEST_PATTERN.test(entry.programIdentity)
	)
		throw invalidManifest(`invalid program identity for ${key}`);
	validateProvenance(entry.provenance, key);
	validateLicense(entry.license, key, hashLengths);
}

function validateLegacyMappingRecord(entry, key, request, hashLengths) {
	exactKeys(
		entry,
		[
			"schema",
			"tfmSha256",
			"fontKey",
			"object",
			"sha256",
			"bytes",
			"container",
			"programIdentity",
			"unicodeMap",
			"mappingVersion",
			"fontdimenVersion",
			"featurePolicyVersion",
			"fallback",
			"provenance",
			"license",
		],
		key,
		["programIdentity"],
	);
	if (entry.schema !== 1)
		throw invalidManifest(
			`unsupported legacy mapping record schema ${String(entry.schema)}`,
		);
	if (entry.tfmSha256 !== request.tfmSha256)
		throw invalidManifest(
			`legacy mapping ${key} TFM digest does not match its request key`,
		);
	parseFontRequestIdentity(entry.fontKey);
	validateObjectEntry(entry, `legacy mapping ${key}`, hashLengths);
	if (
		entry.container !== "woff2" ||
		entry.mappingVersion !== 1 ||
		entry.fontdimenVersion !== 1 ||
		entry.featurePolicyVersion !== 1
	)
		throw invalidManifest(`unsupported legacy mapping policy for ${key}`);
	if (
		entry.programIdentity !== undefined &&
		!DIGEST_PATTERN.test(entry.programIdentity)
	)
		throw invalidManifest(`invalid program identity for ${key}`);
	if (
		!Array.isArray(entry.unicodeMap) ||
		entry.unicodeMap.length !== 256 ||
		entry.unicodeMap.some(
			(value) => value !== null && !validUnicodeMapping(value),
		)
	)
		throw invalidManifest(
			`Unicode map for ${key} must contain exactly 256 valid entries`,
		);
	if (!["classic-tfm-exact", "error"].includes(entry.fallback))
		throw invalidManifest(`unsupported fallback for ${key}`);
	validateProvenance(entry.provenance, key);
	validateLicense(entry.license, key, hashLengths);
}

function validateProvenance(value, key) {
	exactKeys(
		value,
		[
			"identity",
			"upstream",
			"upstreamVersion",
			"sourceUrl",
			"conversionTool",
			"conversionVersion",
		],
		`provenance for ${key}`,
	);
	if (!DIGEST_PATTERN.test(value.identity))
		throw invalidManifest(`invalid provenance identity for ${key}`);
	for (const name of [
		"upstream",
		"upstreamVersion",
		"sourceUrl",
		"conversionTool",
		"conversionVersion",
	])
		validateBoundedText(value[name], MAX_METADATA_BYTES, name, true);
	if (!value.sourceUrl.includes("://"))
		throw invalidManifest(`invalid provenance source URL for ${key}`);
}

function validateLicense(value, key, hashLengths) {
	exactKeys(
		value,
		[
			"identity",
			"object",
			"sha256",
			"bytes",
			"spdx",
			"embeddable",
			"redistributable",
		],
		`license for ${key}`,
	);
	if (!DIGEST_PATTERN.test(value.identity))
		throw invalidManifest(`invalid license identity for ${key}`);
	validateObjectEntry(value, `license for ${key}`, hashLengths);
	validateBoundedText(value.spdx, MAX_METADATA_BYTES, "spdx", true);
	if (
		value.bytes === 0 ||
		value.bytes > MAX_LICENSE_BYTES ||
		value.embeddable !== true ||
		value.redistributable !== true
	)
		throw invalidManifest(
			`record ${key} lacks affirmative embedding and redistribution authority`,
		);
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

function invalidRequest(message) {
	return new ManifestResolverError("invalid-request", message);
}

function validateBoundedText(value, limit, label, manifest = false) {
	const fail = manifest ? invalidManifest : invalidRequest;
	if (
		typeof value !== "string" ||
		value.length === 0 ||
		new TextEncoder().encode(value).byteLength > limit ||
		[...value].some((character) => {
			const code = character.codePointAt(0);
			return code <= 0x1f || (code >= 0x7f && code <= 0x9f);
		})
	)
		throw fail(`invalid ${label}`);
	return value;
}

function canonicalSettings(values, signed, label) {
	if (values.length > 64) throw invalidRequest(`too many font ${label}s`);
	const output = values
		.map((value) => {
			if (!isRecord(value)) throw invalidRequest(`invalid font ${label}`);
			tagBytes(value.tag);
			const validValue =
				Number.isInteger(value.value) &&
				(signed
					? value.value >= -2147483648 && value.value <= 2147483647
					: value.value >= 0 && value.value <= 0xffffffff);
			if (!validValue) throw invalidRequest(`invalid font ${label} value`);
			return { tag: value.tag, value: value.value };
		})
		.sort((left, right) =>
			left.tag < right.tag ? -1 : left.tag > right.tag ? 1 : 0,
		);
	for (let index = 1; index < output.length; index += 1)
		if (output[index - 1].tag === output[index].tag)
			throw invalidRequest(`duplicate font ${label}`);
	return output;
}

function tagBytes(value) {
	const bytes =
		typeof value === "string"
			? new TextEncoder().encode(value)
			: new Uint8Array();
	if (bytes.length !== 4 || bytes.some((byte) => byte < 0x20 || byte > 0x7e))
		throw invalidRequest("OpenType tags must be four printable ASCII bytes");
	return bytes;
}

function unsignedHex(value) {
	return (value >>> 0).toString(16).padStart(8, "0");
}

function hex(bytes) {
	return Array.from(bytes, (byte) => byte.toString(16).padStart(2, "0")).join(
		"",
	);
}

function unhex(value, label) {
	if (
		typeof value !== "string" ||
		value.length % 2 !== 0 ||
		!/^[0-9a-f]*$/.test(value)
	)
		throw invalidManifest(`invalid hexadecimal ${label}`);
	const bytes = new Uint8Array(value.length / 2);
	for (let index = 0; index < bytes.length; index += 1)
		bytes[index] = Number.parseInt(value.slice(index * 2, index * 2 + 2), 16);
	return bytes;
}

function decodeUtf8Hex(value, label) {
	try {
		return new TextDecoder("utf-8", { fatal: true }).decode(
			unhex(value, label),
		);
	} catch (error) {
		throw new ManifestResolverError(
			"invalid-manifest",
			`${label} is not UTF-8`,
			{ cause: error },
		);
	}
}

function canonicalDecimal(value, label) {
	if (!/^(?:0|[1-9][0-9]*)$/.test(value))
		throw invalidManifest(`invalid ${label}`);
	const number = Number(value);
	if (!Number.isSafeInteger(number)) throw invalidManifest(`invalid ${label}`);
	return number;
}

function decodeSettings(value, signed) {
	if (value === "") return [];
	return value.split(",").map((item) => {
		const parts = item.split("=");
		if (
			parts.length !== 2 ||
			!/^[0-9a-f]{8}$/.test(parts[0]) ||
			!/^[0-9a-f]{8}$/.test(parts[1])
		)
			throw invalidManifest("invalid font request setting");
		const tag = new TextDecoder().decode(unhex(parts[0], "OpenType tag"));
		const unsigned = Number.parseInt(parts[1], 16);
		return {
			tag,
			value:
				signed && unsigned >= 0x80000000 ? unsigned - 0x100000000 : unsigned,
		};
	});
}

function validUnicodeMapping(value) {
	if (
		typeof value !== "string" ||
		value.length === 0 ||
		new TextEncoder().encode(value).byteLength > MAX_UNICODE_MAPPING_BYTES
	)
		return false;
	for (let index = 0; index < value.length; index += 1) {
		const unit = value.charCodeAt(index);
		if (unit >= 0xd800 && unit <= 0xdfff) {
			if (
				unit > 0xdbff ||
				index + 1 >= value.length ||
				value.charCodeAt(index + 1) < 0xdc00 ||
				value.charCodeAt(index + 1) > 0xdfff
			)
				return false;
			index += 1;
			continue;
		}
		if (unit <= 0x1f || (unit >= 0x7f && unit <= 0x9f)) return false;
	}
	return true;
}

function exactKeys(value, allowed, label, optional = []) {
	if (!isRecord(value)) throw invalidManifest(`${label} must be an object`);
	const allowedSet = new Set(allowed);
	for (const key of Object.keys(value))
		if (!allowedSet.has(key))
			throw invalidManifest(`unknown field ${key} in ${label}`);
	const optionalSet = new Set(optional);
	for (const key of allowed)
		if (!optionalSet.has(key) && !Object.hasOwn(value, key))
			throw invalidManifest(`${label} is missing required field ${key}`);
}

function rejectDuplicateObjectKeys(text) {
	let cursor = 0;
	const whitespace = () => {
		while (/\s/.test(text[cursor] ?? "")) cursor += 1;
	};
	const stringToken = () => {
		if (text[cursor] !== '"') throw new SyntaxError("expected string");
		const start = cursor++;
		while (cursor < text.length) {
			if (text[cursor] === "\\") cursor += 2;
			else if (text[cursor++] === '"')
				return JSON.parse(text.slice(start, cursor));
		}
		throw new SyntaxError("unterminated string");
	};
	const value = () => {
		whitespace();
		if (text[cursor] === "{") return objectValue();
		if (text[cursor] === "[") return arrayValue();
		if (text[cursor] === '"') {
			stringToken();
			return;
		}
		const start = cursor;
		while (cursor < text.length && !/[\s,\]}]/.test(text[cursor])) cursor += 1;
		if (cursor === start) throw new SyntaxError("expected value");
	};
	const objectValue = () => {
		cursor += 1;
		whitespace();
		const keys = new Set();
		if (text[cursor] === "}") {
			cursor += 1;
			return;
		}
		for (;;) {
			whitespace();
			const key = stringToken();
			if (keys.has(key)) throw invalidManifest(`duplicate object key ${key}`);
			keys.add(key);
			whitespace();
			if (text[cursor++] !== ":") throw new SyntaxError("expected colon");
			value();
			whitespace();
			if (text[cursor] === "}") {
				cursor += 1;
				return;
			}
			if (text[cursor++] !== ",") throw new SyntaxError("expected comma");
		}
	};
	const arrayValue = () => {
		cursor += 1;
		whitespace();
		if (text[cursor] === "]") {
			cursor += 1;
			return;
		}
		for (;;) {
			value();
			whitespace();
			if (text[cursor] === "]") {
				cursor += 1;
				return;
			}
			if (text[cursor++] !== ",") throw new SyntaxError("expected comma");
		}
	};
	value();
	whitespace();
	if (cursor !== text.length) throw new SyntaxError("trailing content");
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
