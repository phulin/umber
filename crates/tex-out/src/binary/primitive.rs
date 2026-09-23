use super::*;

pub(super) struct Writer {
    pub(super) bytes: Vec<u8>,
    pub(super) limits: ArtifactCodecLimits,
    pub(super) error: Option<SerializeError>,
    pub(super) nodes_seen: usize,
    pub(super) collection_items_seen: usize,
}

impl Writer {
    pub(super) fn new(limits: ArtifactCodecLimits) -> Self {
        Self {
            bytes: Vec::new(),
            limits,
            error: None,
            nodes_seen: 0,
            collection_items_seen: 0,
        }
    }

    pub(super) fn finish(self) -> Result<Vec<u8>, SerializeError> {
        if let Some(error) = self.error {
            return Err(error);
        }
        Ok(self.bytes)
    }

    pub(super) fn raw(&mut self, bytes: &[u8]) {
        if self.error.is_some() {
            return;
        }
        let Some(actual) = self.bytes.len().checked_add(bytes.len()) else {
            self.error = Some(SerializeError::LengthOverflow);
            return;
        };
        if actual > self.limits.max_bytes {
            self.error = Some(SerializeError::LimitExceeded {
                kind: CodecLimitKind::Bytes,
                actual,
                limit: self.limits.max_bytes,
            });
            return;
        }
        self.bytes.extend_from_slice(bytes);
    }

    pub(super) fn u8(&mut self, value: u8) {
        self.raw(&[value]);
    }

    pub(super) fn optional_u8(&mut self, value: Option<u8>) {
        match value {
            Some(value) => {
                self.u8(1);
                self.u8(value);
            }
            None => self.u8(0),
        }
    }

    pub(super) fn u32(&mut self, value: u32) {
        self.raw(&value.to_le_bytes());
    }

    pub(super) fn u16(&mut self, value: u16) {
        self.raw(&value.to_le_bytes());
    }

    pub(super) fn i32(&mut self, value: i32) {
        self.raw(&value.to_le_bytes());
    }

    pub(super) fn scaled(&mut self, value: Scaled) {
        self.i32(value.raw());
    }

    pub(super) fn len(&mut self, len: usize) {
        match u32::try_from(len) {
            Ok(len) => self.u32(len),
            Err(_) => self.error = Some(SerializeError::LengthOverflow),
        }
    }

    pub(super) fn collection_len(&mut self, len: usize) {
        if len > self.limits.max_collection_len {
            self.error = Some(SerializeError::LimitExceeded {
                kind: CodecLimitKind::CollectionLength,
                actual: len,
                limit: self.limits.max_collection_len,
            });
            return;
        }
        let Some(actual) = self.collection_items_seen.checked_add(len) else {
            self.error = Some(SerializeError::LengthOverflow);
            return;
        };
        if actual > self.limits.max_collection_items {
            self.error = Some(SerializeError::LimitExceeded {
                kind: CodecLimitKind::CollectionItems,
                actual,
                limit: self.limits.max_collection_items,
            });
            return;
        }
        self.collection_items_seen = actual;
        self.len(len);
    }

    pub(super) fn bytes(&mut self, bytes: &[u8]) {
        if bytes.len() > self.limits.max_collection_len {
            self.error = Some(SerializeError::LimitExceeded {
                kind: CodecLimitKind::CollectionLength,
                actual: bytes.len(),
                limit: self.limits.max_collection_len,
            });
            return;
        }
        self.len(bytes.len());
        self.raw(bytes);
    }

    pub(super) fn str(&mut self, value: &str) {
        self.bytes(value.as_bytes());
    }

    pub(super) fn optional_scaled(&mut self, value: Option<Scaled>) {
        match value {
            Some(value) => {
                self.u8(1);
                self.scaled(value);
            }
            None => self.u8(0),
        }
    }
}

pub(super) struct Reader<'a> {
    pub(super) bytes: &'a [u8],
    pub(super) offset: usize,
    pub(super) limits: ArtifactCodecLimits,
    pub(super) nodes_seen: usize,
    pub(super) collection_items_seen: usize,
}

