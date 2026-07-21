//! Small deterministic PDF input fixtures for parser and importer tests.
//!
//! This deliberately accepts raw PDF value syntax instead of modeling the PDF
//! object system. It owns only the framing that is easy to get subtly wrong:
//! indirect objects, stream lengths, classic xref offsets, and the trailer.

use std::collections::BTreeMap;
use std::error::Error;
use std::fmt;

#[cfg(test)]
mod tests;

const MAX_CLASSIC_XREF_OFFSET: u64 = 9_999_999_999;

/// An insertion-ordered dictionary containing caller-supplied PDF value syntax.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Dictionary {
    entries: Vec<(String, Vec<u8>)>,
}

impl Dictionary {
    pub fn new() -> Self {
        Self::default()
    }

    /// Add an entry in its eventual serialization order.
    ///
    /// `value` is raw PDF value syntax. Keys are restricted to the simple ASCII
    /// names used by synthetic fixtures; use raw objects for malformed names.
    #[must_use]
    pub fn entry(mut self, key: &str, value: impl AsRef<[u8]>) -> Self {
        assert_simple_name(key);
        assert!(
            !self.entries.iter().any(|(existing, _)| existing == key),
            "duplicate PDF fixture dictionary key /{key}"
        );
        self.entries.push((key.to_owned(), value.as_ref().to_vec()));
        self
    }

