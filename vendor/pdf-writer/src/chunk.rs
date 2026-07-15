use super::*;

/// Settings that should be applied while writing a PDF file.
#[derive(Debug, Clone, Copy)]
pub struct Settings {
    /// Whether to enable pretty-writing. In this case, `pdf-writer` will
    /// serialize PDFs in such a way that they are easier to read by humans by
    /// applying more padding and indentation, at the cost of larger file sizes.
    /// If disabled, `pdf-writer` will serialize objects as compactly as
    /// possible, leading to better file sizes but making it harder to inspect
    /// the file manually.
    ///
    /// _Default value_: `true`.
    pub pretty: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self { pretty: true }
    }
}

/// A builder for a collection of indirect PDF objects.
///
/// This type holds written top-level indirect PDF objects. Typically, you won't
/// create a colllection yourself, but use the primary chunk of the top-level
/// [`Pdf`] through its [`Deref`] implementation.
///
/// However, sometimes it's useful to be able to create a separate chunk to be
/// able to write two things at the same time (which isn't possible with a
/// single chunk because of the streaming nature --- only one writer can borrow
/// it at a time).
#[derive(Clone)]
pub struct Chunk {
    pub(crate) buf: Buf,
    pub(crate) entries: Vec<(Ref, XRefEntry)>,
    pub(crate) settings: Settings,
}

#[derive(Clone, Copy, Debug)]
pub(crate) enum XRefEntry {
    Free { next: usize, generation: u16 },
    InUse { offset: usize },
    Compressed { container: Ref, index: u16 },
}

impl Chunk {
    /// Create a new chunk with the default settings and buffer capacity
    /// (currently 1 KB).
    #[allow(clippy::new_without_default)]
    pub fn new() -> Self {
        Self::with_settings(Settings::default())
    }

    /// Create a new chunk with the given settings and the default buffer
    /// capacity (currently 1 KB).
    pub fn with_settings(settings: Settings) -> Self {
        Self::with_settings_and_capacity(settings, 1204)
    }

    /// Create a new chunk with the default settings and the specified initial
    /// capacity.
    pub fn with_capacity(capacity: usize) -> Self {
        Self::with_settings_and_capacity(Settings::default(), capacity)
    }

    /// Create a new chunk with the given settings and the specified initial
    /// buffer capacity.
    pub fn with_settings_and_capacity(settings: Settings, capacity: usize) -> Self {
        Self {
            buf: Buf::with_capacity(capacity),
            entries: vec![],
            settings,
        }
    }

    /// The number of bytes that were written so far.
    #[inline]
    #[allow(clippy::len_without_is_empty)]
    pub fn len(&self) -> usize {
        self.buf.len()
    }

    /// Reserve an additional number of bytes in the buffer.
    pub fn reserve(&mut self, additional: usize) {
        self.buf.reserve(additional);
    }

    /// The bytes already written so far.
    pub fn as_bytes(&self) -> &[u8] {
        self.buf.as_slice()
    }

    /// Add all objects from another chunk to this one.
    pub fn extend(&mut self, other: &Chunk) {
        let base = self.len();
        self.buf.extend_buf(&other.buf);
        self.entries.extend(other.entries.iter().map(|&(id, entry)| {
            let entry = match entry {
                free @ XRefEntry::Free { .. } => free,
                XRefEntry::InUse { offset } => XRefEntry::InUse { offset: base + offset },
                compressed @ XRefEntry::Compressed { .. } => compressed,
            };
            (id, entry)
        }));
    }

