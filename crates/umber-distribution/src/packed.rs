//! Versioned immutable packed distribution lookup shards.

use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fmt;

use crate::{
    DependencyHint, FontManifestRecord, LegacyMappingManifestRecord, LicenseRecord,
    ManifestParseError, ObjectEntry, ProvenanceRecord, SelectionError, ShardFile,
    ShardedManifestRoot,
};

pub const LEGACY_PACKED_SHARD_SCHEMA: u16 = 1;
pub const PACKED_SHARD_SCHEMA: u16 = 2;
const LEGACY_MAGIC: &[u8; 8] = b"UMBRPKS1";
const MAGIC: &[u8; 8] = b"UMBRPKS2";
const HEADER_BYTES: usize = 80;
const BUCKET_BYTES: usize = 16;
const RECORD_BYTES: usize = 32;
const OBJECT_BYTES: usize = 16;
const SPAN_BYTES: usize = 8;
const DEPENDENCY_BYTES: usize = 16;
const EMPTY: u32 = u32::MAX;
const FILE: u8 = 1;
const FONT: u8 = 2;
const LEGACY_MAPPING: u8 = 3;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PackedShardError(String);

impl PackedShardError {
    fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

impl fmt::Display for PackedShardError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl Error for PackedShardError {}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct Header {
    packed_schema: u16,
    manifest_schema: u32,
    index: u32,
    distribution_offset: u32,
    distribution_len: u32,
    bucket_count: u32,
    record_count: u32,
    object_count: u32,
    path_count: u32,
    dependency_count: u32,
    buckets_offset: u32,
    records_offset: u32,
    objects_offset: u32,
    paths_offset: u32,
    dependencies_offset: u32,
    keys_offset: u32,
    strings_offset: u32,
    total_len: u32,
}

/// Owned bytes which have passed complete structural, identity, partition,
/// table, and record validation exactly once.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ValidatedPackedShard {
    bytes: Vec<u8>,
    header: Header,
}

#[derive(Clone, Copy, Debug)]
pub struct PackedRecord<'a> {
    shard: &'a ValidatedPackedShard,
    index: u32,
}

#[derive(Clone, Copy)]
struct Record {
    key_offset: u32,
    key_len: u16,
    kind: u8,
    flags: u8,
    object: u32,
    path: u32,
    dependency_start: u32,
    dependency_len: u16,
    extra_offset: u32,
    extra_len: u32,
}

impl ValidatedPackedShard {
    pub fn new(
        bytes: Vec<u8>,
        root: &ShardedManifestRoot,
        expected_index: u32,
    ) -> Result<Self, PackedShardError> {
        let header = parse_header(&bytes)?;
        if header.manifest_schema != expected_shard_schema(root.schema)
            || header.index != expected_index
        {
            return Err(PackedShardError::new(format!(
                "packed shard {expected_index} identity does not match root manifest"
            )));
        }
        let distribution = string_span(
            &bytes,
            header.strings_offset,
            header.distribution_offset,
            header.distribution_len,
        )?;
        if distribution != root.distribution {
            return Err(PackedShardError::new(format!(
                "packed shard {expected_index} distribution does not match root manifest"
            )));
        }
        validate_sections(&bytes, header)?;
        let shard = Self { bytes, header };
        let key_blob = std::str::from_utf8(
            &shard.bytes[shard.header.keys_offset as usize..shard.header.strings_offset as usize],
        )
        .map_err(|_| PackedShardError::new("packed key blob is not UTF-8"))?;
        shard.validate_records(root.shard_bits, key_blob)?;
        Ok(shard)
    }

    #[must_use]
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    #[must_use]
    pub fn index(&self) -> u32 {
        self.header.index
    }

    #[must_use]
    pub fn record_count(&self) -> u32 {
        self.header.record_count
    }

