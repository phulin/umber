use std::fmt;
use std::fmt::Write as _;
use std::sync::Arc;

use bib_model::{
    BibDiagnostic, BibDiagnosticCode, BibSeverity, DateValue, Entry, Field, FieldValue,
    GeneratedFile, Name, NamePartValue, OptionValue, OutputFormat, OutputNewline, OutputRequest,
    Range, RangeEndpoint,
};
use bib_unicode::{EncodingError, LegacyEncoding, encode_legacy, normalise_nfc};

use crate::{OutputContext, Serializer};

pub const BIBLATEX_XML_NAMESPACE: &str = "http://biblatex-biber.sourceforge.net/biblatexml";
pub const BBL_XML_NAMESPACE: &str = "https://sourceforge.net/projects/biblatex/bblxml";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum XmlOutputFailureKind {
    WrongFormat,
    IncompatibleVersion,
    MalformedValue,
    Unrepresentable,
    Limit,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct XmlOutputFailure {
    kind: XmlOutputFailureKind,
    diagnostics: Arc<[BibDiagnostic]>,
}

impl XmlOutputFailure {
    #[must_use]
    pub const fn kind(&self) -> XmlOutputFailureKind {
        self.kind
    }
    pub fn diagnostics(&self) -> impl ExactSizeIterator<Item = &BibDiagnostic> {
        self.diagnostics.iter()
    }
}

impl fmt::Display for XmlOutputFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(
            self.diagnostics
                .first()
                .map_or("XML output failed", BibDiagnostic::message),
        )
    }
}
impl std::error::Error for XmlOutputFailure {}

#[derive(Clone, Copy, Debug, Default)]
pub struct BibLatexXmlSerializer;

#[derive(Clone, Copy, Debug, Default)]
pub struct BblXmlSerializer;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum XmlSchemaKind {
    BibLatex,
    Bbl,
}

impl Serializer for BibLatexXmlSerializer {
    type Error = XmlOutputFailure;
    fn serialize(
        &self,
        context: OutputContext<'_>,
        request: &OutputRequest,
    ) -> Result<GeneratedFile, Self::Error> {
        validate(context, request, OutputFormat::BibLatexXml, "BibLaTeXML")?;
        let schema = schema_file_name(request.path().as_str());
        let mut xml = XmlWriter::new(request.max_bytes());
        xml.raw(&header(
            "bltx",
            &schema,
            "biblatexml",
            BIBLATEX_XML_NAMESPACE,
        ))?;
        for section in context.document().sections() {
            for entry in section.entries() {
                write_biblatexml_entry(
                    &mut xml,
                    entry,
                    section.aliases().filter_map(|(alias, target)| {
                        (target == entry.id()).then_some(alias.as_str())
                    }),
                )?;
            }
        }
        xml.raw("</bltx:entries>\n")?;
        finish(xml, request)
    }
}

impl Serializer for BblXmlSerializer {
    type Error = XmlOutputFailure;
    fn serialize(
        &self,
        context: OutputContext<'_>,
        request: &OutputRequest,
    ) -> Result<GeneratedFile, Self::Error> {
        validate(context, request, OutputFormat::BblXml, "BBLXML")?;
        let schema = schema_file_name(request.path().as_str());
        let mut xml = XmlWriter::new(request.max_bytes());
        xml.raw(&header("bbl", &schema, "bblxml", BBL_XML_NAMESPACE))?;
        for section in context.document().sections() {
            xml.line(1, &format!("<bbl:refsection number=\"{}\">", section.id()))?;
            for list in section.lists() {
                xml.line(
                    2,
                    &format!(
                        "<bbl:datalist name=\"{}\" type=\"{}\">",
                        attr(list.id().as_str())?,
                        match list.kind() {
                            bib_model::DataListKind::Entry => "entry",
                            bib_model::DataListKind::List => "list",
                        }
                    ),
                )?;
                for item in list.items() {
                    let entry = section.entry(item.entry()).ok_or_else(|| {
                        failure(
                            XmlOutputFailureKind::MalformedValue,
                            "BIB_XML_UNKNOWN_ENTRY",
                            "BBLXML data list references an unknown entry",
                        )
                    })?;
                    write_bblxml_entry(&mut xml, entry)?;
                }
                xml.line(2, "</bbl:datalist>")?;
            }
            for (alias, target) in section.aliases() {
                xml.line(
                    2,
                    &format!(
                        "<bbl:keyalias key=\"{}\" target=\"{}\" />",
                        attr(alias.as_str())?,
                        attr(target.as_str())?
                    ),
                )?;
            }
            for key in section.undefined_keys() {
                xml.line(
                    2,
                    &format!("<bbl:missing key=\"{}\" />", attr(key.as_str())?),
                )?;
            }
            xml.line(1, "</bbl:refsection>")?;
        }
        xml.raw("</bbl:refsections>\n")?;
        finish(xml, request)
    }
}

