//! Portable literal lookup tables for immutable format-backed store prefixes.

const ALGORITHM: u32 = 1;
const VERSION: u32 = 1;
const SEED: u64 = 0xcbf2_9ce4_8422_2325;
const EMPTY: u32 = u32::MAX;
const HEADER_LEN: usize = 32;
const ENTRY_LEN: usize = 16;
const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;

#[derive(Clone, Debug)]
pub(crate) struct FrozenLookup {
    buckets: Vec<u32>,
    targets: Vec<u32>,
    keys: Vec<Vec<u8>>,
}

impl FrozenLookup {
    #[must_use]
    pub(crate) fn empty() -> Self {
        Self {
            buckets: vec![EMPTY; 8],
            targets: Vec::new(),
            keys: Vec::new(),
        }
    }

    #[must_use]
    pub(crate) fn get(&self, key: &[u8]) -> Option<u32> {
        let mask = self.buckets.len() - 1;
        let mut bucket = hash(key) as usize & mask;
        for _ in 0..self.buckets.len() {
            let entry = self.buckets[bucket];
            if entry == EMPTY {
                return None;
            }
            let entry = entry as usize;
            if self.keys[entry] == key {
                return Some(self.targets[entry]);
            }
            bucket = (bucket + 1) & mask;
        }
        None
    }

    pub(crate) fn spot_check(&self, checksum: u64) -> Result<(), &'static str> {
        if self.keys.is_empty() {
            return Ok(());
        }
        let checks = self.keys.len().min(8);
        let mut state = checksum;
        for _ in 0..checks {
            state ^= state >> 12;
            state ^= state << 25;
            state ^= state >> 27;
            let entry = (state.wrapping_mul(0x2545_f491_4f6c_dd1d) as usize) % self.keys.len();
            if self.get(&self.keys[entry]) != Some(self.targets[entry]) {
                return Err("frozen lookup spot check failed");
            }
        }
        Ok(())
    }

    pub(crate) fn validate_targets(&self, expected: &[Vec<u8>]) -> Result<(), &'static str> {
        if expected.len() != self.targets.len() {
            return Err("frozen lookup target count mismatch");
        }
        for (key, &target) in self.keys.iter().zip(&self.targets) {
            if expected
                .get(target as usize)
                .is_none_or(|value| value != key)
            {
                return Err("frozen lookup key does not match target");
            }
        }
        Ok(())
    }
}

pub(crate) fn encode<I>(entries: I) -> Result<Vec<u8>, &'static str>
where
    I: IntoIterator<Item = (Vec<u8>, u32)>,
{
    let mut entries: Vec<_> = entries.into_iter().collect();
    entries.sort_unstable_by(|left, right| left.0.cmp(&right.0));
    if entries.windows(2).any(|pair| pair[0].0 == pair[1].0) {
        return Err("duplicate frozen lookup key");
    }
    let count = u32::try_from(entries.len()).map_err(|_| "frozen lookup entry capacity")?;
    let bucket_count = bucket_count(entries.len())?;
    let mut buckets = vec![EMPTY; bucket_count];
    let mut maximum_probe = 0_u32;
    for (entry, (key, _)) in entries.iter().enumerate() {
        let initial = hash(key) as usize & (bucket_count - 1);
        let mut bucket = initial;
        while buckets[bucket] != EMPTY {
            bucket = (bucket + 1) & (bucket_count - 1);
        }
        let probe = ((bucket + bucket_count - initial) & (bucket_count - 1)) as u32;
        maximum_probe = maximum_probe.max(probe);
        buckets[bucket] = entry as u32;
    }
    let buckets_len = bucket_count
        .checked_mul(4)
        .ok_or("frozen lookup size overflow")?;
    let entries_len = entries
        .len()
        .checked_mul(ENTRY_LEN)
        .ok_or("frozen lookup size overflow")?;
    let keys_len = entries.iter().try_fold(0_usize, |total, (key, _)| {
        total
            .checked_add(key.len())
            .ok_or("frozen lookup size overflow")
    })?;
    let entries_offset = HEADER_LEN
        .checked_add(buckets_len)
        .ok_or("frozen lookup size overflow")?;
    let keys_offset = entries_offset
        .checked_add(entries_len)
        .ok_or("frozen lookup size overflow")?;
    let mut out = vec![
        0;
        keys_offset
            .checked_add(keys_len)
            .ok_or("frozen lookup size overflow")?
    ];
    put_u32(&mut out, 0, ALGORITHM);
    put_u32(&mut out, 4, VERSION);
    put_u64(&mut out, 8, SEED);
    put_u32(&mut out, 16, bucket_count as u32);
    put_u32(&mut out, 20, count);
    put_u32(&mut out, 24, EMPTY);
    put_u32(&mut out, 28, maximum_probe);
    for (index, bucket) in buckets.iter().copied().enumerate() {
        put_u32(&mut out, HEADER_LEN + index * 4, bucket);
    }
    let mut key_cursor = 0_usize;
    for (index, (key, target)) in entries.iter().enumerate() {
        let record = entries_offset + index * ENTRY_LEN;
        put_u32(
            &mut out,
            record,
            u32::try_from(key_cursor).map_err(|_| "frozen lookup key offset")?,
        );
        put_u32(
            &mut out,
            record + 4,
            u32::try_from(key.len()).map_err(|_| "frozen lookup key length")?,
        );
        put_u32(&mut out, record + 8, *target);
        out[keys_offset + key_cursor..keys_offset + key_cursor + key.len()].copy_from_slice(key);
        key_cursor += key.len();
    }
    Ok(out)
}