    #[must_use]
    pub fn lookup(&self, key: &str) -> Option<PackedRecord<'_>> {
        let hash = crate::ahash64::shard_key(key.as_bytes());
        let mask = self.header.bucket_count - 1;
        let mut bucket = (hash as u32) & mask;
        for _ in 0..self.header.bucket_count {
            let offset = self.header.buckets_offset as usize + bucket as usize * BUCKET_BYTES;
            let stored_hash = read_u64(&self.bytes, offset).expect("validated bucket hash");
            let index = read_u32(&self.bytes, offset + 8).expect("validated bucket index");
            if index == EMPTY {
                return None;
            }
            if stored_hash == hash {
                let record = self.record(index).expect("validated record index");
                if self.key(record).expect("validated key span") == key {
                    return Some(PackedRecord { shard: self, index });
                }
            }
            bucket = (bucket + 1) & mask;
        }
        None
    }

    pub fn records(&self) -> impl Iterator<Item = PackedRecord<'_>> {
        (0..self.header.record_count).map(|index| PackedRecord { shard: self, index })
    }

    fn validate_records(&self, shard_bits: u8, key_blob: &str) -> Result<(), PackedShardError> {
        if self.header.packed_schema == PACKED_SHARD_SCHEMA {
            self.validate_canonical_object_table()?;
            self.validate_canonical_path_table()?;
        } else {
            self.validate_legacy_object_table()?;
            self.validate_legacy_path_table()?;
        }
        self.validate_dependency_table(key_blob)?;

        let mut hashes = Vec::with_capacity(self.header.record_count as usize);
        let mut previous_key = None;
        for index in 0..self.header.record_count {
            let record = self.record(index)?;
            let key = validated_key_span(key_blob, record.key_offset, record.key_len)?;
            if key.is_empty()
                || key.len() > crate::MAX_REQUEST_KEY_BYTES
                || key.chars().any(char::is_control)
            {
                return Err(PackedShardError::new("invalid packed shard request key"));
            }
            if previous_key.is_some_and(|previous: &str| previous >= key) {
                return Err(PackedShardError::new(
                    "packed shard record keys are not strictly sorted",
                ));
            }
            let hash = crate::ahash64::shard_key(key.as_bytes());
            let shard_index = if shard_bits == 0 {
                0
            } else {
                (hash >> (64 - shard_bits)) as u32
            };
            if shard_index != self.header.index {
                return Err(PackedShardError::new(format!(
                    "lookup key {key} is not in canonical shard {}",
                    self.header.index
                )));
            }
            self.validate_record(record, key, key_blob)?;
            previous_key = Some(key);
            hashes.push(hash);
        }
        self.validate_bucket_table(&hashes)
    }

    fn validate_canonical_object_table(&self) -> Result<(), PackedShardError> {
        let mut previous: Option<(u64, u64)> = None;
        for index in 0..self.header.object_count {
            let value = self.raw_object(index)?;
            self.validate_object_length(value.1)?;
            if let Some(previous) = previous {
                if previous.0 == value.0 {
                    return Err(PackedShardError::new(if previous.1 == value.1 {
                        "packed object table contains a duplicate"
                    } else {
                        "packed object digest has conflicting lengths"
                    }));
                }
                if previous.0 > value.0 {
                    return Err(PackedShardError::new(
                        "packed object table is not strictly sorted",
                    ));
                }
            }
            previous = Some(value);
        }
        Ok(())
    }

    fn validate_legacy_object_table(&self) -> Result<(), PackedShardError> {
        let mut object_values = Vec::with_capacity(self.header.object_count as usize);
        for index in 0..self.header.object_count {
            let value = self.raw_object(index)?;
            self.validate_object_length(value.1)?;
            object_values.push(value);
        }
        object_values.sort_unstable();
        for pair in object_values.windows(2) {
            if pair[0].0 == pair[1].0 {
                return Err(PackedShardError::new(if pair[0].1 == pair[1].1 {
                    "packed object table contains a duplicate"
                } else {
                    "packed object digest has conflicting lengths"
                }));
            }
        }
        Ok(())
    }

    fn validate_object_length(&self, length: u64) -> Result<(), PackedShardError> {
        if length > 128 * 1024 * 1024 {
            return Err(PackedShardError::new("packed object length is invalid"));
        }
        Ok(())
    }

    fn validate_canonical_path_table(&self) -> Result<(), PackedShardError> {
        let mut previous = None;
        for index in 0..self.header.path_count {
            let path = self.path(index)?;
            Self::validate_path(path)?;
            if let Some(previous) = previous {
                if previous == path {
                    return Err(PackedShardError::new(
                        "packed path table contains a duplicate",
                    ));
                }
                if previous > path {
                    return Err(PackedShardError::new(
                        "packed path table is not strictly sorted",
                    ));
                }
            }
            previous = Some(path);
        }
        Ok(())
    }

    fn validate_legacy_path_table(&self) -> Result<(), PackedShardError> {
        let mut path_values = Vec::with_capacity(self.header.path_count as usize);
        for index in 0..self.header.path_count {
            let path = self.path(index)?;
            Self::validate_path(path)?;
            path_values.push(path);
        }
        path_values.sort_unstable();
        if path_values.windows(2).any(|pair| pair[0] == pair[1]) {
            return Err(PackedShardError::new(
                "packed path table contains a duplicate",
            ));
        }
        Ok(())
    }

    fn validate_path(path: &str) -> Result<(), PackedShardError> {
        crate::manifest::validate_path(path, "/texlive/", "packed virtual path")
            .map_err(|error| PackedShardError::new(error.to_string()))
    }

    fn validate_dependency_table(&self, key_blob: &str) -> Result<(), PackedShardError> {
        for index in 0..self.header.dependency_count {
            let offset =
                self.header.dependencies_offset as usize + index as usize * DEPENDENCY_BYTES;
            if read_u16(&self.bytes, offset + 6)? != 0 {
                return Err(PackedShardError::new(
                    "packed dependency reserved bits are nonzero",
                ));
            }
            let dependency = self.dependency(index)?;
            let key = validated_key_span(key_blob, dependency.0, dependency.1)?;
            crate::manifest::validate_file_key(key)
                .map_err(|error| PackedShardError::new(error.to_string()))?;
            self.ensure_object_index(dependency.2)?;
            self.ensure_path_index(dependency.3)?;
        }
        Ok(())
    }

    fn validate_bucket_table(&self, record_hashes: &[u64]) -> Result<(), PackedShardError> {
        let mut occupied = 0_u32;
        let mut seen = vec![false; self.header.record_count as usize];
        let mut empty_bucket = None;
        for bucket in 0..self.header.bucket_count {
            let offset = self.header.buckets_offset as usize + bucket as usize * BUCKET_BYTES;
            let hash = read_u64(&self.bytes, offset)?;
            let index = read_u32(&self.bytes, offset + 8)?;
            let reserved = read_u32(&self.bytes, offset + 12)?;
            if reserved != 0 {
                return Err(PackedShardError::new(
                    "packed shard bucket reserved bits are nonzero",
                ));
            }
            if index == EMPTY {
                if hash != 0 {
                    return Err(PackedShardError::new(
                        "empty packed shard bucket has a hash",
                    ));
                }
                empty_bucket.get_or_insert(bucket);
                continue;
            }
            let slot = seen.get_mut(index as usize).ok_or_else(|| {
                PackedShardError::new("packed shard bucket record is out of bounds")
            })?;
            if *slot {
                return Err(PackedShardError::new(
                    "packed shard table contains a duplicate record",
                ));
            }
            *slot = true;
            occupied += 1;
            if record_hashes[index as usize] != hash {
                return Err(PackedShardError::new(
                    "packed shard bucket hash does not match its key",
                ));
            }
        }
        if occupied != self.header.record_count || seen.iter().any(|seen| !seen) {
            return Err(PackedShardError::new(
                "packed shard table does not cover every record",
            ));
        }

        // At <=80% load there is always an empty bucket. Start immediately
        // after one such bucket and unwrap the circular table into a line.
        // Every occupied slot's ideal bucket must lie inside its current
        // uninterrupted cluster and at or before the stored slot; otherwise a
        // normal lookup would stop at an earlier empty bucket.
        let empty_bucket = empty_bucket.expect("validated packed load has an empty bucket");
        let bucket_count = u64::from(self.header.bucket_count);
        let mask = self.header.bucket_count - 1;
        let mut cluster_start = u64::from(empty_bucket) + 1;
        for distance in 1..self.header.bucket_count {
            let bucket = (empty_bucket + distance) & mask;
            let linear_bucket = u64::from(empty_bucket) + u64::from(distance);
            let offset = self.header.buckets_offset as usize + bucket as usize * BUCKET_BYTES;
            let index = read_u32(&self.bytes, offset + 8)?;
            if index == EMPTY {
                cluster_start = linear_bucket + 1;
                continue;
            }
            let ideal_bucket = (record_hashes[index as usize] as u32) & mask;
            let linear_ideal = if ideal_bucket <= empty_bucket {
                u64::from(ideal_bucket) + bucket_count
            } else {
                u64::from(ideal_bucket)
            };
            if linear_ideal < cluster_start || linear_ideal > linear_bucket {
                return Err(PackedShardError::new(
                    "packed shard contains an invalid probe chain",
                ));
            }
        }
        Ok(())
    }

    fn validate_record(
        &self,
        record: Record,
        key: &str,
        key_blob: &str,
    ) -> Result<(), PackedShardError> {
        let extra = self.extra(record)?;
        match record.kind {
            FILE => {
                crate::manifest::validate_file_key(key)
                    .map_err(|error| PackedShardError::new(error.to_string()))?;
                self.ensure_object_index(record.object)?;
                if record.flags != 0 || record.path == EMPTY || !extra.is_empty() {
                    return Err(PackedShardError::new("invalid packed file record"));
                }
                self.ensure_path_index(record.path)?;
                let end = record
                    .dependency_start
                    .checked_add(u32::from(record.dependency_len))
                    .filter(|end| *end <= self.header.dependency_count)
                    .ok_or_else(|| {
                        PackedShardError::new("packed dependency span is out of bounds")
                    })?;
                let mut previous = None;
                for index in record.dependency_start..end {
                    let dependency = self.dependency(index)?;
                    let dependency_key = validated_key_span(key_blob, dependency.0, dependency.1)?;
                    if previous.is_some_and(|value: &str| value >= dependency_key) {
                        return Err(PackedShardError::new(
                            "packed dependency keys are not strictly sorted",
                        ));
                    }
                    previous = Some(dependency_key);
                }
            }
            FONT | LEGACY_MAPPING => {
                if record.path != EMPTY || record.dependency_len != 0 || record.flags != 0 {
                    return Err(PackedShardError::new("invalid packed catalogue record"));
                }
                self.ensure_object_index(record.object)?;
                if record.kind == FONT {
                    decode_font_extra(key, self.object(record.object)?, extra)?;
                } else {
                    decode_mapping_extra(key, self.object(record.object)?, extra)?;
                }
            }
            _ => return Err(PackedShardError::new("unknown packed shard record kind")),
        }
        Ok(())
    }

    fn record(&self, index: u32) -> Result<Record, PackedShardError> {
        if index >= self.header.record_count {
            return Err(PackedShardError::new(
                "packed shard record is out of bounds",
            ));
        }
        let offset = self.header.records_offset as usize + index as usize * RECORD_BYTES;
        Ok(Record {
            key_offset: read_u32(&self.bytes, offset)?,
            key_len: read_u16(&self.bytes, offset + 4)?,
            kind: self.bytes[offset + 6],
            flags: self.bytes[offset + 7],
            object: read_u32(&self.bytes, offset + 8)?,
            path: read_u32(&self.bytes, offset + 12)?,
            dependency_start: read_u32(&self.bytes, offset + 16)?,
            dependency_len: read_u16(&self.bytes, offset + 20)?,
            extra_offset: read_u32(&self.bytes, offset + 24)?,
            extra_len: read_u32(&self.bytes, offset + 28)?,
        })
    }

    fn key(&self, record: Record) -> Result<&str, PackedShardError> {
        key_span(
            &self.bytes,
            self.header.keys_offset,
            self.header.strings_offset,
            record.key_offset,
            record.key_len,
        )
    }

    fn object(&self, index: u32) -> Result<ObjectEntry, PackedShardError> {
        let (digest, bytes) = self.raw_object(index)?;
        let ahash64 = format!("{digest:016x}");
        Ok(ObjectEntry {
            object: format!("ahash64-v1-{ahash64}"),
            ahash64,
            bytes,
        })
    }

    fn raw_object(&self, index: u32) -> Result<(u64, u64), PackedShardError> {
        self.ensure_object_index(index)?;
        let offset = self.header.objects_offset as usize + index as usize * OBJECT_BYTES;
        let digest = read_u64(&self.bytes, offset)?;
        let bytes = read_u64(&self.bytes, offset + 8)?;
        Ok((digest, bytes))
    }

    fn ensure_object_index(&self, index: u32) -> Result<(), PackedShardError> {
        if index >= self.header.object_count {
            return Err(PackedShardError::new(
                "packed object index is out of bounds",
            ));
        }
        Ok(())
    }

    fn path(&self, index: u32) -> Result<&str, PackedShardError> {
        self.ensure_path_index(index)?;
        let offset = self.header.paths_offset as usize + index as usize * SPAN_BYTES;
        let start = read_u32(&self.bytes, offset)?;
        let len = read_u32(&self.bytes, offset + 4)?;
        string_span(&self.bytes, self.header.strings_offset, start, len)
    }

    fn ensure_path_index(&self, index: u32) -> Result<(), PackedShardError> {
        if index >= self.header.path_count {
            return Err(PackedShardError::new("packed path index is out of bounds"));
        }
        Ok(())
    }

    fn dependency(&self, index: u32) -> Result<(u32, u16, u32, u32), PackedShardError> {
        if index >= self.header.dependency_count {
            return Err(PackedShardError::new(
                "packed dependency index is out of bounds",
            ));
        }
        let offset = self.header.dependencies_offset as usize + index as usize * DEPENDENCY_BYTES;
        Ok((
            read_u32(&self.bytes, offset)?,
            read_u16(&self.bytes, offset + 4)?,
            read_u32(&self.bytes, offset + 8)?,
            read_u32(&self.bytes, offset + 12)?,
        ))
    }

    fn extra(&self, record: Record) -> Result<&[u8], PackedShardError> {
        bytes_span(
            &self.bytes,
            self.header.strings_offset,
            record.extra_offset,
            record.extra_len,
        )
    }
}