    /// An iterator over the references of the top-level objects
    /// of the chunk, in the order they appear in the chunk.
    pub fn refs(&self) -> impl ExactSizeIterator<Item = Ref> + '_ {
        self.entries.iter().map(|&(id, _)| id)
    }

    /// Returns the limits of data written into the chunk.
    pub fn limits(&self) -> &Limits {
        self.buf.limits()
    }

    /// Merges other limits into this chunk, taking the maximum of each field
    /// from the chunk's current [`limits()`](Self::limits) and `other`.
    ///
    /// This is, for instance, useful when adding a content stream (with limits
    /// of its own) to the chunk.
    ///
    /// ```
    /// use pdf_writer::{Chunk, Content, Ref};
    ///
    /// let mut content = Content::new();
    /// content.set_dash_pattern([1.0, 3.0, 2.0, 4.0, 5.0], 1.0);
    /// let buf = content.finish();
    ///
    /// let mut chunk = Chunk::new();
    /// chunk.stream(Ref::new(1), &buf);
    /// chunk.merge_limits(buf.limits());
    ///
    /// // Dash pattern had an array with 5 entries.
    /// assert_eq!(chunk.limits().array_len(), 5);
    /// ```
    pub fn merge_limits(&mut self, other: &Limits) {
        self.buf.limits.merge(other);
    }

    /// Renumbers the IDs of indirect objects and all indirect references in the
    /// chunk and returns the resulting chunk.
    ///
    /// The given closure is called for each object and indirect reference in
    /// the chunk. When an ID appears multiple times in the chunk (for object
    /// and/or reference), it will be called multiple times. When assigning new
    /// IDs, it is up to you to provide a well-defined mapping (it should most
    /// probably be a pure function so that a specific old ID is always mapped
    /// to the same new ID).
    ///
    /// A simple way to renumber a chunk is to map all old IDs to new
    /// consecutive IDs. This can be achieved by allocating a new ID for each
    /// unique ID we have seen and memoizing this mapping in a hash map:
    ///
    /// ```
    /// # use std::collections::HashMap;
    /// # use pdf_writer::{Chunk, Ref, TextStr, Name};
    /// let mut chunk = Chunk::new();
    /// chunk.indirect(Ref::new(10)).primitive(true);
    /// chunk.indirect(Ref::new(17))
    ///     .dict()
    ///     .pair(Name(b"Self"), Ref::new(17))
    ///     .pair(Name(b"Ref"), Ref::new(10))
    ///     .pair(Name(b"NoRef"), TextStr("Text with 10 0 R"));
    ///
    /// // Gives the objects consecutive IDs.
    /// // - The `true` object will get ID 1.
    /// // - The dictionary object will get ID 2.
    /// let mut alloc = Ref::new(1);
    /// let mut map = HashMap::new();
    /// let renumbered = chunk.renumber(|old| {
    ///     *map.entry(old).or_insert_with(|| alloc.bump())
    /// });
    /// ```
    ///
    /// If a chunk references indirect objects that are not defined within it,
    /// the closure is still called with those references. Allocating new IDs
    /// for them will probably not make sense, so it's up to you to either not
    /// have dangling references or handle them in a way that makes sense for
    /// your use case.
    pub fn renumber<F>(&self, mapping: F) -> Chunk
    where
        F: FnMut(Ref) -> Ref,
    {
        let mut chunk = Chunk::with_capacity(self.len());
        self.renumber_into(&mut chunk, mapping);
        chunk
    }

    /// Same as [`renumber`](Self::renumber), but writes the results into an
    /// existing `target` chunk instead of creating a new chunk.
    pub fn renumber_into<F>(&self, target: &mut Chunk, mut mapping: F)
    where
        F: FnMut(Ref) -> Ref,
    {
        target.buf.reserve(self.len());
        crate::renumber::renumber(self, target, &mut mapping);
    }
}