/// Generates the deterministic Relax NG companion for the active frozen data model.
pub fn generate_xml_schema(
    context: OutputContext<'_>,
    request: &OutputRequest,
    kind: XmlSchemaKind,
) -> Result<GeneratedFile, XmlOutputFailure> {
    if context.document().configuration().version() != context.unicode().compatibility() {
        return Err(failure(
            XmlOutputFailureKind::IncompatibleVersion,
            "BIB_XML_VERSION",
            "the processed document and Unicode tables are incompatible",
        ));
    }
    let (prefix, namespace, root) = match kind {
        XmlSchemaKind::BibLatex => ("bltx", BIBLATEX_XML_NAMESPACE, "entries"),
        XmlSchemaKind::Bbl => ("bbl", BBL_XML_NAMESPACE, "refsections"),
    };
    let mut fields = context
        .document()
        .sections()
        .flat_map(|section| section.entries())
        .flat_map(|entry| entry.fields().iter())
        .map(|field| field.id().as_str())
        .collect::<Vec<_>>();
    fields.sort_unstable();
    fields.dedup();
    let mut schema = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<grammar xmlns=\"http://relaxng.org/ns/structure/1.0\" ns=\"{namespace}\" datatypeLibrary=\"http://www.w3.org/2001/XMLSchema-datatypes\">\n  <start><ref name=\"{prefix}:{root}\" /></start>\n  <define name=\"{prefix}:{root}\"><element name=\"{root}\"><zeroOrMore><choice>\n"
    );
    for field in fields {
        writeln!(
            schema,
            "    <element name=\"{}\"><text /></element>",
            attr(field)?
        )
        .expect("writing a String cannot fail");
    }
    schema.push_str("  </choice></zeroOrMore></element></define>\n</grammar>\n");
    encode_result(schema, request)
}

fn validate(
    context: OutputContext<'_>,
    request: &OutputRequest,
    expected: OutputFormat,
    label: &str,
) -> Result<(), XmlOutputFailure> {
    if request.format() != expected {
        return Err(failure(
            XmlOutputFailureKind::WrongFormat,
            "BIB_XML_FORMAT",
            &format!("the {label} serializer requires a {label} output request"),
        ));
    }
    if context.document().configuration().version() != context.unicode().compatibility() {
        return Err(failure(
            XmlOutputFailureKind::IncompatibleVersion,
            "BIB_XML_VERSION",
            "the processed document and Unicode tables are incompatible",
        ));
    }
    Ok(())
}

fn header(prefix: &str, schema: &str, format: &str, namespace: &str) -> String {
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<?xml-model href=\"{schema}\" type=\"application/xml\" schematypens=\"http://relaxng.org/ns/structure/1.0\"?>\n<!-- Auto-generated by Biber::Output::{format} -->\n\n<{prefix}:{} xmlns:{prefix}=\"{namespace}\">\n",
        if prefix == "bltx" {
            "entries"
        } else {
            "refsections"
        }
    )
}