impl<'a> PackedRecord<'a> {
    #[must_use]
    pub fn key(&self) -> &str {
        self.shard
            .key(self.shard.record(self.index).expect("validated record"))
            .expect("validated key")
    }

    #[must_use]
    pub fn kind(&self) -> PackedRecordKind {
        match self
            .shard
            .record(self.index)
            .expect("validated record")
            .kind
        {
            FILE => PackedRecordKind::File,
            FONT => PackedRecordKind::Font,
            LEGACY_MAPPING => PackedRecordKind::LegacyMapping,
            _ => unreachable!("validated packed kind"),
        }
    }

    pub fn file(self) -> Option<PackedFileRecord<'a>> {
        (self.kind() == PackedRecordKind::File).then_some(PackedFileRecord { record: self })
    }

    pub fn font(&self) -> Result<Option<FontManifestRecord>, PackedShardError> {
        let record = self.shard.record(self.index)?;
        if record.kind != FONT {
            return Ok(None);
        }
        decode_font_extra(
            self.key(),
            self.shard.object(record.object)?,
            self.shard.extra(record)?,
        )
        .map(Some)
    }

    pub fn legacy_mapping(&self) -> Result<Option<LegacyMappingManifestRecord>, PackedShardError> {
        let record = self.shard.record(self.index)?;
        if record.kind != LEGACY_MAPPING {
            return Ok(None);
        }
        decode_mapping_extra(
            self.key(),
            self.shard.object(record.object)?,
            self.shard.extra(record)?,
        )
        .map(Some)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PackedRecordKind {
    File,
    Font,
    LegacyMapping,
}

#[derive(Clone, Copy, Debug)]
pub struct PackedFileRecord<'a> {
    record: PackedRecord<'a>,
}

