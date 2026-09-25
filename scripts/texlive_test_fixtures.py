"""Small authenticated packed catalogs for provisioning contract tests."""

import struct

import texlive


def packed_fixture_shard(distribution: str, files: dict[str, dict[str, object]], *, index: int = 0) -> bytes:
    records = sorted(files.items())
    object_lengths: dict[int, int] = {}
    for record in files.values():
        digest = int(str(record["ahash64"]), 16)
        length = int(record["bytes"])
        assert digest not in object_lengths or object_lengths[digest] == length
        object_lengths[digest] = length
    objects = sorted(object_lengths.items())
    object_indexes = {digest: index for index, (digest, _) in enumerate(objects)}
    paths = sorted({str(record["virtualPath"]) for record in files.values()})
    path_indexes = {path: index for index, path in enumerate(paths)}
    key_blob = bytearray()
    encoded_records: list[tuple[int, int, int, int]] = []
    for key, record in records:
        key_offset = len(key_blob)
        key_blob.extend(key.encode())
        object_index = object_indexes[int(str(record["ahash64"]), 16)]
        path = str(record["virtualPath"])
        path_index = path_indexes[path]
        encoded_records.append((key_offset, len(key), object_index, path_index))
    bucket_count = 2
    while len(records) * 5 > bucket_count * 4:
        bucket_count *= 2
    buckets_offset = 80
    records_offset = buckets_offset + bucket_count * 16
    objects_offset = records_offset + len(records) * 32
    paths_offset = objects_offset + len(objects) * 16
    dependencies_offset = paths_offset + len(paths) * 8
    keys_offset = dependencies_offset
    strings_offset = keys_offset + len(key_blob)
    strings = bytearray(distribution.encode())
    path_spans = []
    for path in paths:
        path_spans.append((len(strings), len(path)))
        strings.extend(path.encode())
    total_len = strings_offset + len(strings)
    output = bytearray(total_len)
    output[:8] = b"UMBRPKS2"
    struct.pack_into(
        "<HH17I",
        output,
        8,
        2,
        0,
        3,
        index,
        0,
        len(distribution),
        bucket_count,
        len(records),
        len(objects),
        len(paths),
        0,
        buckets_offset,
        records_offset,
        objects_offset,
        paths_offset,
        dependencies_offset,
        keys_offset,
        strings_offset,
        total_len,
    )
    for bucket in range(bucket_count):
        struct.pack_into("<QII", output, buckets_offset + bucket * 16, 0, 0xFFFFFFFF, 0)
    for index, ((key, _), (key_offset, key_len, object_index, path_index)) in enumerate(
        zip(records, encoded_records, strict=True)
    ):
        struct.pack_into(
            "<IHBBIIIHHII",
            output,
            records_offset + index * 32,
            key_offset,
            key_len,
            1,
            0,
            object_index,
            path_index,
            0,
            0,
            0,
            0,
            0,
        )
        key_hash = int(texlive.ahash64_bytes(key.encode(), 2), 16)
        bucket = key_hash & (bucket_count - 1)
        while struct.unpack_from("<I", output, buckets_offset + bucket * 16 + 8)[0] != 0xFFFFFFFF:
            bucket = (bucket + 1) & (bucket_count - 1)
        struct.pack_into("<QII", output, buckets_offset + bucket * 16, key_hash, index, 0)
    for index, (digest, length) in enumerate(objects):
        struct.pack_into("<QQ", output, objects_offset + index * 16, digest, length)
    for index, span in enumerate(path_spans):
        struct.pack_into("<II", output, paths_offset + index * 8, *span)
    output[keys_offset:strings_offset] = key_blob
    output[strings_offset:] = strings
    return bytes(output)