/// Indirect objects and streams.
impl Chunk {
    /// Start writing an indirectly referenceable object.
    pub fn indirect(&mut self, id: Ref) -> Obj<'_> {
        self.entries.push((id, XRefEntry::InUse { offset: self.buf.len() }));
        Obj::indirect(&mut self.buf, id, self.settings)
    }

    /// Start writing a PDF object stream.
    ///
    /// Each value added through [`ObjectStream::object`] is serialized by
    /// `pdf-writer` and registered as a compressed xref entry. Finish the PDF
    /// with [`Pdf::finish_with_xref_stream`](crate::Pdf::finish_with_xref_stream),
    /// since classic xref tables cannot represent compressed objects. PDF 1.5+.
    pub fn object_stream(&mut self, id: Ref) -> ObjectStream<'_> {
        ObjectStream {
            chunk: self,
            id,
            data: Buf::new(),
            objects: Vec::new(),
            finished: false,
        }
    }

    /// Start writing an indirectly referenceable stream.
    ///
    /// The stream data and the `/Length` field are written automatically. You
    /// can add additional key-value pairs to the stream dictionary with the
    /// returned stream writer.
    ///
    /// You can use this function together with a [`Content`] stream builder to
    /// provide a [page's contents](Page::contents).
    /// ```
    /// use pdf_writer::{Pdf, Content, Ref};
    ///
    /// // Create a simple content stream.
    /// let mut content = Content::new();
    /// content.rect(50.0, 50.0, 50.0, 50.0);
    /// content.stroke();
    ///
    /// // Create a writer and write the stream.
    /// let mut pdf = Pdf::new();
    /// pdf.stream(Ref::new(1), &content.finish());
    /// ```
    ///
    /// This crate does not do any compression for you. If you want to compress
    /// a stream, you have to pass already compressed data into this function
    /// and specify the appropriate filter in the stream dictionary.
    ///
    /// For example, if you want to compress your content stream with DEFLATE,
    /// you could do something like this:
    /// ```
    /// use pdf_writer::{Pdf, Content, Ref, Filter};
    /// use miniz_oxide::deflate::{compress_to_vec_zlib, CompressionLevel};
    ///
    /// // Create a simple content stream.
    /// let mut content = Content::new();
    /// content.rect(50.0, 50.0, 50.0, 50.0);
    /// content.stroke();
    ///
    /// // Compress the stream.
    /// let level = CompressionLevel::DefaultLevel as u8;
    /// let compressed = compress_to_vec_zlib(&content.finish(), level);
    ///
    /// // Create a writer, write the compressed stream and specify that it
    /// // needs to be decoded with a FLATE filter.
    /// let mut pdf = Pdf::new();
    /// pdf.stream(Ref::new(1), &compressed).filter(Filter::FlateDecode);
    /// ```
    /// For all the specialized stream functions below, it works the same way:
    /// You can pass compressed data and specify a filter.
    ///
    /// Panics if the stream length exceeds `i32::MAX`.
    pub fn stream<'a>(&'a mut self, id: Ref, data: &'a [u8]) -> Stream<'a> {
        Stream::start(self.indirect(id), data)
    }
}

/// A builder for a PDF object stream.
///
/// Object streams can contain arbitrary non-stream objects. The stream object
/// itself remains an ordinary in-use indirect object, while each contained
/// value receives a type-2 entry in the document's cross-reference stream.
pub struct ObjectStream<'a> {
    chunk: &'a mut Chunk,
    id: Ref,
    data: Buf,
    objects: Vec<(Ref, usize)>,
    finished: bool,
}