impl PackedFileRecord<'_> {
    #[must_use]
    pub fn key(&self) -> &str {
        self.record.key()
    }

    #[must_use]
    pub fn virtual_path(&self) -> &str {
        let raw = self
            .record
            .shard
            .record(self.record.index)
            .expect("validated record");
        self.record.shard.path(raw.path).expect("validated path")
    }

    #[must_use]
    pub fn object(&self) -> ObjectEntry {
        let raw = self
            .record
            .shard
            .record(self.record.index)
            .expect("validated record");
        self.record
            .shard
            .object(raw.object)
            .expect("validated object")
    }

    pub fn dependencies(&self) -> impl Iterator<Item = PackedDependency<'_>> {
        let raw = self
            .record
            .shard
            .record(self.record.index)
            .expect("validated record");
        (raw.dependency_start..raw.dependency_start + u32::from(raw.dependency_len)).map(|index| {
            PackedDependency {
                shard: self.record.shard,
                index,
            }
        })
    }
}

#[derive(Clone, Copy, Debug)]
pub struct PackedDependency<'a> {
    shard: &'a ValidatedPackedShard,
    index: u32,
}

impl PackedDependency<'_> {
    #[must_use]
    pub fn key(&self) -> &str {
        let (offset, len, _, _) = self
            .shard
            .dependency(self.index)
            .expect("validated dependency");
        key_span(
            &self.shard.bytes,
            self.shard.header.keys_offset,
            self.shard.header.strings_offset,
            offset,
            len,
        )
        .expect("validated dependency key")
    }

    #[must_use]
    pub fn virtual_path(&self) -> &str {
        let (_, _, _, path) = self
            .shard
            .dependency(self.index)
            .expect("validated dependency");
        self.shard.path(path).expect("validated dependency path")
    }

    #[must_use]
    pub fn object(&self) -> ObjectEntry {
        let (_, _, object, _) = self
            .shard
            .dependency(self.index)
            .expect("validated dependency");
        self.shard
            .object(object)
            .expect("validated dependency object")
    }
}

struct BuildRecord {
    key: String,
    kind: u8,
    object: ObjectEntry,
    path: Option<String>,
    dependencies: Vec<DependencyHint>,
    extra: Vec<u8>,
}

/// Produces the canonical bytes for one publisher-resolved shard.
pub fn pack_shard(shard: &crate::ManifestShard) -> Result<Vec<u8>, PackedShardError> {
    let mut records =
        Vec::with_capacity(shard.files.len() + shard.fonts.len() + shard.legacy_mappings.len());
    records.extend(shard.files.iter().map(|(key, file)| BuildRecord {
        key: key.clone(),
        kind: FILE,
        object: file.object_entry(),
        path: Some(file.virtual_path.clone()),
        dependencies: file.dependencies.clone(),
        extra: Vec::new(),
    }));
    for (key, record) in &shard.fonts {
        records.push(BuildRecord {
            key: key.clone(),
            kind: FONT,
            object: record.object.clone(),
            path: None,
            dependencies: Vec::new(),
            extra: encode_font_extra(record)?,
        });
    }
    for (key, record) in &shard.legacy_mappings {
        records.push(BuildRecord {
            key: key.clone(),
            kind: LEGACY_MAPPING,
            object: record.object.clone(),
            path: None,
            dependencies: Vec::new(),
            extra: encode_mapping_extra(record)?,
        });
    }
    records.sort_by(|left, right| left.key.cmp(&right.key));
    if records.windows(2).any(|pair| pair[0].key == pair[1].key) {
        return Err(PackedShardError::new("duplicate packed lookup key"));
    }

    let (object_values, object_indexes, path_values, path_indexes) = canonical_tables(&records)?;
    let mut keys = BTreeMap::<String, (u32, u16)>::new();
    let mut key_blob = Vec::new();
    let mut strings = Vec::new();
    let distribution_offset = push_bytes(&mut strings, shard.distribution.as_bytes())?;
    let distribution_len = u32::try_from(shard.distribution.len())
        .map_err(|_| PackedShardError::new("distribution identity is too long"))?;
    let mut dependencies = Vec::<(u32, u16, u32, u32)>::new();
    let mut encoded_records = Vec::<Record>::new();

    for record in &records {
        let (key_offset, key_len) = intern_key(&mut keys, &mut key_blob, &record.key)?;
        let object = canonical_object_index(&object_indexes, &record.object)?;
        let path = record
            .path
            .as_deref()
            .map(|value| canonical_path_index(&path_indexes, value))
            .transpose()?
            .unwrap_or(EMPTY);
        let dependency_start = u32::try_from(dependencies.len())
            .map_err(|_| PackedShardError::new("too many packed dependencies"))?;
        for dependency in &record.dependencies {
            let (offset, len) = intern_key(&mut keys, &mut key_blob, &dependency.key)?;
            dependencies.push((
                offset,
                len,
                canonical_object_index(&object_indexes, &dependency.object_entry())?,
                canonical_path_index(&path_indexes, &dependency.virtual_path)?,
            ));
        }
        let dependency_len = u16::try_from(record.dependencies.len())
            .map_err(|_| PackedShardError::new("too many dependencies for one record"))?;
        let extra_offset = push_bytes(&mut strings, &record.extra)?;
        let extra_len = u32::try_from(record.extra.len())
            .map_err(|_| PackedShardError::new("packed record metadata is too large"))?;
        encoded_records.push(Record {
            key_offset,
            key_len,
            kind: record.kind,
            flags: 0,
            object,
            path,
            dependency_start,
            dependency_len,
            extra_offset,
            extra_len,
        });
    }
    let mut path_spans = Vec::with_capacity(path_values.len());
    for path in &path_values {
        let offset = push_bytes(&mut strings, path.as_bytes())?;
        path_spans.push((offset, path.len() as u32));
    }

    let record_count = u32::try_from(encoded_records.len())
        .map_err(|_| PackedShardError::new("too many packed records"))?;
    let bucket_count = (record_count.max(1).saturating_mul(5).saturating_add(3) / 4)
        .next_power_of_two()
        .max(2);
    let buckets_offset = HEADER_BYTES;
    let records_offset = checked_section(buckets_offset, bucket_count, BUCKET_BYTES)?;
    let objects_offset = checked_section(records_offset, record_count, RECORD_BYTES)?;
    let paths_offset = checked_section(objects_offset, object_values.len() as u32, OBJECT_BYTES)?;
    let dependencies_offset = checked_section(paths_offset, path_values.len() as u32, SPAN_BYTES)?;
    let keys_offset = checked_section(
        dependencies_offset,
        dependencies.len() as u32,
        DEPENDENCY_BYTES,
    )?;
    let strings_offset = keys_offset
        .checked_add(key_blob.len())
        .ok_or_else(|| PackedShardError::new("packed shard is too large"))?;
    let total_len = strings_offset
        .checked_add(strings.len())
        .ok_or_else(|| PackedShardError::new("packed shard is too large"))?;
    let mut output = vec![0_u8; total_len];
    output[..8].copy_from_slice(MAGIC);
    write_u16(&mut output, 8, PACKED_SHARD_SCHEMA);
    write_u32(&mut output, 12, shard.schema);
    write_u32(&mut output, 16, shard.index);
    write_u32(&mut output, 20, distribution_offset);
    write_u32(&mut output, 24, distribution_len);
    write_u32(&mut output, 28, bucket_count);
    write_u32(&mut output, 32, record_count);
    write_u32(&mut output, 36, object_values.len() as u32);
    write_u32(&mut output, 40, path_values.len() as u32);
    write_u32(&mut output, 44, dependencies.len() as u32);
    for (offset, value) in [
        (48, buckets_offset),
        (52, records_offset),
        (56, objects_offset),
        (60, paths_offset),
        (64, dependencies_offset),
        (68, keys_offset),
        (72, strings_offset),
        (76, total_len),
    ] {
        write_u32(&mut output, offset, value as u32);
    }
    for (index, record) in encoded_records.iter().copied().enumerate() {
        let offset = records_offset + index * RECORD_BYTES;
        write_record(&mut output, offset, record);
    }
    for (index, (digest, bytes)) in object_values.iter().copied().enumerate() {
        let offset = objects_offset + index * OBJECT_BYTES;
        write_u64(&mut output, offset, digest);
        write_u64(&mut output, offset + 8, bytes);
    }
    for (index, (path_offset, path_len)) in path_spans.iter().copied().enumerate() {
        let offset = paths_offset + index * SPAN_BYTES;
        write_u32(&mut output, offset, path_offset);
        write_u32(&mut output, offset + 4, path_len);
    }
    for (index, (key_offset, key_len, object, path)) in dependencies.iter().copied().enumerate() {
        let offset = dependencies_offset + index * DEPENDENCY_BYTES;
        write_u32(&mut output, offset, key_offset);
        write_u16(&mut output, offset + 4, key_len);
        write_u32(&mut output, offset + 8, object);
        write_u32(&mut output, offset + 12, path);
    }
    output[keys_offset..strings_offset].copy_from_slice(&key_blob);
    output[strings_offset..].copy_from_slice(&strings);
    let mask = bucket_count - 1;
    for (index, record) in encoded_records.iter().enumerate() {
        let key = key_span(
            &output,
            keys_offset as u32,
            strings_offset as u32,
            record.key_offset,
            record.key_len,
        )?;
        let hash = crate::ahash64::shard_key(key.as_bytes());
        let mut bucket = (hash as u32) & mask;
        loop {
            let offset = buckets_offset + bucket as usize * BUCKET_BYTES;
            if read_u32(&output, offset + 8)? == 0 {
                // Buckets are zero-initialized; encode empties after insertion.
                write_u64(&mut output, offset, hash);
                write_u32(&mut output, offset + 8, index as u32 + 1);
                break;
            }
            bucket = (bucket + 1) & mask;
        }
    }
    for bucket in 0..bucket_count {
        let offset = buckets_offset + bucket as usize * BUCKET_BYTES;
        let encoded = read_u32(&output, offset + 8)?;
        write_u32(
            &mut output,
            offset + 8,
            if encoded == 0 { EMPTY } else { encoded - 1 },
        );
    }
    Ok(output)
}