fn write_biblatexml_entry<'a>(
    xml: &mut XmlWriter,
    entry: &Entry,
    aliases: impl Iterator<Item = &'a str>,
) -> Result<(), XmlOutputFailure> {
    xml.line(
        1,
        &format!(
            "<bltx:entry id=\"{}\" entrytype=\"{}\">",
            attr(entry.id().as_str())?,
            attr(entry.entry_type().as_str())?
        ),
    )?;
    let aliases = aliases.collect::<Vec<_>>();
    if !aliases.is_empty() {
        xml.line(2, "<bltx:ids>")?;
        xml.line(3, "<bltx:list>")?;
        for alias in aliases {
            element(xml, 4, "bltx:item", alias)?;
        }
        xml.line(3, "</bltx:list>")?;
        xml.line(2, "</bltx:ids>")?;
    }
    let mut options = entry
        .options()
        .layers()
        .flat_map(|layer| layer.iter())
        .map(|(id, value)| format!("{}={}", id, option(value)))
        .collect::<Vec<_>>();
    options.sort();
    options.dedup_by(|a, b| a.split('=').next() == b.split('=').next());
    if !options.is_empty() {
        element(xml, 2, "bltx:options", &options.join(","))?;
    }
    let mut fields = entry.fields().iter().collect::<Vec<_>>();
    fields.sort_by(|a, b| {
        field_rank(a)
            .cmp(&field_rank(b))
            .then_with(|| a.id().cmp(b.id()))
    });
    for field in fields {
        write_biblatexml_field(xml, field)?;
    }
    for annotation in entry.annotations() {
        let mut attrs = String::new();
        if let Some(field) = annotation.field() {
            write!(attrs, " field=\"{}\"", attr(field.as_str())?)
                .expect("writing a String cannot fail");
        }
        write!(
            attrs,
            " name=\"{}\" literal=\"0\"",
            attr(annotation.name().as_str())?
        )
        .expect("writing a String cannot fail");
        element_attrs(xml, 2, "bltx:annotation", &attrs, annotation.value())?;
    }
    xml.line(1, "</bltx:entry>")
}

fn field_rank(field: &Field) -> u8 {
    match field.value() {
        FieldValue::NameList(_) => 0,
        FieldValue::LiteralList(_) | FieldValue::KeyList(_) | FieldValue::UriList(_) => 1,
        FieldValue::Literal(_)
        | FieldValue::Verbatim(_)
        | FieldValue::Integer(_)
        | FieldValue::Boolean(_) => 2,
        FieldValue::RangeList(_) => 3,
        FieldValue::Date(_) => 4,
    }
}

fn write_biblatexml_field(xml: &mut XmlWriter, field: &Field) -> Result<(), XmlOutputFailure> {
    let id = field.id().as_str();
    match field.value() {
        FieldValue::NameList(names) => {
            xml.line(
                2,
                &format!(
                    "<bltx:names type=\"{}\"{}>",
                    attr(id)?,
                    if names.has_others() {
                        " morenames=\"1\""
                    } else {
                        ""
                    }
                ),
            )?;
            for name in names.iter() {
                write_xml_name(xml, name)?;
            }
            xml.line(2, "</bltx:names>")
        }
        FieldValue::LiteralList(values) => {
            write_xml_list(xml, id, values.iter().map(|v| v.as_str()))
        }
        FieldValue::KeyList(values) => write_xml_list(xml, id, values.iter().map(|v| v.as_str())),
        FieldValue::UriList(values) => write_xml_list(xml, id, values.iter().map(|v| v.as_str())),
        FieldValue::RangeList(values) => {
            xml.line(2, &format!("<bltx:{}>", attr(id)?))?;
            xml.line(3, "<bltx:list>")?;
            for range in values {
                xml.line(4, "<bltx:item>")?;
                element(xml, 5, "bltx:start", &endpoint(range.start()))?;
                element(xml, 5, "bltx:end", &endpoint(range.end()))?;
                xml.line(4, "</bltx:item>")?;
            }
            xml.line(3, "</bltx:list>")?;
            xml.line(2, &format!("</bltx:{id}>"))
        }
        FieldValue::Date(date) => {
            let (tag, attrs) = if id == "date" {
                ("bltx:date", "".to_owned())
            } else if let Some(kind) = id.strip_suffix("date") {
                ("bltx:date", format!(" type=\"{}\"", attr(kind)?))
            } else {
                (id, String::new())
            };
            element_attrs(xml, 2, tag, &attrs, &date_text(date))
        }
        FieldValue::Literal(v) => element(xml, 2, &format!("bltx:{id}"), v.as_str()),
        FieldValue::Verbatim(v) => element(xml, 2, &format!("bltx:{id}"), v.as_str()),
        FieldValue::Integer(v) => element(xml, 2, &format!("bltx:{id}"), &v.to_string()),
        FieldValue::Boolean(v) => element(
            xml,
            2,
            &format!("bltx:{id}"),
            if *v { "true" } else { "false" },
        ),
    }
}