impl ObjectStream<'_> {
    /// Start writing one value into this object stream.
    ///
    /// Panics if more than 65,536 values are added, because `pdf-writer` uses
    /// a two-byte object-stream index field in cross-reference streams.
    pub fn object(&mut self, id: Ref) -> Obj<'_> {
        assert!(self.objects.len() <= usize::from(u16::MAX));
        if !self.objects.is_empty() {
            self.data.push(b'\n');
        }
        self.objects.push((id, self.data.len()));
        Obj::direct(&mut self.data, 0, self.chunk.settings, false)
    }

    /// Finish the object stream without filtering its bytes.
    pub fn finish(mut self) {
        self.write(None::<(Filter, fn(&[u8]) -> Vec<u8>)>);
        self.finished = true;
    }

    /// Finish the object stream after transforming its bytes with one filter.
    ///
    /// The closure receives the complete unfiltered object-stream data. This
    /// crate writes the corresponding `/Filter` entry through its stream API.
    pub fn finish_with_filter(
        mut self,
        filter: Filter,
        encode: impl FnOnce(&[u8]) -> Vec<u8>,
    ) {
        self.write(Some((filter, encode)));
        self.finished = true;
    }

    fn write<F>(&mut self, filter: Option<(Filter, F)>)
    where
        F: FnOnce(&[u8]) -> Vec<u8>,
    {
        let mut header = Buf::new();
        for &(id, offset) in &self.objects {
            header.push_int(id.get());
            header.push(b' ');
            header.push_int(
                i32::try_from(offset).expect("object stream offset exceeds i32"),
            );
            header.push(b' ');
        }
        header.push(b'\n');
        let first = header.len();
        header.extend_buf(&self.data);

        let (encoded, filter_name) = match filter {
            Some((filter, encode)) => (Some(encode(header.as_slice())), Some(filter)),
            None => (None, None),
        };
        let bytes = encoded.as_deref().unwrap_or_else(|| header.as_slice());
        let mut stream = self.chunk.stream(self.id, bytes);
        stream
            .pair(Name(b"Type"), Name(b"ObjStm"))
            .pair(
                Name(b"N"),
                i32::try_from(self.objects.len())
                    .expect("object stream count exceeds i32"),
            )
            .pair(
                Name(b"First"),
                i32::try_from(first).expect("object stream header exceeds i32"),
            );
        if let Some(filter) = filter_name {
            stream.filter(filter);
        }
        stream.finish();

        self.chunk.merge_limits(self.data.limits());
        self.chunk.entries.extend(self.objects.iter().enumerate().map(
            |(index, &(id, _))| {
                (
                    id,
                    XRefEntry::Compressed {
                        container: self.id,
                        index: u16::try_from(index)
                            .expect("object stream index exceeds u16"),
                    },
                )
            },
        ));
    }
}

impl Drop for ObjectStream<'_> {
    fn drop(&mut self) {
        if !self.finished {
            self.write(None::<(Filter, fn(&[u8]) -> Vec<u8>)>);
        }
    }
}

impl Debug for ObjectStream<'_> {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.pad("ObjectStream(..)")
    }
}

/// Document structure.
impl Chunk {
    /// Start writing a page tree.
    pub fn pages(&mut self, id: Ref) -> Pages<'_> {
        self.indirect(id).start()
    }

    /// Start writing a page.
    pub fn page(&mut self, id: Ref) -> Page<'_> {
        self.indirect(id).start()
    }

    /// Start writing an outline.
    pub fn outline(&mut self, id: Ref) -> Outline<'_> {
        self.indirect(id).start()
    }

    /// Start writing an outline item.
    pub fn outline_item(&mut self, id: Ref) -> OutlineItem<'_> {
        self.indirect(id).start()
    }

    /// Start writing an indirect action dictionary without `/Type /Action`.
    pub fn action(&mut self, id: Ref) -> Action<'_> {
        Action::start_without_type(self.indirect(id))
    }

    /// Start writing a destination for use in a name tree.
    pub fn destination(&mut self, id: Ref) -> Destination<'_> {
        self.indirect(id).start()
    }

    /// Start writing a named-destination dictionary.
    pub fn named_destination(&mut self, id: Ref) -> NamedDestination<'_> {
        self.indirect(id).start()
    }

    /// Start writing a named destination dictionary.
    pub fn destinations(&mut self, id: Ref) -> TypedDict<'_, Destination<'_>> {
        self.indirect(id).dict().typed()
    }

    /// Start writing an indirect catalog `/Names` dictionary.
    pub fn names(&mut self, id: Ref) -> Names<'_> {
        self.indirect(id).start()
    }

    /// Start writing an indirect article-thread reference array.
    pub fn thread_list(&mut self, id: Ref) -> ThreadList<'_> {
        self.indirect(id).start()
    }

    /// Start writing an article-thread dictionary.
    pub fn thread(&mut self, id: Ref) -> Thread<'_> {
        self.indirect(id).start()
    }

    /// Start writing an article bead dictionary.
    pub fn bead(&mut self, id: Ref) -> Bead<'_> {
        self.indirect(id).start()
    }

    /// Start writing a file specification dictionary.
    pub fn file_spec(&mut self, id: Ref) -> FileSpec<'_> {
        self.indirect(id).start()
    }

    /// Start writing an embedded file stream.
    pub fn embedded_file<'a>(&'a mut self, id: Ref, bytes: &'a [u8]) -> EmbeddedFile<'a> {
        EmbeddedFile::start(self.stream(id, bytes))
    }

    /// Start writing a structure tree element.
    pub fn struct_element(&mut self, id: Ref) -> StructElement<'_> {
        self.indirect(id).start()
    }

    /// Start writing a namespace dictionary. PDF 2.0+
    pub fn namespace(&mut self, id: Ref) -> Namespace<'_> {
        self.indirect(id).start()
    }

    /// Start writing a metadata stream.
    pub fn metadata<'a>(&'a mut self, id: Ref, bytes: &'a [u8]) -> Metadata<'a> {
        Metadata::start(self.stream(id, bytes))
    }
}