fn expected_shard_schema(root_schema: u32) -> u32 {
    if root_schema == crate::HTML_SHARDED_ROOT_SCHEMA {
        crate::HTML_INDEX_SHARD_SCHEMA
    } else {
        crate::INDEX_SHARD_SCHEMA
    }
}

fn parse_header(bytes: &[u8]) -> Result<Header, PackedShardError> {
    if bytes.len() < HEADER_BYTES || read_u16(bytes, 10)? != 0 {
        return Err(PackedShardError::new("invalid packed shard header"));
    }
    let packed_schema = read_u16(bytes, 8)?;
    let magic = &bytes[..8];
    let supported = (magic == LEGACY_MAGIC && packed_schema == LEGACY_PACKED_SHARD_SCHEMA)
        || (magic == MAGIC && packed_schema == PACKED_SHARD_SCHEMA);
    if !supported {
        return Err(PackedShardError::new("invalid packed shard header"));
    }
    Ok(Header {
        packed_schema,
        manifest_schema: read_u32(bytes, 12)?,
        index: read_u32(bytes, 16)?,
        distribution_offset: read_u32(bytes, 20)?,
        distribution_len: read_u32(bytes, 24)?,
        bucket_count: read_u32(bytes, 28)?,
        record_count: read_u32(bytes, 32)?,
        object_count: read_u32(bytes, 36)?,
        path_count: read_u32(bytes, 40)?,
        dependency_count: read_u32(bytes, 44)?,
        buckets_offset: read_u32(bytes, 48)?,
        records_offset: read_u32(bytes, 52)?,
        objects_offset: read_u32(bytes, 56)?,
        paths_offset: read_u32(bytes, 60)?,
        dependencies_offset: read_u32(bytes, 64)?,
        keys_offset: read_u32(bytes, 68)?,
        strings_offset: read_u32(bytes, 72)?,
        total_len: read_u32(bytes, 76)?,
    })
}

fn validate_sections(bytes: &[u8], h: Header) -> Result<(), PackedShardError> {
    if h.bucket_count < 2
        || !h.bucket_count.is_power_of_two()
        || h.record_count.saturating_mul(5) > h.bucket_count.saturating_mul(4)
    {
        return Err(PackedShardError::new("invalid packed shard table size"));
    }
    let expected_records =
        checked_section(h.buckets_offset as usize, h.bucket_count, BUCKET_BYTES)? as u32;
    let expected_objects =
        checked_section(h.records_offset as usize, h.record_count, RECORD_BYTES)? as u32;
    let expected_paths =
        checked_section(h.objects_offset as usize, h.object_count, OBJECT_BYTES)? as u32;
    let expected_dependencies =
        checked_section(h.paths_offset as usize, h.path_count, SPAN_BYTES)? as u32;
    let expected_keys = checked_section(
        h.dependencies_offset as usize,
        h.dependency_count,
        DEPENDENCY_BYTES,
    )? as u32;
    if h.buckets_offset != HEADER_BYTES as u32
        || h.records_offset != expected_records
        || h.objects_offset != expected_objects
        || h.paths_offset != expected_paths
        || h.dependencies_offset != expected_dependencies
        || h.keys_offset != expected_keys
        || h.keys_offset > h.strings_offset
        || h.strings_offset > h.total_len
        || h.total_len as usize != bytes.len()
    {
        return Err(PackedShardError::new(
            "packed shard section layout is invalid",
        ));
    }
    Ok(())
}

fn checked_section(start: usize, count: u32, width: usize) -> Result<usize, PackedShardError> {
    start
        .checked_add(
            (count as usize)
                .checked_mul(width)
                .ok_or_else(|| PackedShardError::new("packed shard section overflows"))?,
        )
        .ok_or_else(|| PackedShardError::new("packed shard section overflows"))
}