pub(crate) fn decode(bytes: &[u8], target_count: usize) -> Result<FrozenLookup, &'static str> {
    if bytes.len() < HEADER_LEN
        || read_u32(bytes, 0) != ALGORITHM
        || read_u32(bytes, 4) != VERSION
        || read_u64(bytes, 8) != SEED
        || read_u32(bytes, 24) != EMPTY
    {
        return Err("incompatible frozen lookup header");
    }
    let bucket_count = read_u32(bytes, 16) as usize;
    let entry_count = read_u32(bytes, 20) as usize;
    if bucket_count < 8
        || !bucket_count.is_power_of_two()
        || bucket_count != bucket_count_for_decode(entry_count)?
        || entry_count != target_count
    {
        return Err("invalid frozen lookup capacity");
    }
    let entries_offset = HEADER_LEN
        .checked_add(bucket_count.checked_mul(4).ok_or("frozen lookup range")?)
        .ok_or("frozen lookup range")?;
    let keys_offset = entries_offset
        .checked_add(
            entry_count
                .checked_mul(ENTRY_LEN)
                .ok_or("frozen lookup range")?,
        )
        .ok_or("frozen lookup range")?;
    if keys_offset > bytes.len() {
        return Err("truncated frozen lookup");
    }
    let mut buckets = Vec::with_capacity(bucket_count);
    let mut seen = vec![false; entry_count];
    for index in 0..bucket_count {
        let entry = read_u32(bytes, HEADER_LEN + index * 4);
        if entry != EMPTY {
            let entry = entry as usize;
            if entry >= entry_count || seen[entry] {
                return Err("invalid frozen lookup bucket entry");
            }
            seen[entry] = true;
        }
        buckets.push(entry);
    }
    if seen.iter().any(|seen| !seen) {
        return Err("missing frozen lookup bucket entry");
    }
    let mut keys = Vec::with_capacity(entry_count);
    let mut targets = Vec::with_capacity(entry_count);
    let mut cursor = 0_usize;
    for index in 0..entry_count {
        let record = entries_offset + index * ENTRY_LEN;
        let start = read_u32(bytes, record) as usize;
        let len = read_u32(bytes, record + 4) as usize;
        let target = read_u32(bytes, record + 8);
        if start != cursor || target as usize >= target_count || read_u32(bytes, record + 12) != 0 {
            return Err("invalid frozen lookup entry");
        }
        cursor = start.checked_add(len).ok_or("frozen lookup key range")?;
        if keys_offset
            .checked_add(cursor)
            .is_none_or(|end| end > bytes.len())
        {
            return Err("frozen lookup key range");
        }
        keys.push(bytes[keys_offset + start..keys_offset + cursor].to_vec());
        targets.push(target);
    }
    if keys_offset + cursor != bytes.len() || keys.windows(2).any(|pair| pair[0] >= pair[1]) {
        return Err("non-canonical frozen lookup keys");
    }
    let expected = encode(keys.iter().cloned().zip(targets.iter().copied()))?;
    if expected != bytes {
        return Err("non-canonical frozen lookup structure");
    }
    Ok(FrozenLookup {
        buckets,
        targets,
        keys,
    })
}

fn bucket_count(entry_count: usize) -> Result<usize, &'static str> {
    let mut buckets = 8_usize;
    while entry_count
        .checked_mul(4)
        .ok_or("frozen lookup capacity overflow")?
        > buckets * 3
    {
        buckets = buckets
            .checked_mul(2)
            .ok_or("frozen lookup capacity overflow")?;
    }
    Ok(buckets)
}

fn bucket_count_for_decode(entry_count: usize) -> Result<usize, &'static str> {
    bucket_count(entry_count)
}

fn hash(bytes: &[u8]) -> u64 {
    let mut value = SEED;
    for &byte in bytes {
        value ^= u64::from(byte);
        value = value.wrapping_mul(FNV_PRIME);
    }
    value
}

fn put_u32(bytes: &mut [u8], offset: usize, value: u32) {
    bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}

fn put_u64(bytes: &mut [u8], offset: usize, value: u64) {
    bytes[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
}

fn read_u32(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(bytes[offset..offset + 4].try_into().expect("fixed field"))
}

fn read_u64(bytes: &[u8], offset: usize) -> u64 {
    u64::from_le_bytes(bytes[offset..offset + 8].try_into().expect("fixed field"))
}

#[cfg(test)]
mod tests;