/// Graphics and content.
impl Chunk {
    /// Start writing an image XObject stream.
    ///
    /// The samples should be encoded according to the stream's filter, color
    /// space and bits per component.
    pub fn image_xobject<'a>(
        &'a mut self,
        id: Ref,
        samples: &'a [u8],
    ) -> ImageXObject<'a> {
        ImageXObject::start(self.stream(id, samples))
    }

    /// Start writing a form XObject stream.
    ///
    /// These can be used as transparency groups.
    ///
    /// Note that these have nothing to do with forms that have fields to fill
    /// out. Rather, they are a way to encapsulate and reuse content across the
    /// file.
    ///
    /// You can create the content bytes using a [`Content`] builder.
    pub fn form_xobject<'a>(&'a mut self, id: Ref, content: &'a [u8]) -> FormXObject<'a> {
        FormXObject::start(self.stream(id, content))
    }

    /// Start writing an external graphics state dictionary.
    pub fn ext_graphics(&mut self, id: Ref) -> ExtGraphicsState<'_> {
        self.indirect(id).start()
    }
}

/// Fonts.
impl Chunk {
    /// Start writing a Type-1 font.
    pub fn type1_font(&mut self, id: Ref) -> Type1Font<'_> {
        self.indirect(id).start()
    }

    /// Start writing a Type-3 font.
    pub fn type3_font(&mut self, id: Ref) -> Type3Font<'_> {
        self.indirect(id).start()
    }

    /// Start writing a Type-0 font.
    pub fn type0_font(&mut self, id: Ref) -> Type0Font<'_> {
        self.indirect(id).start()
    }

    /// Start writing a CID font.
    pub fn cid_font(&mut self, id: Ref) -> CidFont<'_> {
        self.indirect(id).start()
    }

    /// Start writing a font descriptor.
    pub fn font_descriptor(&mut self, id: Ref) -> FontDescriptor<'_> {
        self.indirect(id).start()
    }

    /// Start writing a character map stream.
    ///
    /// If you want to use this for a `/ToUnicode` CMap, you can create the
    /// bytes using a [`UnicodeCmap`](types::UnicodeCmap) builder.
    pub fn cmap<'a>(&'a mut self, id: Ref, cmap: &'a [u8]) -> Cmap<'a> {
        Cmap::start(self.stream(id, cmap))
    }
}

/// Color spaces, shadings and patterns.
impl Chunk {
    /// Start writing a color space.
    pub fn color_space(&mut self, id: Ref) -> ColorSpace<'_> {
        self.indirect(id).start()
    }

    /// Start writing a function-based shading (type 1-3).
    pub fn function_shading(&mut self, id: Ref) -> FunctionShading<'_> {
        self.indirect(id).start()
    }

    /// Start writing a stream-based shading (type 4-7).
    pub fn stream_shading<'a>(
        &'a mut self,
        id: Ref,
        content: &'a [u8],
    ) -> StreamShading<'a> {
        StreamShading::start(self.stream(id, content))
    }

    /// Start writing a tiling pattern stream.
    ///
    /// You can create the content bytes using a [`Content`] builder.
    pub fn tiling_pattern<'a>(
        &'a mut self,
        id: Ref,
        content: &'a [u8],
    ) -> TilingPattern<'a> {
        TilingPattern::start_with_stream(self.stream(id, content))
    }

    /// Start writing a shading pattern.
    pub fn shading_pattern(&mut self, id: Ref) -> ShadingPattern<'_> {
        self.indirect(id).start()
    }

    /// Start writing an ICC profile stream.
    ///
    /// The `profile` argument shall contain the ICC profile data conforming to
    /// ICC.1:2004-10 (PDF 1.7), ICC.1:2003-09 (PDF 1.6), ICC.1:2001-12 (PDF 1.5),
    /// ICC.1:1999-04 (PDF 1.4), or ICC 3.3 (PDF 1.3). Profile data is commonly
    /// compressed using the `FlateDecode` filter.
    pub fn icc_profile<'a>(&'a mut self, id: Ref, profile: &'a [u8]) -> IccProfile<'a> {
        IccProfile::start(self.stream(id, profile))
    }
}