fn intern_key(
    map: &mut BTreeMap<String, (u32, u16)>,
    blob: &mut Vec<u8>,
    key: &str,
) -> Result<(u32, u16), PackedShardError> {
    if let Some(span) = map.get(key) {
        return Ok(*span);
    }
    let offset = push_bytes(blob, key.as_bytes())?;
    let len =
        u16::try_from(key.len()).map_err(|_| PackedShardError::new("packed key is too long"))?;
    map.insert(key.to_owned(), (offset, len));
    Ok((offset, len))
}

type CanonicalTables = (
    Vec<(u64, u64)>,
    BTreeMap<u64, u32>,
    Vec<String>,
    BTreeMap<String, u32>,
);

fn canonical_tables(records: &[BuildRecord]) -> Result<CanonicalTables, PackedShardError> {
    let mut objects = BTreeMap::<u64, u64>::new();
    let mut paths = BTreeSet::<String>::new();
    for record in records {
        collect_object(&mut objects, &record.object)?;
        if let Some(path) = &record.path {
            paths.insert(path.clone());
        }
        for dependency in &record.dependencies {
            collect_object(&mut objects, &dependency.object_entry())?;
            paths.insert(dependency.virtual_path.clone());
        }
    }

    let object_values = objects
        .iter()
        .map(|(&digest, &bytes)| (digest, bytes))
        .collect::<Vec<_>>();
    let object_indexes = object_values
        .iter()
        .enumerate()
        .map(|(index, &(digest, _))| {
            u32::try_from(index)
                .map(|index| (digest, index))
                .map_err(|_| PackedShardError::new("too many packed objects"))
        })
        .collect::<Result<BTreeMap<_, _>, _>>()?;
    let path_values = paths.into_iter().collect::<Vec<_>>();
    let path_indexes = path_values
        .iter()
        .enumerate()
        .map(|(index, path)| {
            u32::try_from(index)
                .map(|index| (path.clone(), index))
                .map_err(|_| PackedShardError::new("too many packed paths"))
        })
        .collect::<Result<BTreeMap<_, _>, _>>()?;
    Ok((object_values, object_indexes, path_values, path_indexes))
}

fn object_digest(object: &ObjectEntry) -> Result<u64, PackedShardError> {
    let digest = u64::from_str_radix(&object.ahash64, 16)
        .map_err(|_| PackedShardError::new("invalid packed object digest"))?;
    if object.object != format!("ahash64-v1-{}", object.ahash64) {
        return Err(PackedShardError::new(
            "packed object name does not match digest",
        ));
    }
    Ok(digest)
}

fn collect_object(
    objects: &mut BTreeMap<u64, u64>,
    object: &ObjectEntry,
) -> Result<(), PackedShardError> {
    let digest = object_digest(object)?;
    if let Some(previous_bytes) = objects.insert(digest, object.bytes)
        && previous_bytes != object.bytes
    {
        return Err(PackedShardError::new(
            "packed object digest has conflicting lengths",
        ));
    }
    Ok(())
}

fn canonical_object_index(
    indexes: &BTreeMap<u64, u32>,
    object: &ObjectEntry,
) -> Result<u32, PackedShardError> {
    indexes
        .get(&object_digest(object)?)
        .copied()
        .ok_or_else(|| PackedShardError::new("packed object is absent from canonical table"))
}

fn canonical_path_index(
    indexes: &BTreeMap<String, u32>,
    path: &str,
) -> Result<u32, PackedShardError> {
    indexes
        .get(path)
        .copied()
        .ok_or_else(|| PackedShardError::new("packed path is absent from canonical table"))
}
fn push_bytes(output: &mut Vec<u8>, bytes: &[u8]) -> Result<u32, PackedShardError> {
    let offset = u32::try_from(output.len())
        .map_err(|_| PackedShardError::new("packed shard is too large"))?;
    output.extend_from_slice(bytes);
    Ok(offset)
}

fn write_record(output: &mut [u8], offset: usize, r: Record) {
    write_u32(output, offset, r.key_offset);
    write_u16(output, offset + 4, r.key_len);
    output[offset + 6] = r.kind;
    output[offset + 7] = r.flags;
    write_u32(output, offset + 8, r.object);
    write_u32(output, offset + 12, r.path);
    write_u32(output, offset + 16, r.dependency_start);
    write_u16(output, offset + 20, r.dependency_len);
    write_u32(output, offset + 24, r.extra_offset);
    write_u32(output, offset + 28, r.extra_len)
}
fn read_u16(bytes: &[u8], offset: usize) -> Result<u16, PackedShardError> {
    bytes
        .get(offset..offset + 2)
        .and_then(|v| v.try_into().ok())
        .map(u16::from_le_bytes)
        .ok_or_else(|| PackedShardError::new("truncated packed shard integer"))
}
fn read_u32(bytes: &[u8], offset: usize) -> Result<u32, PackedShardError> {
    bytes
        .get(offset..offset + 4)
        .and_then(|v| v.try_into().ok())
        .map(u32::from_le_bytes)
        .ok_or_else(|| PackedShardError::new("truncated packed shard integer"))
}
fn read_u64(bytes: &[u8], offset: usize) -> Result<u64, PackedShardError> {
    bytes
        .get(offset..offset + 8)
        .and_then(|v| v.try_into().ok())
        .map(u64::from_le_bytes)
        .ok_or_else(|| PackedShardError::new("truncated packed shard integer"))
}
fn write_u16(bytes: &mut [u8], offset: usize, value: u16) {
    bytes[offset..offset + 2].copy_from_slice(&value.to_le_bytes())
}
fn write_u32(bytes: &mut [u8], offset: usize, value: u32) {
    bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes())
}
fn write_u64(bytes: &mut [u8], offset: usize, value: u64) {
    bytes[offset..offset + 8].copy_from_slice(&value.to_le_bytes())
}
fn bytes_span(bytes: &[u8], base: u32, offset: u32, len: u32) -> Result<&[u8], PackedShardError> {
    let start = (base as usize)
        .checked_add(offset as usize)
        .ok_or_else(|| PackedShardError::new("packed span overflows"))?;
    let end = start
        .checked_add(len as usize)
        .ok_or_else(|| PackedShardError::new("packed span overflows"))?;
    bytes
        .get(start..end)
        .ok_or_else(|| PackedShardError::new("packed span is out of bounds"))
}
fn string_span(bytes: &[u8], base: u32, offset: u32, len: u32) -> Result<&str, PackedShardError> {
    std::str::from_utf8(bytes_span(bytes, base, offset, len)?)
        .map_err(|_| PackedShardError::new("packed string is not UTF-8"))
}
fn key_span(
    bytes: &[u8],
    base: u32,
    end: u32,
    offset: u32,
    len: u16,
) -> Result<&str, PackedShardError> {
    let start =
        base.checked_add(offset)
            .ok_or_else(|| PackedShardError::new("packed key span overflows"))? as usize;
    let stop = start
        .checked_add(len as usize)
        .ok_or_else(|| PackedShardError::new("packed key span overflows"))?;
    if stop > end as usize {
        return Err(PackedShardError::new("packed key span is out of bounds"));
    }
    std::str::from_utf8(&bytes[start..stop])
        .map_err(|_| PackedShardError::new("packed key is not UTF-8"))
}