fn write_xml_name(xml: &mut XmlWriter, name: &Name) -> Result<(), XmlOutputFailure> {
    let mut attrs = String::new();
    if let Some(v) = name.use_prefix() {
        write!(attrs, " useprefix=\"{}\"", if v { "true" } else { "false" })
            .expect("writing a String cannot fail");
    }
    if let Some(v) = name.sorting_name_key_template() {
        write!(attrs, " sortingnamekeytemplatename=\"{}\"", attr(v)?)
            .expect("writing a String cannot fail");
    }
    xml.line(3, &format!("<bltx:name{attrs}>"))?;
    for (kind, part) in [
        ("family", name.family()),
        ("given", name.given()),
        ("prefix", name.prefix()),
        ("suffix", name.suffix()),
    ] {
        if let Some(part) = part {
            write_name_part(xml, kind, part)?;
        }
    }
    xml.line(3, "</bltx:name>")
}
fn write_name_part(
    xml: &mut XmlWriter,
    kind: &str,
    part: &NamePartValue,
) -> Result<(), XmlOutputFailure> {
    let initials = part.initials().collect::<Vec<_>>();
    let ia = if initials.is_empty() {
        String::new()
    } else {
        format!(" initial=\"{}\"", attr(&initials.join(" "))?)
    };
    element_attrs(
        xml,
        4,
        "bltx:namepart",
        &format!(" type=\"{kind}\"{ia}"),
        part.value().as_str(),
    )
}
fn write_xml_list<'a>(
    xml: &mut XmlWriter,
    id: &str,
    values: impl Iterator<Item = &'a str>,
) -> Result<(), XmlOutputFailure> {
    xml.line(2, &format!("<bltx:{}>", attr(id)?))?;
    xml.line(3, "<bltx:list>")?;
    for value in values {
        element(xml, 4, "bltx:item", value)?;
    }
    xml.line(3, "</bltx:list>")?;
    xml.line(2, &format!("</bltx:{id}>"))
}