    /// Serialize this dictionary for use as a nested raw PDF value.
    #[must_use]
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut output = Vec::new();
        self.write(&mut output, None);
        output
    }

    fn contains(&self, key: &str) -> bool {
        self.entries.iter().any(|(existing, _)| existing == key)
    }

    fn write(&self, output: &mut Vec<u8>, first_entry: Option<(&str, &[u8])>) {
        output.extend_from_slice(b"<<\n");
        if let Some((key, value)) = first_entry {
            write_entry(output, key, value);
        }
        for (key, value) in &self.entries {
            write_entry(output, key, value);
        }
        output.extend_from_slice(b">>");
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum ObjectBody {
    Raw(Vec<u8>),
    Dictionary(Dictionary),
    Stream {
        dictionary: Dictionary,
        data: Vec<u8>,
    },
}

/// A deterministic classic-xref PDF fixture assembled from explicit objects.
#[derive(Clone, Debug)]
pub struct PdfFixture {
    version: String,
    objects: BTreeMap<u32, ObjectBody>,
    trailer: Dictionary,
}

impl PdfFixture {
    pub fn new(version: &str) -> Result<Self, FixtureError> {
        if !valid_version(version) {
            return Err(FixtureError::InvalidVersion(version.to_owned()));
        }
        Ok(Self {
            version: version.to_owned(),
            objects: BTreeMap::new(),
            trailer: Dictionary::new(),
        })
    }

    pub fn add_raw_object(
        &mut self,
        object_number: u32,
        body: impl AsRef<[u8]>,
    ) -> Result<(), FixtureError> {
        self.add_object(object_number, ObjectBody::Raw(body.as_ref().to_vec()))
    }

    pub fn add_dictionary(
        &mut self,
        object_number: u32,
        dictionary: Dictionary,
    ) -> Result<(), FixtureError> {
        self.add_object(object_number, ObjectBody::Dictionary(dictionary))
    }

    /// Add a raw stream. `/Length` is calculated and must not be supplied.
    pub fn add_stream(
        &mut self,
        object_number: u32,
        dictionary: Dictionary,
        data: impl AsRef<[u8]>,
    ) -> Result<(), FixtureError> {
        if dictionary.contains("Length") {
            return Err(FixtureError::ReservedStreamKey("Length"));
        }
        self.add_object(
            object_number,
            ObjectBody::Stream {
                dictionary,
                data: data.as_ref().to_vec(),
            },
        )
    }

    /// Add an already-encoded stream and declare its single PDF filter.
    ///
    /// This never encodes the bytes. For filter arrays or decode parameters,
    /// put the raw entries in `dictionary` and call [`Self::add_stream`].
    pub fn add_filtered_stream(
        &mut self,
        object_number: u32,
        dictionary: Dictionary,
        filter: &str,
        encoded_data: impl AsRef<[u8]>,
    ) -> Result<(), FixtureError> {
        if dictionary.contains("Filter") {
            return Err(FixtureError::ReservedStreamKey("Filter"));
        }
        self.add_stream(
            object_number,
            dictionary.entry("Filter", name(filter)),
            encoded_data,
        )
    }

    pub fn set_trailer_entry(
        &mut self,
        key: &str,
        value: impl AsRef<[u8]>,
    ) -> Result<(), FixtureError> {
        assert_simple_name(key);
        if key == "Size" {
            return Err(FixtureError::ReservedTrailerKey("Size"));
        }
        if self.trailer.contains(key) {
            return Err(FixtureError::DuplicateTrailerKey(key.to_owned()));
        }
        self.trailer
            .entries
            .push((key.to_owned(), value.as_ref().to_vec()));
        Ok(())
    }

    /// Serialize and internally verify every xref offset and stream length.
    pub fn finish(self) -> Result<Vec<u8>, FixtureError> {
        let max_object = self.objects.keys().next_back().copied().unwrap_or(0);
        let size = max_object
            .checked_add(1)
            .ok_or(FixtureError::ObjectNumberTooLarge(max_object))?;
        let mut output = format!("%PDF-{}\n", self.version).into_bytes();
        output.extend_from_slice(b"%\xd3\xeb\xe9\xe1\n");

        let mut records = Vec::with_capacity(self.objects.len());
        for (object_number, body) in self.objects {
            let offset = output.len();
            if u64::try_from(offset).map_or(true, |offset| offset > MAX_CLASSIC_XREF_OFFSET) {
                return Err(FixtureError::OffsetTooLarge(offset));
            }
            output.extend_from_slice(format!("{object_number} 0 obj\n").as_bytes());
            let stream = match body {
                ObjectBody::Raw(bytes) => {
                    output.extend_from_slice(&bytes);
                    None
                }
                ObjectBody::Dictionary(dictionary) => {
                    dictionary.write(&mut output, None);
                    None
                }
                ObjectBody::Stream { dictionary, data } => {
                    let length = data.len();
                    let length_value = length.to_string();
                    dictionary.write(&mut output, Some(("Length", length_value.as_bytes())));
                    output.extend_from_slice(b"\nstream\n");
                    let data_offset = output.len();
                    output.extend_from_slice(&data);
                    output.extend_from_slice(b"\nendstream");
                    Some((data_offset, data.len()))
                }
            };
            output.extend_from_slice(b"\nendobj\n");
            records.push(ObjectRecord {
                object_number,
                offset,
                stream,
            });
        }

        let xref_offset = output.len();
        if u64::try_from(xref_offset).map_or(true, |offset| offset > MAX_CLASSIC_XREF_OFFSET) {
            return Err(FixtureError::OffsetTooLarge(xref_offset));
        }
        output.extend_from_slice(format!("xref\n0 {size}\n").as_bytes());
        let offsets: BTreeMap<_, _> = records
            .iter()
            .map(|record| (record.object_number, record.offset))
            .collect();
        let mut free_objects = (1..size)
            .filter(|object_number| !offsets.contains_key(object_number))
            .peekable();
        let first_free = free_objects.peek().copied().unwrap_or(0);
        output.extend_from_slice(format!("{first_free:010} 65535 f \n").as_bytes());
        for object_number in 1..size {
            if let Some(offset) = offsets.get(&object_number) {
                output.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
            } else {
                debug_assert_eq!(free_objects.next(), Some(object_number));
                let next_free = free_objects.peek().copied().unwrap_or(0);
                output.extend_from_slice(format!("{next_free:010} 00000 f \n").as_bytes());
            }
        }
        output.extend_from_slice(b"trailer\n");
        let size_value = size.to_string();
        self.trailer
            .write(&mut output, Some(("Size", size_value.as_bytes())));
        output.extend_from_slice(format!("\nstartxref\n{xref_offset}\n%%EOF\n").as_bytes());

        verify_records(&output, xref_offset, size, &records)?;
        Ok(output)
    }

    fn add_object(&mut self, object_number: u32, body: ObjectBody) -> Result<(), FixtureError> {
        if object_number == 0 {
            return Err(FixtureError::ZeroObjectNumber);
        }
        if self.objects.contains_key(&object_number) {
            return Err(FixtureError::DuplicateObject(object_number));
        }
        self.objects.insert(object_number, body);
        Ok(())
    }
}

#[derive(Clone, Debug)]
struct ObjectRecord {
    object_number: u32,
    offset: usize,
    stream: Option<(usize, usize)>,
}

fn verify_records(
    bytes: &[u8],
    xref_offset: usize,
    size: u32,
    records: &[ObjectRecord],
) -> Result<(), FixtureError> {
    if bytes.get(xref_offset..xref_offset + 5) != Some(b"xref\n") {
        return Err(FixtureError::InternalVerification("startxref offset"));
    }
    for record in records {
        let header = format!("{} 0 obj\n", record.object_number);
        if bytes.get(record.offset..record.offset + header.len()) != Some(header.as_bytes()) {
            return Err(FixtureError::InternalVerification("object offset"));
        }
        if let Some((data_offset, length)) = record.stream {
            let length_entry = format!("/Length {length}\n");
            if !bytes[record.offset..data_offset]
                .windows(length_entry.len())
                .any(|window| window == length_entry.as_bytes())
                || bytes.get(data_offset + length..data_offset + length + 10)
                    != Some(b"\nendstream")
            {
                return Err(FixtureError::InternalVerification("stream length"));
            }
        }
    }
    let trailer_offset = bytes[xref_offset..]
        .windows(b"trailer\n".len())
        .position(|window| window == b"trailer\n")
        .map(|relative| xref_offset + relative)
        .ok_or(FixtureError::InternalVerification("xref trailer"))?;
    let xref = std::str::from_utf8(&bytes[xref_offset..trailer_offset])
        .map_err(|_| FixtureError::InternalVerification("xref encoding"))?;
    let lines: Vec<_> = xref.lines().collect();
    if lines.first() != Some(&"xref") || lines.get(1) != Some(&format!("0 {size}").as_str()) {
        return Err(FixtureError::InternalVerification("xref header"));
    }
    for record in records {
        let expected = format!("{:010} 00000 n ", record.offset);
        if lines.get(record.object_number as usize + 2) != Some(&expected.as_str()) {
            return Err(FixtureError::InternalVerification("xref entry"));
        }
    }
    let startxref_marker = b"startxref\n";
    let declared_startxref = bytes
        .windows(startxref_marker.len())
        .rposition(|window| window == startxref_marker)
        .map(|marker| marker + startxref_marker.len())
        .and_then(|start| {
            bytes[start..]
                .iter()
                .position(|byte| *byte == b'\n')
                .map(|length| &bytes[start..start + length])
        })
        .and_then(|value| std::str::from_utf8(value).ok())
        .and_then(|value| value.parse::<usize>().ok());
    if declared_startxref != Some(xref_offset) {
        return Err(FixtureError::InternalVerification("startxref value"));
    }
    Ok(())
}

fn write_entry(output: &mut Vec<u8>, key: &str, value: &[u8]) {
    output.push(b'/');
    output.extend_from_slice(key.as_bytes());
    output.push(b' ');
    output.extend_from_slice(value);
    output.push(b'\n');
}

fn valid_version(version: &str) -> bool {
    let bytes = version.as_bytes();
    bytes.len() == 3 && bytes[0].is_ascii_digit() && bytes[1] == b'.' && bytes[2].is_ascii_digit()
}

fn assert_simple_name(name: &str) {
    assert!(
        !name.is_empty()
            && name
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.')),
        "PDF fixture names must be simple ASCII names: {name:?}"
    );
}

/// Serialize a simple PDF name.
pub fn name(value: &str) -> Vec<u8> {
    assert_simple_name(value);
    let mut result = Vec::with_capacity(value.len() + 1);
    result.push(b'/');
    result.extend_from_slice(value.as_bytes());
    result
}

/// Serialize a generation-zero indirect reference.
pub fn reference(object_number: u32) -> Vec<u8> {
    assert!(object_number != 0, "object zero is the free-list head");
    format!("{object_number} 0 R").into_bytes()
}

/// Serialize an array of caller-supplied raw PDF values.
pub fn array<I, V>(values: I) -> Vec<u8>
where
    I: IntoIterator<Item = V>,
    V: AsRef<[u8]>,
{
    let mut result = vec![b'['];
    for (index, value) in values.into_iter().enumerate() {
        if index != 0 {
            result.push(b' ');
        }
        result.extend_from_slice(value.as_ref());
    }
    result.push(b']');
    result
}

/// Wrap raw PDF value syntax in exactly `depth` nested one-element arrays.
pub fn nested_array(depth: usize, leaf: impl AsRef<[u8]>) -> Vec<u8> {
    let leaf = leaf.as_ref();
    let mut result = Vec::with_capacity(depth.saturating_mul(2).saturating_add(leaf.len()));
    result.resize(depth, b'[');
    result.extend_from_slice(leaf);
    result.resize(result.len().saturating_add(depth), b']');
    result
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum FixtureError {
    InvalidVersion(String),
    ZeroObjectNumber,
    ObjectNumberTooLarge(u32),
    DuplicateObject(u32),
    ReservedStreamKey(&'static str),
    ReservedTrailerKey(&'static str),
    DuplicateTrailerKey(String),
    OffsetTooLarge(usize),
    InternalVerification(&'static str),
}

impl fmt::Display for FixtureError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidVersion(version) => write!(formatter, "invalid PDF version {version:?}"),
            Self::ZeroObjectNumber => formatter.write_str("object zero is reserved"),
            Self::ObjectNumberTooLarge(number) => {
                write!(
                    formatter,
                    "object number {number} cannot fit classic xref size"
                )
            }
            Self::DuplicateObject(number) => write!(formatter, "duplicate object {number}"),
            Self::ReservedStreamKey(key) => write!(formatter, "/{key} is writer-owned"),
            Self::ReservedTrailerKey(key) => write!(formatter, "/{key} is writer-owned"),
            Self::DuplicateTrailerKey(key) => write!(formatter, "duplicate trailer key /{key}"),
            Self::OffsetTooLarge(offset) => {
                write!(
                    formatter,
                    "offset {offset} does not fit a classic xref entry"
                )
            }
            Self::InternalVerification(kind) => {
                write!(formatter, "internal PDF fixture {kind} verification failed")
            }
        }
    }
}

impl Error for FixtureError {}