fn validated_key_span(blob: &str, offset: u32, len: u16) -> Result<&str, PackedShardError> {
    let start = offset as usize;
    let end = start
        .checked_add(len as usize)
        .ok_or_else(|| PackedShardError::new("packed key span overflows"))?;
    blob.get(start..end)
        .ok_or_else(|| PackedShardError::new("packed key span is out of bounds"))
}

struct ExtraWriter(Vec<u8>);
impl ExtraWriter {
    fn string(&mut self, value: &str) -> Result<(), PackedShardError> {
        let len = u32::try_from(value.len())
            .map_err(|_| PackedShardError::new("packed metadata field is too large"))?;
        self.0.extend_from_slice(&len.to_le_bytes());
        self.0.extend_from_slice(value.as_bytes());
        Ok(())
    }
    fn optional(&mut self, value: Option<&str>) -> Result<(), PackedShardError> {
        match value {
            Some(v) => {
                self.0.push(1);
                self.string(v)
            }
            None => {
                self.0.push(0);
                Ok(())
            }
        }
    }
    fn u32(&mut self, v: u32) {
        self.0.extend_from_slice(&v.to_le_bytes())
    }
    fn bool(&mut self, v: bool) {
        self.0.push(u8::from(v))
    }
}
struct ExtraReader<'a> {
    bytes: &'a [u8],
    offset: usize,
}
impl<'a> ExtraReader<'a> {
    fn string(&mut self) -> Result<String, PackedShardError> {
        let len = read_u32(self.bytes, self.offset)? as usize;
        self.offset += 4;
        let end = self
            .offset
            .checked_add(len)
            .ok_or_else(|| PackedShardError::new("packed metadata overflows"))?;
        let value = std::str::from_utf8(
            self.bytes
                .get(self.offset..end)
                .ok_or_else(|| PackedShardError::new("packed metadata is truncated"))?,
        )
        .map_err(|_| PackedShardError::new("packed metadata is not UTF-8"))?
        .to_owned();
        self.offset = end;
        Ok(value)
    }
    fn optional(&mut self) -> Result<Option<String>, PackedShardError> {
        let flag = *self
            .bytes
            .get(self.offset)
            .ok_or_else(|| PackedShardError::new("packed metadata is truncated"))?;
        self.offset += 1;
        match flag {
            0 => Ok(None),
            1 => self.string().map(Some),
            _ => Err(PackedShardError::new("invalid packed optional field")),
        }
    }
    fn u32(&mut self) -> Result<u32, PackedShardError> {
        let v = read_u32(self.bytes, self.offset)?;
        self.offset += 4;
        Ok(v)
    }
    fn bool(&mut self) -> Result<bool, PackedShardError> {
        let v = *self
            .bytes
            .get(self.offset)
            .ok_or_else(|| PackedShardError::new("packed metadata is truncated"))?;
        self.offset += 1;
        match v {
            0 => Ok(false),
            1 => Ok(true),
            _ => Err(PackedShardError::new("invalid packed boolean")),
        }
    }
    fn finish(self) -> Result<(), PackedShardError> {
        if self.offset == self.bytes.len() {
            Ok(())
        } else {
            Err(PackedShardError::new("trailing packed metadata"))
        }
    }
}

fn encode_provenance(w: &mut ExtraWriter, p: &ProvenanceRecord) -> Result<(), PackedShardError> {
    for value in [
        &p.identity,
        &p.upstream,
        &p.upstream_version,
        &p.source_url,
        &p.conversion_tool,
        &p.conversion_version,
    ] {
        w.string(value)?
    }
    Ok(())
}
fn decode_provenance(r: &mut ExtraReader<'_>) -> Result<ProvenanceRecord, PackedShardError> {
    Ok(ProvenanceRecord {
        identity: r.string()?,
        upstream: r.string()?,
        upstream_version: r.string()?,
        source_url: r.string()?,
        conversion_tool: r.string()?,
        conversion_version: r.string()?,
    })
}
fn encode_license(w: &mut ExtraWriter, l: &LicenseRecord) -> Result<(), PackedShardError> {
    w.string(&l.identity)?;
    w.string(&l.object.object)?;
    w.string(&l.object.ahash64)?;
    w.0.extend_from_slice(&l.object.bytes.to_le_bytes());
    w.string(&l.spdx)?;
    w.bool(l.embeddable);
    w.bool(l.redistributable);
    Ok(())
}
fn decode_license(r: &mut ExtraReader<'_>) -> Result<LicenseRecord, PackedShardError> {
    let identity = r.string()?;
    let object_name = r.string()?;
    let digest = r.string()?;
    let bytes = read_u64(r.bytes, r.offset)?;
    r.offset += 8;
    let spdx = r.string()?;
    let embeddable = r.bool()?;
    let redistributable = r.bool()?;
    Ok(LicenseRecord {
        identity,
        object: ObjectEntry {
            object: object_name,
            ahash64: digest,
            bytes,
        },
        spdx,
        embeddable,
        redistributable,
    })
}
fn encode_font_extra(value: &FontManifestRecord) -> Result<Vec<u8>, PackedShardError> {
    let mut w = ExtraWriter(Vec::new());
    w.u32(value.schema);
    w.string(&value.container)?;
    w.optional(value.declared_program_identity.as_deref())?;
    w.u32(value.feature_policy_version);
    encode_provenance(&mut w, &value.provenance)?;
    encode_license(&mut w, &value.license)?;
    Ok(w.0)
}
fn decode_font_extra(
    key: &str,
    object: ObjectEntry,
    bytes: &[u8],
) -> Result<FontManifestRecord, PackedShardError> {
    let mut r = ExtraReader { bytes, offset: 0 };
    let value = FontManifestRecord {
        schema: r.u32()?,
        request: crate::FontRequestKey::from_manifest_key(key)
            .map_err(|e| PackedShardError::new(e.to_string()))?,
        object,
        container: r.string()?,
        declared_program_identity: r.optional()?,
        feature_policy_version: r.u32()?,
        provenance: decode_provenance(&mut r)?,
        license: decode_license(&mut r)?,
    };
    r.finish()?;
    validate_font_record(&value)?;
    Ok(value)
}
fn encode_mapping_extra(value: &LegacyMappingManifestRecord) -> Result<Vec<u8>, PackedShardError> {
    let mut w = ExtraWriter(Vec::new());
    w.u32(value.schema);
    w.string(&value.font_request.manifest_key().to_string())?;
    w.string(&value.container)?;
    w.optional(value.declared_program_identity.as_deref())?;
    w.u32(value.unicode_map.len() as u32);
    for v in &value.unicode_map {
        w.optional(v.as_deref())?
    }
    w.u32(value.mapping_version);
    w.u32(value.fontdimen_version);
    w.u32(value.feature_policy_version);
    w.string(&value.fallback)?;
    encode_provenance(&mut w, &value.provenance)?;
    encode_license(&mut w, &value.license)?;
    Ok(w.0)
}
fn decode_mapping_extra(
    key: &str,
    object: ObjectEntry,
    bytes: &[u8],
) -> Result<LegacyMappingManifestRecord, PackedShardError> {
    let mut r = ExtraReader { bytes, offset: 0 };
    let schema = r.u32()?;
    let font_key = r.string()?;
    let container = r.string()?;
    let program = r.optional()?;
    let count = r.u32()?;
    if count != 256 {
        return Err(PackedShardError::new(
            "packed Unicode map must contain 256 entries",
        ));
    }
    let mut unicode_map = Vec::with_capacity(256);
    for _ in 0..256 {
        unicode_map.push(r.optional()?)
    }
    let value = LegacyMappingManifestRecord {
        schema,
        request: crate::LegacyMappingRequestKey::from_manifest_key(key)
            .map_err(|e| PackedShardError::new(e.to_string()))?,
        font_request: crate::FontRequestKey::from_manifest_key(&font_key)
            .map_err(|e| PackedShardError::new(e.to_string()))?,
        object,
        container,
        declared_program_identity: program,
        unicode_map,
        mapping_version: r.u32()?,
        fontdimen_version: r.u32()?,
        feature_policy_version: r.u32()?,
        fallback: r.string()?,
        provenance: decode_provenance(&mut r)?,
        license: decode_license(&mut r)?,
    };
    r.finish()?;
    validate_mapping_record(&value)?;
    Ok(value)
}