/// Functions.
impl Chunk {
    /// Start writing a sampled function stream.
    pub fn sampled_function<'a>(
        &'a mut self,
        id: Ref,
        samples: &'a [u8],
    ) -> SampledFunction<'a> {
        SampledFunction::start(self.stream(id, samples))
    }

    /// Start writing an exponential function.
    pub fn exponential_function(&mut self, id: Ref) -> ExponentialFunction<'_> {
        self.indirect(id).start()
    }

    /// Start writing a stitching function.
    pub fn stitching_function(&mut self, id: Ref) -> StitchingFunction<'_> {
        self.indirect(id).start()
    }

    /// Start writing a PostScript function stream.
    ///
    /// You can create the code bytes using [`PostScriptOp::encode`](types::PostScriptOp::encode).
    pub fn post_script_function<'a>(
        &'a mut self,
        id: Ref,
        code: &'a [u8],
    ) -> PostScriptFunction<'a> {
        PostScriptFunction::start(self.stream(id, code))
    }
}

/// Tree data structures.
impl Chunk {
    /// Start writing a name tree node.
    pub fn name_tree<T: Primitive>(&mut self, id: Ref) -> NameTree<'_, T> {
        self.indirect(id).start()
    }

    /// Start writing a number tree node.
    pub fn number_tree<T: Primitive>(&mut self, id: Ref) -> NumberTree<'_, T> {
        self.indirect(id).start()
    }
}

/// Interactive features.
impl Chunk {
    /// Start writing an annotation dictionary.
    pub fn annotation(&mut self, id: Ref) -> Annotation<'_> {
        self.indirect(id).start()
    }

    /// Start writing a form field dictionary.
    pub fn form_field(&mut self, id: Ref) -> Field<'_> {
        self.indirect(id).start()
    }
}

impl Debug for Chunk {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.pad("Chunk(..)")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::ActionType;