fn write_bblxml_entry(xml: &mut XmlWriter, entry: &Entry) -> Result<(), XmlOutputFailure> {
    xml.line(
        3,
        &format!(
            "<bbl:entry key=\"{}\" type=\"{}\">",
            attr(entry.id().as_str())?,
            attr(entry.entry_type().as_str())?
        ),
    )?;
    for field in entry.fields().iter() {
        write_bblxml_field(xml, field)?;
    }
    xml.line(3, "</bbl:entry>")
}
fn write_bblxml_field(xml: &mut XmlWriter, field: &Field) -> Result<(), XmlOutputFailure> {
    let name = attr(field.id().as_str())?;
    match field.value() {
        FieldValue::NameList(v) => {
            xml.line(
                4,
                &format!(
                    "<bbl:names type=\"{name}\"{}>",
                    if v.has_others() { " more=\"true\"" } else { "" }
                ),
            )?;
            for n in v.iter() {
                xml.line(5, "<bbl:name>")?;
                for (k, p) in [
                    ("family", n.family()),
                    ("given", n.given()),
                    ("prefix", n.prefix()),
                    ("suffix", n.suffix()),
                ] {
                    if let Some(p) = p {
                        element_attrs(
                            xml,
                            6,
                            "bbl:namepart",
                            &format!(" type=\"{k}\""),
                            p.value().as_str(),
                        )?;
                    }
                }
                xml.line(5, "</bbl:name>")?;
            }
            xml.line(4, "</bbl:names>")
        }
        FieldValue::LiteralList(v) => bbl_list(xml, &name, v.iter().map(|x| x.as_str())),
        FieldValue::KeyList(v) => bbl_list(xml, &name, v.iter().map(|x| x.as_str())),
        FieldValue::UriList(v) => bbl_list(xml, &name, v.iter().map(|x| x.as_str())),
        FieldValue::RangeList(v) => bbl_list(
            xml,
            &name,
            v.iter()
                .map(range_text)
                .collect::<Vec<_>>()
                .iter()
                .map(String::as_str),
        ),
        _ => element_attrs(
            xml,
            4,
            "bbl:field",
            &format!(" name=\"{name}\""),
            &scalar(field.value()),
        ),
    }
}
fn bbl_list<'a>(
    xml: &mut XmlWriter,
    name: &str,
    values: impl Iterator<Item = &'a str>,
) -> Result<(), XmlOutputFailure> {
    xml.line(4, &format!("<bbl:list name=\"{name}\">"))?;
    for v in values {
        element(xml, 5, "bbl:item", v)?;
    }
    xml.line(4, "</bbl:list>")
}

fn scalar(v: &FieldValue) -> String {
    match v {
        FieldValue::Literal(x) => x.as_str().into(),
        FieldValue::Verbatim(x) => x.as_str().into(),
        FieldValue::Integer(x) => x.to_string(),
        FieldValue::Boolean(x) => x.to_string(),
        FieldValue::Date(x) => date_text(x),
        _ => String::new(),
    }
}
fn option(v: &OptionValue) -> String {
    match v {
        OptionValue::Boolean(x) => x.to_string(),
        OptionValue::Integer(x) => x.to_string(),
        OptionValue::String(x) => x.clone(),
        OptionValue::Strings(x) => x.join(","),
    }
}
fn date_text(v: &DateValue) -> String {
    let mut s = format!("{:04}", v.year());
    if let Some(m) = v.month() {
        write!(s, "-{m:02}").expect("writing a String cannot fail");
    }
    if let Some(d) = v.day() {
        write!(s, "-{d:02}").expect("writing a String cannot fail");
    }
    if v.is_uncertain() {
        s.push('?')
    }
    if v.is_approximate() {
        s.push('~')
    }
    s
}
fn endpoint(v: &RangeEndpoint) -> String {
    match v {
        RangeEndpoint::Integer(x) => x.to_string(),
        RangeEndpoint::Literal(x) => x.as_str().into(),
        RangeEndpoint::Open => String::new(),
    }
}
fn range_text(v: &Range) -> String {
    format!("{}--{}", endpoint(v.start()), endpoint(v.end()))
}
fn schema_file_name(path: &str) -> String {
    let name = path.rsplit('/').next().unwrap_or(path);
    match name.rsplit_once('.') {
        Some((stem, _)) => format!("{stem}.rng"),
        None => format!("{name}.rng"),
    }
}

