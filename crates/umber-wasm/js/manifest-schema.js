const KEY_PATTERN = /^(tex|tfm|bib-aux|classic-bib|bst):(.+)$/;
const DIGEST_PATTERN = /^[0-9a-f]{64}$/;
const FORMAT_NAME_PATTERN = /^[A-Za-z0-9._-]+$/;
const MAX_SHARD_BITS = 16;
const MAX_FONT_KEY_BYTES = 4096;

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
	if (typeof key !== "string") throw invalidManifest("invalid catalog key");
	if (key.startsWith("font:")) parseFontRequestIdentity(key);
	else if (key.startsWith("legacy-mapping:")) parseLegacyMappingIdentity(key);
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