    fn stream_data_after<'a>(pdf: &'a [u8], marker: &[u8]) -> &'a [u8] {
        let marker = memchr::memmem::find(pdf, marker).expect("stream marker");
        let stream =
            memchr::memmem::find(&pdf[marker..], b"stream\n").expect("stream data");
        let start = marker + stream + b"stream\n".len();
        let end = start
            + memchr::memmem::find(&pdf[start..], b"\nendstream").expect("endstream");
        &pdf[start..end]
    }

    #[test]
    fn test_chunk() {
        let mut w = Pdf::new();
        let mut font = w.type3_font(Ref::new(1));
        let mut c = Chunk::new();
        c.font_descriptor(Ref::new(2)).name(Name(b"MyFont"));
        font.font_descriptor(Ref::new(2));
        font.finish();
        w.extend(&c);
        test!(
            w.finish(),
            b"%PDF-1.7\n%\x80\x80\x80\x80\n",
            b"1 0 obj",
            b"<<\n  /Type /Font\n  /Subtype /Type3\n  /FontDescriptor 2 0 R\n>>",
            b"endobj\n",
            b"2 0 obj",
            b"<<\n  /Type /FontDescriptor\n  /FontName /MyFont\n>>",
            b"endobj\n",
            b"xref",
            b"0 3",
            b"0000000000 65535 f\r",
            b"0000000016 00000 n\r",
            b"0000000094 00000 n\r",
            b"trailer",
            b"<<\n  /Size 3\n>>",
            b"startxref\n160\n%%EOF",
        );
    }

    #[test]
    fn object_stream_registers_type_two_xref_entries() {
        let mut pdf = Pdf::with_settings(Settings { pretty: false });
        pdf.catalog(Ref::new(1)).pages(Ref::new(2));
        pdf.stream(Ref::new(4), b"ordinary stream");

        let mut objects = pdf.object_stream(Ref::new(6));
        objects
            .object(Ref::new(2))
            .dict()
            .pair(Name(b"Type"), Name(b"Pages"))
            .pair(Name(b"Count"), 0)
            .insert(Name(b"Kids"))
            .array();
        objects.object(Ref::new(3)).primitive(true);
        objects.finish();

        let bytes = pdf.finish_with_xref_stream(Ref::new(7));
        assert!(bytes.windows(12).any(|window| window == b"/Type/ObjStm"));
        assert!(bytes.windows(12).any(|window| window == b"ordinary str"));
        assert!(bytes.windows(9).any(|window| window == b"/W[1 1 2]"));

        let xref = stream_data_after(&bytes, b"/Type/XRef");
        assert_eq!(&xref[2 * 4..3 * 4], &[2, 6, 0, 0]);
        assert_eq!(&xref[3 * 4..4 * 4], &[2, 6, 0, 1]);
        assert_eq!(xref[4 * 4], 1, "ordinary stream has a type-1 entry");
        assert_eq!(xref[6 * 4], 1, "object stream has a type-1 entry");
    }

    #[test]
    fn object_stream_filter_receives_complete_writer_data() {
        use miniz_oxide::deflate::compress_to_vec_zlib;
        use miniz_oxide::inflate::decompress_to_vec_zlib;

        let mut pdf = Pdf::with_settings(Settings { pretty: false });
        let mut objects = pdf.object_stream(Ref::new(2));
        objects.object(Ref::new(1)).primitive(Str(b"deterministic"));
        objects.finish_with_filter(Filter::FlateDecode, |data| {
            compress_to_vec_zlib(data, 6)
        });
        let bytes = pdf.finish_with_xref_stream(Ref::new(3));

        assert!(bytes.windows(19).any(|window| window == b"/Filter/FlateDecode"));
        let encoded = stream_data_after(&bytes, b"/Type/ObjStm");
        let decoded = decompress_to_vec_zlib(encoded).expect("valid zlib data");
        assert_eq!(decoded, b"1 0 \n(deterministic)");
    }

    #[test]
    #[should_panic(expected = "compressed objects require a cross-reference stream")]
    fn object_stream_rejects_plain_xref_table() {
        let mut pdf = Pdf::new();
        pdf.object_stream(Ref::new(2)).object(Ref::new(1)).primitive(Null);
        pdf.finish();
    }

    #[test]
    fn dictionary_raw_entries_stay_inside_writer_framing() {
        let mut pdf = Pdf::new();
        let mut dict = pdf.indirect(Ref::new(1)).dict();
        dict.pair(Name(b"Typed"), true);
        dict.raw_entries(b"/Extension << /Value 7 >>");
        dict.finish();
        let bytes = pdf.finish();
        assert!(bytes
            .windows(b"/Typed true\n  /Extension << /Value 7 >>".len())
            .any(|window| window == b"/Typed true\n  /Extension << /Value 7 >>"));
    }

    #[test]
    fn navigation_writers_emit_exact_typed_objects() {
        let mut chunk = Chunk::with_settings(Settings { pretty: false });

        chunk
            .named_destination(Ref::new(1))
            .destination()
            .page(Ref::new(2))
            .xyz(10.0, 20.0, None);
        chunk
            .action(Ref::new(3))
            .action_type(ActionType::GoTo)
            .destination_pdftex_string(PdfStringSyntax(b"(dest)"));
        chunk
            .outline_item(Ref::new(4))
            .title_ref(Ref::new(5))
            .action_ref(Ref::new(3))
            .parent(Ref::new(14));
        chunk.indirect(Ref::new(5)).primitive(PdfStringSyntax(b"<FEFF0054>"));

        let mut names = chunk.name_tree::<Ref>(Ref::new(6));
        names.limits_pdftex(PdfStringSyntax(b"(dest)"), PdfStringSyntax(b"(dest)"));
        names.names().insert_pdftex(PdfStringSyntax(b"(dest)"), Ref::new(1));
        names.finish();

        chunk.thread_list(Ref::new(7)).threads([Ref::new(8)]);
        let mut thread = chunk.thread(Ref::new(8));
        thread.first_bead(Ref::new(9));
        thread.info().title_pdftex(PdfStringSyntax(b"(article)"));
        thread.finish();
        chunk
            .bead(Ref::new(9))
            .thread(Ref::new(8))
            .previous(Ref::new(9))
            .next(Ref::new(9))
            .page(Ref::new(12))
            .rectangle(Ref::new(10));
        chunk.indirect(Ref::new(10)).primitive(Rect::new(1.0, 2.0, 3.0, 4.0));
        Catalog::start(chunk.indirect(Ref::new(11)))
            .pages(Ref::new(13))
            .threads(Ref::new(7));
        chunk.page(Ref::new(12)).parent(Ref::new(13)).beads([Ref::new(9)]);

        assert_eq!(
            chunk.as_bytes(),
            b"1 0 obj\n<</D[2 0 R/XYZ 10 20 null]>>\nendobj\n\
3 0 obj\n<</S/GoTo/D(dest)>>\nendobj\n\
4 0 obj\n<</Title 5 0 R/A 3 0 R/Parent 14 0 R>>\nendobj\n\
5 0 obj\n<FEFF0054>\nendobj\n\
6 0 obj\n<</Limits[(dest)(dest)]/Names[(dest)1 0 R]>>\nendobj\n\
7 0 obj\n[8 0 R]\nendobj\n\
8 0 obj\n<</F 9 0 R/I<</Title(article)>>>>\nendobj\n\
9 0 obj\n<</T 8 0 R/V 9 0 R/N 9 0 R/P 12 0 R/R 10 0 R>>\nendobj\n\
10 0 obj\n[1 2 3 4]\nendobj\n\
11 0 obj\n<</Type/Catalog/Pages 13 0 R/Threads 7 0 R>>\nendobj\n\
12 0 obj\n<</Type/Page/Parent 13 0 R/B[9 0 R]>>\nendobj\n"
        );
    }

    #[test]
    fn navigation_writers_compose_inside_object_streams() {
        let mut pdf = Pdf::with_settings(Settings { pretty: false });
        let mut objects = pdf.object_stream(Ref::new(20));

        NamedDestination::start(objects.object(Ref::new(1)))
            .destination()
            .page(Ref::new(2))
            .fit();
        Action::start_without_type(objects.object(Ref::new(3)))
            .action_type(ActionType::GoTo)
            .destination_pdftex_string(PdfStringSyntax(b"(dest)"));
        Thread::start(objects.object(Ref::new(8))).first_bead(Ref::new(9));
        Bead::start(objects.object(Ref::new(9)))
            .thread(Ref::new(8))
            .previous(Ref::new(9))
            .next(Ref::new(9))
            .page(Ref::new(2))
            .rectangle(Ref::new(10));
        objects.finish();

        let bytes = pdf.finish_with_xref_stream(Ref::new(21));
        let decoded = stream_data_after(&bytes, b"/Type/ObjStm");
        assert!(decoded
            .windows(b"<</D[2 0 R/Fit]>>".len())
            .any(|window| window == b"<</D[2 0 R/Fit]>>"));
        assert!(decoded
            .windows(b"<</S/GoTo/D(dest)>>".len())
            .any(|window| window == b"<</S/GoTo/D(dest)>>"));
        assert!(decoded
            .windows(b"<</F 9 0 R>>".len())
            .any(|window| window == b"<</F 9 0 R>>"));
        assert!(decoded
            .windows(b"<</T 8 0 R/V 9 0 R/N 9 0 R/P 2 0 R/R 10 0 R>>".len())
            .any(|window| {
                window == b"<</T 8 0 R/V 9 0 R/N 9 0 R/P 2 0 R/R 10 0 R>>"
            }));
    }
}