fn element(
    xml: &mut XmlWriter,
    indent: usize,
    tag: &str,
    text: &str,
) -> Result<(), XmlOutputFailure> {
    element_attrs(xml, indent, tag, "", text)
}
fn element_attrs(
    xml: &mut XmlWriter,
    indent: usize,
    tag: &str,
    attrs: &str,
    text: &str,
) -> Result<(), XmlOutputFailure> {
    xml.line(indent, &format!("<{tag}{attrs}>{}</{tag}>", escape(text)?))
}
fn attr(value: &str) -> Result<String, XmlOutputFailure> {
    escape_impl(value, true)
}
fn escape(value: &str) -> Result<String, XmlOutputFailure> {
    escape_impl(value, false)
}
fn escape_impl(value: &str, attribute: bool) -> Result<String, XmlOutputFailure> {
    if value
        .chars()
        .any(|c| c == '\0' || matches!(c,'\u{1}'..='\u{8}'|'\u{b}'|'\u{c}'|'\u{e}'..='\u{1f}'))
    {
        return Err(failure(
            XmlOutputFailureKind::MalformedValue,
            "BIB_XML_CHARACTER",
            "XML output contains a forbidden control character",
        ));
    }
    let mut out = normalise_nfc(value)
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;");
    if attribute {
        out = out.replace('"', "&quot;");
    }
    Ok(out)
}

struct XmlWriter {
    value: String,
    limit: usize,
}
impl XmlWriter {
    fn new(limit: usize) -> Self {
        Self {
            value: String::new(),
            limit,
        }
    }
    fn raw(&mut self, s: &str) -> Result<(), XmlOutputFailure> {
        if self.value.len().saturating_add(s.len()) > self.limit.saturating_mul(4) {
            return Err(limit(self.limit));
        }
        self.value.push_str(s);
        Ok(())
    }
    fn line(&mut self, indent: usize, s: &str) -> Result<(), XmlOutputFailure> {
        self.raw(&"  ".repeat(indent))?;
        self.raw(s)?;
        self.raw("\n")
    }
}
fn finish(xml: XmlWriter, request: &OutputRequest) -> Result<GeneratedFile, XmlOutputFailure> {
    encode_result(xml.value, request)
}
fn encode_result(
    mut text: String,
    request: &OutputRequest,
) -> Result<GeneratedFile, XmlOutputFailure> {
    if request.encoding() != LegacyEncoding::Utf8 {
        text = text.replacen(
            "encoding=\"UTF-8\"",
            &format!("encoding=\"{}\"", encoding_name(request.encoding())),
            1,
        );
    }
    if request.newline() == OutputNewline::CrLf {
        text = text.replace('\n', "\r\n");
    }
    let bytes = encode_legacy(&text, request.encoding()).map_err(|e| match e {
        EncodingError::UnmappableCharacter => failure(
            XmlOutputFailureKind::Unrepresentable,
            "BIB_XML_ENCODING",
            "XML output contains a character unavailable in the requested encoding",
        ),
        _ => failure(
            XmlOutputFailureKind::MalformedValue,
            "BIB_XML_ENCODING",
            "the requested XML encoding is invalid",
        ),
    })?;
    if bytes.len() > request.max_bytes() {
        return Err(limit(request.max_bytes()));
    }
    Ok(GeneratedFile::new(request.path().clone(), bytes))
}
fn encoding_name(v: LegacyEncoding) -> &'static str {
    match v {
        LegacyEncoding::Utf8 => "UTF-8",
        LegacyEncoding::Latin1 => "ISO-8859-1",
        LegacyEncoding::Latin2 => "ISO-8859-2",
        LegacyEncoding::Latin3 => "ISO-8859-3",
        LegacyEncoding::MacRoman => "macintosh",
    }
}
fn limit(max: usize) -> XmlOutputFailure {
    failure(
        XmlOutputFailureKind::Limit,
        "BIB_XML_LIMIT",
        &format!("XML output exceeds the {max} byte limit"),
    )
}
fn failure(kind: XmlOutputFailureKind, code: &str, message: &str) -> XmlOutputFailure {
    let diagnostic = bib_model::DiagnosticBuilder::new(
        BibDiagnosticCode::new(code).expect("static diagnostic code"),
        BibSeverity::Error,
        message,
    )
    .expect("output diagnostic message is valid")
    .freeze();
    XmlOutputFailure {
        kind,
        diagnostics: Arc::from([diagnostic]),
    }
}
