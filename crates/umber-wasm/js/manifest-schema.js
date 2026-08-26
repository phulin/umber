const KEY_PATTERN = /^(tex|tfm|bib-aux|classic-bib|bst):(.+)$/;
const DIGEST_PATTERN = /^[0-9a-f]{16}$/;

export class ManifestResolverError extends Error {
	constructor(code, message, options) {
		super(message, options);
		this.name = "ManifestResolverError";
		this.code = code;
	}
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
		!DIGEST_PATTERN.test(request.tfmAhash64) ||
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
	return `legacy-mapping:2:${request.tfmAhash64}:1:${request.purpose}:${encoding}`;
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