fn validate_font_record(value: &FontManifestRecord) -> Result<(), PackedShardError> {
    if value.schema != crate::FONT_RECORD_SCHEMA
        || value.container != "woff2"
        || value.feature_policy_version != 1
    {
        return Err(PackedShardError::new("invalid packed font record policy"));
    }
    validate_optional_digest(value.declared_program_identity.as_deref())?;
    validate_provenance(&value.provenance)?;
    validate_license(&value.license)
}

fn validate_mapping_record(value: &LegacyMappingManifestRecord) -> Result<(), PackedShardError> {
    if value.schema != crate::LEGACY_MAPPING_RECORD_SCHEMA
        || value.container != "woff2"
        || value.mapping_version != 1
        || value.fontdimen_version != 1
        || value.feature_policy_version != 1
        || !matches!(value.fallback.as_str(), "error" | "classic-tfm-exact")
        || value.unicode_map.len() != 256
    {
        return Err(PackedShardError::new(
            "invalid packed legacy mapping policy",
        ));
    }
    for entry in value.unicode_map.iter().flatten() {
        if entry.is_empty() || entry.len() > 64 || entry.chars().any(char::is_control) {
            return Err(PackedShardError::new(
                "invalid packed Unicode mapping entry",
            ));
        }
    }
    validate_optional_digest(value.declared_program_identity.as_deref())?;
    validate_provenance(&value.provenance)?;
    validate_license(&value.license)
}

fn validate_optional_digest(value: Option<&str>) -> Result<(), PackedShardError> {
    if let Some(value) = value {
        crate::manifest::validate_digest(value, "packed digest")
            .map_err(|error| PackedShardError::new(error.to_string()))?;
    }
    Ok(())
}

fn validate_provenance(value: &ProvenanceRecord) -> Result<(), PackedShardError> {
    crate::manifest::validate_digest(&value.identity, "packed provenance identity")
        .map_err(|error| PackedShardError::new(error.to_string()))?;
    for field in [
        &value.upstream,
        &value.upstream_version,
        &value.source_url,
        &value.conversion_tool,
        &value.conversion_version,
    ] {
        if field.is_empty() || field.len() > 4096 || field.chars().any(char::is_control) {
            return Err(PackedShardError::new("invalid packed provenance field"));
        }
    }
    if !value.source_url.contains("://") {
        return Err(PackedShardError::new("invalid packed provenance URL"));
    }
    Ok(())
}

fn validate_license(value: &LicenseRecord) -> Result<(), PackedShardError> {
    crate::manifest::validate_digest(&value.identity, "packed license identity")
        .map_err(|error| PackedShardError::new(error.to_string()))?;
    crate::manifest::validate_digest(&value.object.ahash64, "packed license digest")
        .map_err(|error| PackedShardError::new(error.to_string()))?;
    if value.object.object != format!("ahash64-v1-{}", value.object.ahash64)
        || value.object.bytes == 0
        || value.object.bytes > 1024 * 1024
        || value.spdx.is_empty()
        || value.spdx.len() > 4096
        || value.spdx.chars().any(char::is_control)
        || !value.embeddable
        || !value.redistributable
    {
        return Err(PackedShardError::new("invalid packed license record"));
    }
    Ok(())
}

impl From<PackedShardError> for SelectionError {
    fn from(value: PackedShardError) -> Self {
        SelectionError::new(value.to_string())
    }
}
impl From<PackedShardError> for ManifestParseError {
    fn from(value: PackedShardError) -> Self {
        ManifestParseError::new(value.to_string())
    }
}

/// Publisher/verifier conversion back to the typed complete shard model.
pub fn unpack_shard(
    shard: &ValidatedPackedShard,
) -> Result<crate::ManifestShard, PackedShardError> {
    let mut files = BTreeMap::new();
    let mut fonts = BTreeMap::new();
    let mut legacy_mappings = BTreeMap::new();
    for record in shard.records() {
        match record.kind() {
            PackedRecordKind::File => {
                let file = record.file().expect("file kind");
                let dependencies = file
                    .dependencies()
                    .map(|d| {
                        let object = d.object();
                        DependencyHint {
                            key: d.key().to_owned(),
                            virtual_path: d.virtual_path().to_owned(),
                            object: object.object,
                            ahash64: object.ahash64,
                            bytes: object.bytes,
                        }
                    })
                    .collect();
                let object = file.object();
                files.insert(
                    file.key().to_owned(),
                    ShardFile {
                        virtual_path: file.virtual_path().to_owned(),
                        object: object.object,
                        ahash64: object.ahash64,
                        bytes: object.bytes,
                        dependencies,
                    },
                );
            }
            PackedRecordKind::Font => {
                fonts.insert(record.key().to_owned(), record.font()?.expect("font kind"));
            }
            PackedRecordKind::LegacyMapping => {
                legacy_mappings.insert(
                    record.key().to_owned(),
                    record.legacy_mapping()?.expect("mapping kind"),
                );
            }
        }
    }
    Ok(crate::ManifestShard {
        schema: shard.header.manifest_schema,
        distribution: string_span(
            &shard.bytes,
            shard.header.strings_offset,
            shard.header.distribution_offset,
            shard.header.distribution_len,
        )?
        .to_owned(),
        index: shard.header.index,
        files,
        fonts,
        legacy_mappings,
    })
}