impl Reader<'_> {
    pub(super) fn new(bytes: &[u8], limits: ArtifactCodecLimits) -> Reader<'_> {
        Self::new_at(bytes, limits, 0)
    }

    pub(super) fn new_at(bytes: &[u8], limits: ArtifactCodecLimits, offset: usize) -> Reader<'_> {
        Reader {
            bytes,
            offset,
            limits,
            nodes_seen: 0,
            collection_items_seen: 0,
        }
    }

    pub(super) fn header(
        &mut self,
    ) -> Result<(crate::JobInfo, Vec<FontResource>, [i32; 10]), ParseError> {
        self.expect_header()?;
        let mag = self.i32()?;
        let banner = self.str()?;
        let h_offset = self.scaled()?;
        let v_offset = self.scaled()?;
        let (page_origin_x, page_origin_y, page_width, page_height) = (
            self.scaled()?,
            self.scaled()?,
            self.scaled()?,
            self.scaled()?,
        );
        let job = crate::JobInfo {
            mag,
            banner,
            h_offset,
            v_offset,
            page_origin_x,
            page_origin_y,
            page_width,
            page_height,
        };
        let fonts = self.fonts()?;
        let mut counts = [0; 10];
        for value in &mut counts {
            *value = self.i32()?;
        }
        Ok((job, fonts, counts))
    }

    pub(super) fn expect_header(&mut self) -> Result<(), ParseError> {
        self.expect_magic()?;
        let version = self.u8()?;
        if version != VERSION {
            return Err(ParseError::UnsupportedVersion(version));
        }
        Ok(())
    }

    fn expect_magic(&mut self) -> Result<(), ParseError> {
        let magic = self.take(MAGIC.len())?;
        if magic == MAGIC {
            Ok(())
        } else {
            Err(ParseError::InvalidMagic)
        }
    }

    pub(super) fn finish(&self) -> Result<(), ParseError> {
        if self.offset == self.bytes.len() {
            Ok(())
        } else {
            Err(ParseError::TrailingBytes {
                offset: self.offset,
                len: self.bytes.len(),
            })
        }
    }

    pub(super) fn take(&mut self, len: usize) -> Result<&[u8], ParseError> {
        let end = self
            .offset
            .checked_add(len)
            .ok_or(ParseError::LengthOverflow)?;
        if end > self.bytes.len() {
            return Err(ParseError::UnexpectedEof);
        }
        let bytes = &self.bytes[self.offset..end];
        self.offset = end;
        Ok(bytes)
    }

    pub(super) fn u8(&mut self) -> Result<u8, ParseError> {
        Ok(self.take(1)?[0])
    }

    pub(super) fn optional_u8(&mut self, kind: &'static str) -> Result<Option<u8>, ParseError> {
        match self.u8()? {
            0 => Ok(None),
            1 => Ok(Some(self.u8()?)),
            tag => Err(ParseError::InvalidTag { kind, tag }),
        }
    }

    pub(super) fn u32(&mut self) -> Result<u32, ParseError> {
        let mut bytes = [0; 4];
        bytes.copy_from_slice(self.take(4)?);
        Ok(u32::from_le_bytes(bytes))
    }

    pub(super) fn u16(&mut self) -> Result<u16, ParseError> {
        let mut bytes = [0; 2];
        bytes.copy_from_slice(self.take(2)?);
        Ok(u16::from_le_bytes(bytes))
    }

    pub(super) fn i32(&mut self) -> Result<i32, ParseError> {
        let mut bytes = [0; 4];
        bytes.copy_from_slice(self.take(4)?);
        Ok(i32::from_le_bytes(bytes))
    }

    pub(super) fn scaled(&mut self) -> Result<Scaled, ParseError> {
        Ok(Scaled::from_raw(self.i32()?))
    }

    pub(super) fn len(&mut self) -> Result<usize, ParseError> {
        usize::try_from(self.u32()?).map_err(|_| ParseError::LengthOverflow)
    }

    pub(super) fn collection_len(&mut self, min_item_bytes: usize) -> Result<usize, ParseError> {
        let len = self.len()?;
        if len > self.limits.max_collection_len {
            return Err(ParseError::LimitExceeded {
                kind: CodecLimitKind::CollectionLength,
                actual: len,
                limit: self.limits.max_collection_len,
            });
        }
        self.collection_items_seen = self
            .collection_items_seen
            .checked_add(len)
            .ok_or(ParseError::LengthOverflow)?;
        if self.collection_items_seen > self.limits.max_collection_items {
            return Err(ParseError::LimitExceeded {
                kind: CodecLimitKind::CollectionItems,
                actual: self.collection_items_seen,
                limit: self.limits.max_collection_items,
            });
        }
        let minimum_bytes = len
            .checked_mul(min_item_bytes)
            .ok_or(ParseError::LengthOverflow)?;
        if minimum_bytes > self.bytes.len() - self.offset {
            return Err(ParseError::UnexpectedEof);
        }
        Ok(len)
    }

    pub(super) fn bytes(&mut self) -> Result<Vec<u8>, ParseError> {
        Ok(self.bytes_ref()?.to_vec())
    }

    pub(super) fn bytes_ref(&mut self) -> Result<&[u8], ParseError> {
        let len = self.len()?;
        if len > self.limits.max_collection_len {
            return Err(ParseError::LimitExceeded {
                kind: CodecLimitKind::CollectionLength,
                actual: len,
                limit: self.limits.max_collection_len,
            });
        }
        self.take(len)
    }

    pub(super) fn str(&mut self) -> Result<String, ParseError> {
        Ok(self.str_ref()?.to_owned())
    }

    pub(super) fn str_ref(&mut self) -> Result<&str, ParseError> {
        std::str::from_utf8(self.bytes_ref()?).map_err(|_| ParseError::InvalidUtf8)
    }

    pub(super) fn optional_scaled(&mut self) -> Result<Option<Scaled>, ParseError> {
        match self.u8()? {
            0 => Ok(None),
            1 => Ok(Some(self.scaled()?)),
            tag => Err(ParseError::InvalidTag {
                kind: "optional scaled",
                tag,
            }),
        }
    }
}
