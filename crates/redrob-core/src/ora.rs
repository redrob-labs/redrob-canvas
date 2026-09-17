// SPDX-License-Identifier: GPL-3.0-or-later

use std::collections::{HashMap, HashSet};
use std::io::{Cursor, Read, Seek, SeekFrom, Write};

use quick_xml::Reader;
use quick_xml::XmlVersion;
use quick_xml::events::{BytesStart, Event};
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipArchive, ZipWriter};

use crate::{
    BlendMode, Document, DocumentImportBuilder, ExportOptions, FileFormat, FormatError,
    FormatWarning, FrameId, ImportNode, ImportOptions, LossPolicy, NodeId, NodeKind, RasterCel,
    RenderSnapshot, Result,
};

const MIMETYPE: &[u8] = b"image/openraster";
const MAX_ARCHIVE_ENTRIES: usize = 8_192;
const MAX_EXPANDED_BYTES: u64 = 512 * 1024 * 1024;
const MAX_ENTRY_BYTES: u64 = 256 * 1024 * 1024;
const MAX_XML_BYTES: usize = 4 * 1024 * 1024;

#[derive(Debug)]
struct BoundedCursor {
    inner: Cursor<Vec<u8>>,
    limit: usize,
}

impl BoundedCursor {
    fn new(limit: usize) -> Self {
        Self {
            inner: Cursor::new(Vec::new()),
            limit,
        }
    }

    fn into_inner(self) -> Vec<u8> {
        self.inner.into_inner()
    }
}

impl Write for BoundedCursor {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        let end = self
            .inner
            .position()
            .checked_add(bytes.len() as u64)
            .ok_or_else(|| std::io::Error::from(std::io::ErrorKind::WriteZero))?;
        if end > self.limit as u64 {
            return Err(std::io::ErrorKind::WriteZero.into());
        }
        self.inner.write(bytes)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.inner.flush()
    }
}

impl Seek for BoundedCursor {
    fn seek(&mut self, position: SeekFrom) -> std::io::Result<u64> {
        self.inner.seek(position)
    }
}

fn map_write_error(error: std::io::Error) -> crate::CoreError {
    if error.kind() == std::io::ErrorKind::WriteZero {
        FormatError::OutputTooLarge.into()
    } else {
        error.into()
    }
}

fn map_zip_error(error: zip::result::ZipError) -> crate::CoreError {
    match error {
        zip::result::ZipError::Io(error) if error.kind() == std::io::ErrorKind::WriteZero => {
            FormatError::OutputTooLarge.into()
        }
        _ => FormatError::Malformed("could not create ORA ZIP").into(),
    }
}

pub(crate) fn has_canonical_mimetype(bytes: &[u8]) -> bool {
    let Ok(mut archive) = ZipArchive::new(Cursor::new(bytes)) else {
        return false;
    };
    let Ok(mut first) = archive.by_index(0) else {
        return false;
    };
    if first.name() != "mimetype" || first.compression() != CompressionMethod::Stored {
        return false;
    }
    let mut value = Vec::new();
    first.read_to_end(&mut value).is_ok() && value == MIMETYPE
}

fn safe_path(path: &str) -> bool {
    !path.is_empty()
        && !path.starts_with('/')
        && !path.contains('\\')
        && path
            .split('/')
            .all(|component| !component.is_empty() && component != "." && component != "..")
}

fn little_u16(bytes: &[u8], offset: usize) -> Option<u16> {
    Some(u16::from_le_bytes(
        bytes.get(offset..offset + 2)?.try_into().ok()?,
    ))
}

fn little_u32(bytes: &[u8], offset: usize) -> Option<u32> {
    Some(u32::from_le_bytes(
        bytes.get(offset..offset + 4)?.try_into().ok()?,
    ))
}

fn has_zip64_extra(extra: &[u8]) -> Result<bool> {
    let mut position = 0_usize;
    while position < extra.len() {
        let id = little_u16(extra, position)
            .ok_or(FormatError::Malformed("invalid ORA ZIP extra field"))?;
        let size = little_u16(extra, position + 2)
            .ok_or(FormatError::Malformed("invalid ORA ZIP extra field"))?
            as usize;
        position = position
            .checked_add(4)
            .and_then(|value| value.checked_add(size))
            .ok_or(FormatError::Malformed("invalid ORA ZIP extra field"))?;
        if position > extra.len() {
            return Err(FormatError::Malformed("invalid ORA ZIP extra field").into());
        }
        if id == 0x0001 {
            return Ok(true);
        }
    }
    Ok(false)
}

fn validate_central_paths(bytes: &[u8]) -> Result<()> {
    let search_start = bytes.len().saturating_sub(65_557);
    let eocd = (search_start..bytes.len().saturating_sub(3))
        .rev()
        .find(|offset| bytes.get(*offset..*offset + 4) == Some(b"PK\x05\x06".as_slice()))
        .ok_or(FormatError::Malformed("missing ORA ZIP directory"))?;
    let disk =
        little_u16(bytes, eocd + 4).ok_or(FormatError::Malformed("invalid ORA ZIP directory"))?;
    let central_disk =
        little_u16(bytes, eocd + 6).ok_or(FormatError::Malformed("invalid ORA ZIP directory"))?;
    let disk_entries =
        little_u16(bytes, eocd + 8).ok_or(FormatError::Malformed("invalid ORA ZIP directory"))?;
    let entries =
        little_u16(bytes, eocd + 10).ok_or(FormatError::Malformed("invalid ORA ZIP directory"))?;
    if disk != 0 || central_disk != 0 || disk_entries != entries || entries == u16::MAX {
        return Err(FormatError::UnsupportedFeature("multi-disk or ZIP64 ORA archive").into());
    }
    if entries as usize > MAX_ARCHIVE_ENTRIES {
        return Err(FormatError::LimitExceeded("ORA entry count").into());
    }
    let central_size_raw =
        little_u32(bytes, eocd + 12).ok_or(FormatError::Malformed("invalid ORA ZIP directory"))?;
    let central_offset_raw =
        little_u32(bytes, eocd + 16).ok_or(FormatError::Malformed("invalid ORA ZIP directory"))?;
    if central_size_raw == u32::MAX || central_offset_raw == u32::MAX {
        return Err(FormatError::UnsupportedFeature("multi-disk or ZIP64 ORA archive").into());
    }
    let central_size = central_size_raw as usize;
    let central_offset = central_offset_raw as usize;
    if central_offset
        .checked_add(central_size)
        .is_none_or(|end| end > eocd)
    {
        return Err(FormatError::Malformed("invalid ORA ZIP directory bounds").into());
    }
    let mut names = HashSet::with_capacity(entries as usize);
    let mut expanded = 0_u64;
    let mut position = central_offset;
    for _ in 0..entries {
        if bytes.get(position..position + 4) != Some(b"PK\x01\x02".as_slice()) {
            return Err(FormatError::Malformed("invalid ORA ZIP directory entry").into());
        }
        let flags = little_u16(bytes, position + 8)
            .ok_or(FormatError::Malformed("invalid ORA ZIP directory entry"))?;
        if flags & 1 != 0 {
            return Err(FormatError::UnsupportedFeature("encrypted ORA ZIP entry").into());
        }
        let compression = little_u16(bytes, position + 10)
            .ok_or(FormatError::Malformed("invalid ORA ZIP directory entry"))?;
        let checksum = little_u32(bytes, position + 16)
            .ok_or(FormatError::Malformed("invalid ORA ZIP directory entry"))?;
        let compressed_size = little_u32(bytes, position + 20)
            .ok_or(FormatError::Malformed("invalid ORA ZIP directory entry"))?;
        let uncompressed_size = little_u32(bytes, position + 24)
            .ok_or(FormatError::Malformed("invalid ORA ZIP directory entry"))?;
        let local_offset = little_u32(bytes, position + 42)
            .ok_or(FormatError::Malformed("invalid ORA ZIP directory entry"))?;
        if compressed_size == u32::MAX || uncompressed_size == u32::MAX || local_offset == u32::MAX
        {
            return Err(FormatError::UnsupportedFeature("multi-disk or ZIP64 ORA archive").into());
        }
        if u64::from(uncompressed_size) > MAX_ENTRY_BYTES {
            return Err(FormatError::LimitExceeded("ORA entry bytes").into());
        }
        expanded = expanded
            .checked_add(u64::from(uncompressed_size))
            .ok_or(FormatError::LimitExceeded("ORA expanded bytes"))?;
        if expanded > MAX_EXPANDED_BYTES {
            return Err(FormatError::LimitExceeded("ORA expanded bytes").into());
        }
        let name_len = little_u16(bytes, position + 28)
            .ok_or(FormatError::Malformed("invalid ORA ZIP directory entry"))?
            as usize;
        let extra_len = little_u16(bytes, position + 30)
            .ok_or(FormatError::Malformed("invalid ORA ZIP directory entry"))?
            as usize;
        let comment_len = little_u16(bytes, position + 32)
            .ok_or(FormatError::Malformed("invalid ORA ZIP directory entry"))?
            as usize;
        let name_start = position
            .checked_add(46)
            .ok_or(FormatError::Malformed("invalid ORA ZIP directory entry"))?;
        let name_end = name_start
            .checked_add(name_len)
            .ok_or(FormatError::Malformed("invalid ORA ZIP directory entry"))?;
        let name = std::str::from_utf8(
            bytes
                .get(name_start..name_end)
                .ok_or(FormatError::Malformed("invalid ORA ZIP directory entry"))?,
        )
        .map_err(|_| FormatError::Malformed("non-UTF-8 ORA ZIP path"))?;
        if !safe_path(name) || !names.insert(name) {
            return Err(FormatError::Malformed("unsafe or duplicate ORA path").into());
        }
        let extra_end = name_end
            .checked_add(extra_len)
            .ok_or(FormatError::Malformed("invalid ORA ZIP directory entry"))?;
        let central_extra = bytes
            .get(name_end..extra_end)
            .ok_or(FormatError::Malformed("invalid ORA ZIP directory entry"))?;
        if has_zip64_extra(central_extra)? {
            return Err(FormatError::UnsupportedFeature("multi-disk or ZIP64 ORA archive").into());
        }

        let local = local_offset as usize;
        if bytes.get(local..local + 4) != Some(b"PK\x03\x04".as_slice()) {
            return Err(FormatError::Malformed("invalid ORA ZIP local entry").into());
        }
        let local_flags = little_u16(bytes, local + 6)
            .ok_or(FormatError::Malformed("invalid ORA ZIP local entry"))?;
        let local_compression = little_u16(bytes, local + 8)
            .ok_or(FormatError::Malformed("invalid ORA ZIP local entry"))?;
        let local_checksum = little_u32(bytes, local + 14)
            .ok_or(FormatError::Malformed("invalid ORA ZIP local entry"))?;
        let local_compressed = little_u32(bytes, local + 18)
            .ok_or(FormatError::Malformed("invalid ORA ZIP local entry"))?;
        let local_uncompressed = little_u32(bytes, local + 22)
            .ok_or(FormatError::Malformed("invalid ORA ZIP local entry"))?;
        let local_name_len = little_u16(bytes, local + 26)
            .ok_or(FormatError::Malformed("invalid ORA ZIP local entry"))?
            as usize;
        let local_extra_len = little_u16(bytes, local + 28)
            .ok_or(FormatError::Malformed("invalid ORA ZIP local entry"))?
            as usize;
        let local_name_start = local
            .checked_add(30)
            .ok_or(FormatError::Malformed("invalid ORA ZIP local entry"))?;
        let local_name_end = local_name_start
            .checked_add(local_name_len)
            .ok_or(FormatError::Malformed("invalid ORA ZIP local entry"))?;
        let local_extra_end = local_name_end
            .checked_add(local_extra_len)
            .ok_or(FormatError::Malformed("invalid ORA ZIP local entry"))?;
        let local_name = bytes
            .get(local_name_start..local_name_end)
            .ok_or(FormatError::Malformed("invalid ORA ZIP local entry"))?;
        let local_extra = bytes
            .get(local_name_end..local_extra_end)
            .ok_or(FormatError::Malformed("invalid ORA ZIP local entry"))?;
        let uses_descriptor = flags & 0x0008 != 0;
        let descriptor_values_valid = (local_checksum == 0 || local_checksum == checksum)
            && (local_compressed == 0 || local_compressed == compressed_size)
            && (local_uncompressed == 0 || local_uncompressed == uncompressed_size);
        if has_zip64_extra(local_extra)? {
            return Err(FormatError::UnsupportedFeature("multi-disk or ZIP64 ORA archive").into());
        }
        if local_flags != flags
            || local_compression != compression
            || local_name != name.as_bytes()
            || (!uses_descriptor
                && (local_checksum != checksum
                    || local_compressed != compressed_size
                    || local_uncompressed != uncompressed_size))
            || (uses_descriptor && !descriptor_values_valid)
            || local_extra_end
                .checked_add(compressed_size as usize)
                .is_none_or(|end| end > central_offset)
        {
            return Err(FormatError::Malformed("ORA ZIP local/central mismatch").into());
        }
        position = extra_end
            .checked_add(comment_len)
            .ok_or(FormatError::Malformed("invalid ORA ZIP directory entry"))?;
    }
    if position != central_offset + central_size {
        return Err(FormatError::Malformed("invalid ORA ZIP directory size").into());
    }
    Ok(())
}

fn read_archive(bytes: &[u8]) -> Result<HashMap<String, Vec<u8>>> {
    validate_central_paths(bytes)?;
    let mut archive = ZipArchive::new(Cursor::new(bytes))
        .map_err(|_| FormatError::Malformed("invalid ORA ZIP"))?;
    if archive.is_empty() || archive.len() > MAX_ARCHIVE_ENTRIES {
        return Err(FormatError::LimitExceeded("ORA entry count").into());
    }
    let mut files = HashMap::new();
    let mut expanded = 0_u64;
    for index in 0..archive.len() {
        let mut entry = archive
            .by_index(index)
            .map_err(|_| FormatError::Malformed("invalid ORA ZIP entry"))?;
        let name = entry.name().to_owned();
        if !safe_path(&name) || !files.keys().all(|existing| existing != &name) {
            return Err(FormatError::Malformed("unsafe or duplicate ORA path").into());
        }
        if entry.is_dir() || entry.size() > MAX_ENTRY_BYTES {
            return Err(FormatError::LimitExceeded("ORA entry bytes").into());
        }
        expanded = expanded
            .checked_add(entry.size())
            .ok_or(FormatError::LimitExceeded("ORA expanded bytes"))?;
        if expanded > MAX_EXPANDED_BYTES {
            return Err(FormatError::LimitExceeded("ORA expanded bytes").into());
        }
        let mut data = Vec::new();
        entry
            .by_ref()
            .take(MAX_ENTRY_BYTES + 1)
            .read_to_end(&mut data)?;
        if data.len() as u64 > MAX_ENTRY_BYTES || data.len() as u64 != entry.size() {
            return Err(FormatError::LimitExceeded("ORA entry bytes").into());
        }
        files.insert(name, data);
    }
    if files.get("mimetype").map(Vec::as_slice) != Some(MIMETYPE)
        || !files.contains_key("stack.xml")
    {
        return Err(FormatError::Malformed("missing canonical ORA files").into());
    }
    Ok(files)
}

#[derive(Debug)]
struct OraProperties {
    name: String,
    visible: bool,
    opacity: f32,
    blend: BlendMode,
}

#[derive(Debug)]
enum OraItem {
    Layer {
        properties: OraProperties,
        src: String,
        x: i32,
        y: i32,
    },
    Group {
        properties: OraProperties,
        children: Vec<OraItem>,
    },
}

#[derive(Debug)]
struct GroupDraft {
    properties: OraProperties,
    children: Vec<OraItem>,
    root: bool,
}

fn decode_attr(
    reader: &Reader<&[u8]>,
    value: quick_xml::events::attributes::Attribute<'_>,
) -> Result<String> {
    value
        // decode_and_unescape_value is deprecated as of quick-xml 0.41. The replacement also
        // applies XML attribute-value normalization, and needs the version because 1.0 and 1.1
        // normalize line endings differently. ORA's stack.xml is an XML 1.0 document.
        .decoded_and_normalized_value(XmlVersion::Implicit1_0, reader.decoder())
        .map(|value| value.into_owned())
        .map_err(|_| FormatError::Malformed("invalid ORA XML attribute").into())
}

fn attrs(reader: &Reader<&[u8]>, start: &BytesStart<'_>) -> Result<HashMap<String, String>> {
    let mut values = HashMap::new();
    for attribute in start.attributes() {
        let attribute =
            attribute.map_err(|_| FormatError::Malformed("invalid ORA XML attribute"))?;
        let name = std::str::from_utf8(attribute.key.as_ref())
            .map_err(|_| FormatError::Malformed("non-UTF-8 ORA XML name"))?
            .to_owned();
        let value = decode_attr(reader, attribute)?;
        if values.insert(name, value).is_some() {
            return Err(FormatError::Malformed("duplicate ORA XML attribute").into());
        }
    }
    Ok(values)
}

fn parse_float(value: Option<&String>, default: f32) -> Result<f32> {
    let value = value.map_or(Ok(default), |value| {
        value
            .parse::<f32>()
            .map_err(|_| FormatError::Malformed("invalid ORA numeric attribute"))
    })?;
    if !value.is_finite() || !(0.0..=1.0).contains(&value) {
        return Err(FormatError::Malformed("invalid ORA opacity").into());
    }
    Ok(value)
}

fn parse_blend(value: Option<&String>) -> Result<BlendMode> {
    match value.map(String::as_str).unwrap_or("svg:src-over") {
        "svg:src-over" => Ok(BlendMode::Normal),
        "svg:multiply" => Ok(BlendMode::Multiply),
        "svg:screen" => Ok(BlendMode::Screen),
        "svg:overlay" => Ok(BlendMode::Overlay),
        "svg:plus" => Ok(BlendMode::Add),
        _ => Err(FormatError::UnsupportedFeature("unknown ORA composite-op").into()),
    }
}

fn parse_properties(
    values: &mut HashMap<String, String>,
    default_name: &str,
) -> Result<OraProperties> {
    let name = values
        .remove("name")
        .unwrap_or_else(|| default_name.to_owned());
    let visible = match values.remove("visibility").as_deref().unwrap_or("visible") {
        "visible" => true,
        "hidden" => false,
        _ => return Err(FormatError::Malformed("invalid ORA visibility").into()),
    };
    let opacity = parse_float(values.get("opacity"), 1.0)?;
    values.remove("opacity");
    let blend = parse_blend(values.get("composite-op"))?;
    values.remove("composite-op");
    Ok(OraProperties {
        name,
        visible,
        opacity,
        blend,
    })
}

fn root_is_synthetic(properties: &OraProperties) -> bool {
    properties.name == "root"
        && properties.visible
        && properties.opacity == 1.0
        && properties.blend == BlendMode::Normal
}

fn document_parent_depth(groups: &[GroupDraft]) -> usize {
    groups.len().saturating_sub(1)
        + usize::from(
            groups
                .first()
                .is_some_and(|root| !root_is_synthetic(&root.properties)),
        )
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ImageState {
    NotSeen,
    Open,
    Closed,
}

fn parse_stack_xml(xml: &[u8]) -> Result<(u32, u32, OraProperties, Vec<OraItem>)> {
    if xml.len() > MAX_XML_BYTES {
        return Err(FormatError::LimitExceeded("ORA XML bytes").into());
    }
    if xml
        .windows(9)
        .any(|window| window.eq_ignore_ascii_case(b"<!doctype"))
        || xml
            .windows(8)
            .any(|window| window.eq_ignore_ascii_case(b"<!entity"))
    {
        return Err(FormatError::UnsupportedFeature("ORA DTD/entity content").into());
    }
    let mut reader = Reader::from_reader(xml);
    reader.config_mut().trim_text(true);
    let mut buffer = Vec::new();
    let mut dimensions = None;
    let mut groups = Vec::<GroupDraft>::new();
    let mut roots = None;
    let mut image = ImageState::NotSeen;
    loop {
        match reader
            .read_event_into(&mut buffer)
            .map_err(|_| FormatError::Malformed("invalid ORA XML"))?
        {
            Event::Decl(_) if image == ImageState::NotSeen => {}
            Event::Start(start) if start.name().as_ref() == b"image" => {
                if image != ImageState::NotSeen || !groups.is_empty() || roots.is_some() {
                    return Err(FormatError::Malformed("invalid ORA image root").into());
                }
                image = ImageState::Open;
                let mut values = attrs(&reader, &start)?;
                let width = values
                    .remove("w")
                    .ok_or(FormatError::Malformed("missing ORA width"))?
                    .parse::<u32>()
                    .map_err(|_| FormatError::Malformed("invalid ORA width"))?;
                let height = values
                    .remove("h")
                    .ok_or(FormatError::Malformed("missing ORA height"))?
                    .parse::<u32>()
                    .map_err(|_| FormatError::Malformed("invalid ORA height"))?;
                values.remove("version");
                values.remove("name");
                if !values.is_empty() {
                    return Err(
                        FormatError::UnsupportedFeature("unknown ORA image attribute").into(),
                    );
                }
                crate::document::pixel_count(width, height)?;
                dimensions = Some((width, height));
            }
            Event::Start(start) if start.name().as_ref() == b"stack" => {
                if image != ImageState::Open {
                    return Err(FormatError::Malformed("ORA stack outside image").into());
                }
                let root = groups.is_empty();
                if root && roots.is_some() {
                    return Err(FormatError::Malformed("multiple ORA root stacks").into());
                }
                if !root && document_parent_depth(&groups) > crate::MAX_HIERARCHY_DEPTH {
                    return Err(FormatError::LimitExceeded("ORA hierarchy depth").into());
                }
                let mut values = attrs(&reader, &start)?;
                let properties =
                    parse_properties(&mut values, if root { "root" } else { "Group" })?;
                if !values.is_empty() {
                    return Err(
                        FormatError::UnsupportedFeature("unknown ORA stack attribute").into(),
                    );
                }
                groups.push(GroupDraft {
                    properties,
                    children: Vec::new(),
                    root,
                });
            }
            Event::Empty(start) if start.name().as_ref() == b"layer" => {
                if image != ImageState::Open {
                    return Err(FormatError::Malformed("ORA layer outside image").into());
                }
                if groups.is_empty() {
                    return Err(FormatError::Malformed("ORA layer outside stack").into());
                }
                if document_parent_depth(&groups) > crate::MAX_HIERARCHY_DEPTH {
                    return Err(FormatError::LimitExceeded("ORA hierarchy depth").into());
                }
                let group = groups.last_mut().expect("ORA group was checked above");
                let mut values = attrs(&reader, &start)?;
                let properties = parse_properties(&mut values, "Layer")?;
                let src = values
                    .remove("src")
                    .ok_or(FormatError::Malformed("missing ORA layer source"))?;
                let x = values
                    .remove("x")
                    .map_or(Ok(0), |value| value.parse::<i32>())
                    .map_err(|_| FormatError::Malformed("invalid ORA layer x"))?;
                let y = values
                    .remove("y")
                    .map_or(Ok(0), |value| value.parse::<i32>())
                    .map_err(|_| FormatError::Malformed("invalid ORA layer y"))?;
                if !safe_path(&src) || !values.is_empty() {
                    return Err(FormatError::UnsupportedFeature(
                        "unknown or unsafe ORA layer attribute",
                    )
                    .into());
                }
                group.children.push(OraItem::Layer {
                    properties,
                    src,
                    x,
                    y,
                });
            }
            Event::Empty(start) if start.name().as_ref() == b"mask" => {
                return Err(FormatError::UnsupportedFeature("ORA masks on import").into());
            }
            Event::End(end) if end.name().as_ref() == b"stack" => {
                if image != ImageState::Open {
                    return Err(FormatError::Malformed("ORA stack close outside image").into());
                }
                let group = groups
                    .pop()
                    .ok_or(FormatError::Malformed("unbalanced ORA stack"))?;
                if group.root {
                    if !groups.is_empty()
                        || roots.replace((group.properties, group.children)).is_some()
                    {
                        return Err(FormatError::Malformed("multiple ORA root stacks").into());
                    }
                } else {
                    groups
                        .last_mut()
                        .ok_or(FormatError::Malformed("nested ORA stack without parent"))?
                        .children
                        .push(OraItem::Group {
                            properties: group.properties,
                            children: group.children,
                        });
                }
            }
            Event::End(end) if end.name().as_ref() == b"image" => {
                if image != ImageState::Open || !groups.is_empty() || roots.is_none() {
                    return Err(FormatError::Malformed("unbalanced ORA image root").into());
                }
                image = ImageState::Closed;
            }
            Event::Text(text) if text.as_ref().iter().all(u8::is_ascii_whitespace) => {}
            Event::Eof => break,
            Event::DocType(_) => {
                return Err(FormatError::UnsupportedFeature("ORA DTD/entity content").into());
            }
            _ if image == ImageState::Closed => {
                return Err(FormatError::Malformed("content after ORA image root").into());
            }
            _ => return Err(FormatError::UnsupportedFeature("unknown ORA XML element").into()),
        }
        buffer.clear();
    }
    if image != ImageState::Closed || !groups.is_empty() {
        return Err(FormatError::Malformed("unclosed ORA image or stack").into());
    }
    let (width, height) = dimensions.ok_or(FormatError::Malformed("missing ORA image"))?;
    let (root, items) = roots.ok_or(FormatError::Malformed("missing ORA root stack"))?;
    Ok((width, height, root, items))
}

fn place_layer(
    source: &[u8],
    source_width: u32,
    source_height: u32,
    width: u32,
    height: u32,
    x: i32,
    y: i32,
) -> Vec<u8> {
    let mut output = vec![0; width as usize * height as usize * 4];
    for source_y in 0..source_height {
        let destination_y = i64::from(y) + i64::from(source_y);
        if !(0..i64::from(height)).contains(&destination_y) {
            continue;
        }
        for source_x in 0..source_width {
            let destination_x = i64::from(x) + i64::from(source_x);
            if !(0..i64::from(width)).contains(&destination_x) {
                continue;
            }
            let source_offset = (source_y as usize * source_width as usize + source_x as usize) * 4;
            let destination_offset =
                (destination_y as usize * width as usize + destination_x as usize) * 4;
            output[destination_offset..destination_offset + 4]
                .copy_from_slice(&source[source_offset..source_offset + 4]);
        }
    }
    output
}

fn add_items(
    items: Vec<OraItem>,
    parent: Option<NodeId>,
    width: u32,
    height: u32,
    files: &HashMap<String, Vec<u8>>,
    referenced: &mut HashSet<String>,
    builder: &mut DocumentImportBuilder,
) -> Result<()> {
    // ORA children are top-first; core siblings are bottom-first.
    for item in items.into_iter().rev() {
        match item {
            OraItem::Layer {
                properties,
                src,
                x,
                y,
            } => {
                if !referenced.insert(src.clone()) {
                    return Err(FormatError::Malformed("duplicate ORA layer source").into());
                }
                let data = files
                    .get(&src)
                    .ok_or(FormatError::Malformed("missing ORA layer source"))?;
                let (source_width, source_height, source) =
                    crate::formats::decode_rgba(data, FileFormat::Png)?;
                let pixels = place_layer(&source, source_width, source_height, width, height, x, y);
                builder.push_node(
                    ImportNode::raster(
                        properties.name,
                        vec![RasterCel::new(FrameId::DEFAULT, pixels)],
                    )
                    .with_parent(parent)
                    .with_visibility(properties.visible)
                    .with_opacity(properties.opacity)
                    .with_blend_mode(properties.blend),
                )?;
            }
            OraItem::Group {
                properties,
                children,
            } => {
                let group = ImportNode::group(properties.name)
                    .with_parent(parent)
                    .with_visibility(properties.visible)
                    .with_opacity(properties.opacity)
                    .with_blend_mode(properties.blend);
                let id = group.id();
                builder.push_node(group)?;
                add_items(
                    children,
                    Some(id),
                    width,
                    height,
                    files,
                    referenced,
                    builder,
                )?;
            }
        }
    }
    Ok(())
}

pub(crate) fn import_ora(
    bytes: &[u8],
    _options: &ImportOptions,
) -> Result<(Document, Vec<FormatWarning>)> {
    if !has_canonical_mimetype(bytes) {
        return Err(FormatError::Malformed("non-canonical ORA mimetype").into());
    }
    let files = read_archive(bytes)?;
    let xml = files
        .get("stack.xml")
        .ok_or(FormatError::Malformed("missing stack.xml"))?;
    let (width, height, root, items) = parse_stack_xml(xml)?;
    let mut builder = DocumentImportBuilder::new(width, height)?;
    let mut referenced = HashSet::new();
    let root_is_synthetic = root_is_synthetic(&root);
    let parent = if root_is_synthetic {
        None
    } else {
        let node = ImportNode::group(root.name)
            .with_visibility(root.visible)
            .with_opacity(root.opacity)
            .with_blend_mode(root.blend);
        let id = node.id();
        builder.push_node(node)?;
        Some(id)
    };
    add_items(
        items,
        parent,
        width,
        height,
        &files,
        &mut referenced,
        &mut builder,
    )?;
    Ok((builder.build()?, Vec::new()))
}

fn xml_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

fn blend_name(blend: BlendMode) -> &'static str {
    match blend {
        BlendMode::Normal => "svg:src-over",
        BlendMode::Multiply => "svg:multiply",
        BlendMode::Screen => "svg:screen",
        BlendMode::Overlay => "svg:overlay",
        BlendMode::Add => "svg:plus",
    }
}

fn properties(node: &crate::Layer) -> String {
    format!(
        "name=\"{}\" visibility=\"{}\" opacity=\"{}\" composite-op=\"{}\"",
        xml_escape(node.name()),
        if node.is_visible() {
            "visible"
        } else {
            "hidden"
        },
        node.opacity(),
        blend_name(node.blend_mode())
    )
}

fn preflight_nodes(
    document: &Document,
    policy: LossPolicy,
    warnings: &mut Vec<FormatWarning>,
) -> Result<()> {
    for node in document.nodes() {
        if node.kind() == NodeKind::Group {
            if let Some(mask) = node.mask() {
                if mask.is_enabled() {
                    return Err(FormatError::UnsupportedFeature(
                        "masked ORA groups cannot be represented safely",
                    )
                    .into());
                }
                if policy == LossPolicy::RejectLoss {
                    return Err(FormatError::LossRequired(
                        "ORA cannot preserve disabled group mask source",
                    )
                    .into());
                }
                warnings.push(FormatWarning::OmittedDisabledMask { node: node.id() });
            }
            continue;
        }
        if matches!(node.kind(), NodeKind::Text | NodeKind::Vector) {
            if policy == LossPolicy::RejectLoss {
                return Err(
                    FormatError::LossRequired("ORA cannot preserve semantic source nodes").into(),
                );
            }
            warnings.push(FormatWarning::RasterizedSemanticNode { node: node.id() });
        }
        if let Some(mask) = node.mask() {
            if policy == LossPolicy::RejectLoss {
                return Err(
                    FormatError::LossRequired("ORA cannot preserve raster mask source").into(),
                );
            }
            warnings.push(if mask.is_enabled() {
                FormatWarning::BakedRasterMask { node: node.id() }
            } else {
                FormatWarning::OmittedDisabledMask { node: node.id() }
            });
        }
    }
    Ok(())
}

fn source_pixels(document: &Document, node: &crate::Layer, frame: FrameId) -> Result<Vec<u8>> {
    let mut pixels = match node.kind() {
        NodeKind::Raster => node.raster_pixels(frame).map_or_else(
            |_| vec![0; document.width() as usize * document.height() as usize * 4],
            <[u8]>::to_vec,
        ),
        NodeKind::Text | NodeKind::Vector => {
            crate::semantic::rasterize(node.content(), document.width(), document.height())?
        }
        NodeKind::Group => unreachable!(),
    };
    if let Some(mask) = node.mask().filter(|mask| mask.is_enabled()) {
        for (pixel, coverage) in pixels.chunks_exact_mut(4).zip(mask.pixels()) {
            pixel[3] = ((u16::from(pixel[3]) * u16::from(*coverage) + 127) / 255) as u8;
        }
    }
    Ok(pixels)
}

struct OraExportWriter<'a> {
    xml: &'a mut String,
    document: &'a Document,
    paths: &'a HashMap<NodeId, String>,
}

impl OraExportWriter<'_> {
    fn write_nodes(&mut self, parent: Option<NodeId>, indent: usize) -> Result<()> {
        let children = self
            .document
            .nodes()
            .iter()
            .filter(|node| node.parent_id() == parent)
            .collect::<Vec<_>>();
        for node in children.into_iter().rev() {
            let padding = "  ".repeat(indent);
            if node.kind() == NodeKind::Group {
                self.xml
                    .push_str(&format!("{padding}<stack {}>\n", properties(node)));
                self.write_nodes(Some(node.id()), indent + 1)?;
                self.xml.push_str(&format!("{padding}</stack>\n"));
            } else {
                let path = &self.paths[&node.id()];
                self.xml.push_str(&format!(
                    "{padding}<layer {} src=\"{}\" x=\"0\" y=\"0\"/>\n",
                    properties(node),
                    path
                ));
            }
            if self.xml.len() > MAX_XML_BYTES {
                return Err(FormatError::OutputTooLarge.into());
            }
        }
        Ok(())
    }
}

pub(crate) fn export_ora(
    document: &Document,
    frame: FrameId,
    options: &ExportOptions,
) -> Result<(Vec<u8>, Vec<FormatWarning>)> {
    let mut warnings = Vec::new();
    crate::formats::check_metadata_loss(document, options.loss_policy(), &mut warnings)?;
    if document.timeline().frames().len() > 1 {
        if options.loss_policy() == LossPolicy::RejectLoss {
            return Err(FormatError::LossRequired("ORA export omits other frames").into());
        }
        warnings.push(FormatWarning::OmittedFrames { exported: frame });
    }
    if document.is_selection_active() {
        if options.loss_policy() == LossPolicy::RejectLoss {
            return Err(FormatError::LossRequired("ORA export omits selection").into());
        }
        warnings.push(FormatWarning::OmittedSelection);
    }
    preflight_nodes(document, options.loss_policy(), &mut warnings)?;

    let mut paths = HashMap::new();
    let mut index = 0_usize;
    for node in document.nodes() {
        if node.kind() != NodeKind::Group {
            paths.insert(node.id(), format!("data/layer{index:04}.png"));
            index += 1;
        }
    }
    let mut xml = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<image version=\"0.0.1\" w=\"{}\" h=\"{}\">\n  <stack name=\"root\">\n",
        document.width(),
        document.height()
    );
    OraExportWriter {
        xml: &mut xml,
        document,
        paths: &paths,
    }
    .write_nodes(None, 2)?;
    xml.push_str("  </stack>\n</image>\n");
    if xml.len() > MAX_XML_BYTES {
        return Err(FormatError::OutputTooLarge.into());
    }

    let cursor = BoundedCursor::new(crate::MAX_FORMAT_OUTPUT_BYTES);
    let mut writer = ZipWriter::new(cursor);
    writer
        .start_file(
            "mimetype",
            SimpleFileOptions::default().compression_method(CompressionMethod::Stored),
        )
        .map_err(map_zip_error)?;
    writer.write_all(MIMETYPE).map_err(map_write_error)?;
    writer
        .start_file(
            "stack.xml",
            SimpleFileOptions::default().compression_method(CompressionMethod::Deflated),
        )
        .map_err(map_zip_error)?;
    writer.write_all(xml.as_bytes()).map_err(map_write_error)?;
    for node in document
        .nodes()
        .iter()
        .filter(|node| node.kind() != NodeKind::Group)
    {
        let path = &paths[&node.id()];
        let pixels = source_pixels(document, node, frame)?;
        let png = crate::formats::encode_png(document.width(), document.height(), &pixels)?;
        writer
            .start_file(
                path,
                SimpleFileOptions::default().compression_method(CompressionMethod::Deflated),
            )
            .map_err(map_zip_error)?;
        writer.write_all(&png).map_err(map_write_error)?;
    }
    let merged = RenderSnapshot::try_render_frame(document, 0, frame)?;
    let merged_png = crate::formats::encode_png(merged.width(), merged.height(), merged.pixels())?;
    writer
        .start_file(
            "mergedimage.png",
            SimpleFileOptions::default().compression_method(CompressionMethod::Deflated),
        )
        .map_err(map_zip_error)?;
    writer.write_all(&merged_png).map_err(map_write_error)?;
    let bytes = writer.finish().map_err(map_zip_error)?.into_inner();
    Ok((bytes, warnings))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Clone)]
    struct MetadataEntry {
        name: String,
        flags: u16,
        checksum: u32,
        compressed: u32,
        uncompressed: u32,
        local_extra: Vec<u8>,
        central_extra: Vec<u8>,
    }

    impl MetadataEntry {
        fn empty(name: impl Into<String>) -> Self {
            Self {
                name: name.into(),
                flags: 0,
                checksum: 0,
                compressed: 0,
                uncompressed: 0,
                local_extra: Vec::new(),
                central_extra: Vec::new(),
            }
        }

        fn declared(name: impl Into<String>, uncompressed: u32) -> Self {
            Self {
                uncompressed,
                ..Self::empty(name)
            }
        }
    }

    fn metadata_zip(entries: &[MetadataEntry]) -> Vec<u8> {
        assert!(entries.len() <= u16::MAX as usize);
        let mut bytes = Vec::new();
        let mut central = Vec::new();
        for entry in entries {
            let offset = bytes.len() as u32;
            let name = entry.name.as_bytes();
            bytes.extend_from_slice(&0x0403_4b50_u32.to_le_bytes());
            bytes.extend_from_slice(&20_u16.to_le_bytes());
            bytes.extend_from_slice(&entry.flags.to_le_bytes());
            bytes.extend_from_slice(&0_u16.to_le_bytes());
            bytes.extend_from_slice(&0_u16.to_le_bytes());
            bytes.extend_from_slice(&0_u16.to_le_bytes());
            bytes.extend_from_slice(&entry.checksum.to_le_bytes());
            bytes.extend_from_slice(&entry.compressed.to_le_bytes());
            bytes.extend_from_slice(&entry.uncompressed.to_le_bytes());
            bytes.extend_from_slice(&(name.len() as u16).to_le_bytes());
            bytes.extend_from_slice(&(entry.local_extra.len() as u16).to_le_bytes());
            bytes.extend_from_slice(name);
            bytes.extend_from_slice(&entry.local_extra);

            central.extend_from_slice(&0x0201_4b50_u32.to_le_bytes());
            central.extend_from_slice(&20_u16.to_le_bytes());
            central.extend_from_slice(&20_u16.to_le_bytes());
            central.extend_from_slice(&entry.flags.to_le_bytes());
            central.extend_from_slice(&0_u16.to_le_bytes());
            central.extend_from_slice(&0_u16.to_le_bytes());
            central.extend_from_slice(&0_u16.to_le_bytes());
            central.extend_from_slice(&entry.checksum.to_le_bytes());
            central.extend_from_slice(&entry.compressed.to_le_bytes());
            central.extend_from_slice(&entry.uncompressed.to_le_bytes());
            central.extend_from_slice(&(name.len() as u16).to_le_bytes());
            central.extend_from_slice(&(entry.central_extra.len() as u16).to_le_bytes());
            central.extend_from_slice(&0_u16.to_le_bytes());
            central.extend_from_slice(&0_u16.to_le_bytes());
            central.extend_from_slice(&0_u16.to_le_bytes());
            central.extend_from_slice(&0_u32.to_le_bytes());
            central.extend_from_slice(&offset.to_le_bytes());
            central.extend_from_slice(name);
            central.extend_from_slice(&entry.central_extra);
        }
        let central_offset = bytes.len() as u32;
        bytes.extend_from_slice(&central);
        bytes.extend_from_slice(&0x0605_4b50_u32.to_le_bytes());
        bytes.extend_from_slice(&0_u16.to_le_bytes());
        bytes.extend_from_slice(&0_u16.to_le_bytes());
        bytes.extend_from_slice(&(entries.len() as u16).to_le_bytes());
        bytes.extend_from_slice(&(entries.len() as u16).to_le_bytes());
        bytes.extend_from_slice(&(central.len() as u32).to_le_bytes());
        bytes.extend_from_slice(&central_offset.to_le_bytes());
        bytes.extend_from_slice(&0_u16.to_le_bytes());
        bytes
    }

    fn assert_format_error(result: Result<()>, expected: FormatError) {
        let error = result.unwrap_err();
        let matches = match (&error, expected) {
            (
                crate::CoreError::Format(FormatError::LimitExceeded(actual)),
                FormatError::LimitExceeded(expected),
            )
            | (
                crate::CoreError::Format(FormatError::UnsupportedFeature(actual)),
                FormatError::UnsupportedFeature(expected),
            )
            | (
                crate::CoreError::Format(FormatError::Malformed(actual)),
                FormatError::Malformed(expected),
            ) => actual == &expected,
            _ => false,
        };
        assert!(matches, "unexpected error: {error:?}");
    }

    #[test]
    fn stack_parser_enforces_exact_hierarchy_and_xml_byte_limits() {
        let mut exact_depth = String::from("<image w=\"1\" h=\"1\"><stack>");
        for _ in 0..crate::MAX_HIERARCHY_DEPTH {
            exact_depth.push_str("<stack>");
        }
        exact_depth.push_str("<layer src=\"data/a.png\"/>");
        for _ in 0..crate::MAX_HIERARCHY_DEPTH {
            exact_depth.push_str("</stack>");
        }
        exact_depth.push_str("</stack></image>");
        assert!(parse_stack_xml(exact_depth.as_bytes()).is_ok());

        let one_over = exact_depth.replacen(
            "<layer src=\"data/a.png\"/>",
            "<stack><layer src=\"data/a.png\"/></stack>",
            1,
        );
        assert_format_error(
            parse_stack_xml(one_over.as_bytes()).map(|_| ()),
            FormatError::LimitExceeded("ORA hierarchy depth"),
        );

        let mut materialized_exact =
            String::from("<image w=\"1\" h=\"1\"><stack name=\"materialized\">");
        for _ in 1..crate::MAX_HIERARCHY_DEPTH {
            materialized_exact.push_str("<stack>");
        }
        materialized_exact.push_str("<layer src=\"data/a.png\"/>");
        for _ in 1..crate::MAX_HIERARCHY_DEPTH {
            materialized_exact.push_str("</stack>");
        }
        materialized_exact.push_str("</stack></image>");
        assert!(parse_stack_xml(materialized_exact.as_bytes()).is_ok());
        let materialized_over = materialized_exact.replacen(
            "<layer src=\"data/a.png\"/>",
            "<stack><layer src=\"data/a.png\"/></stack>",
            1,
        );
        assert_format_error(
            parse_stack_xml(materialized_over.as_bytes()).map(|_| ()),
            FormatError::LimitExceeded("ORA hierarchy depth"),
        );

        let prefix = "<image w=\"1\" h=\"1\"><stack></stack>";
        let suffix = "</image>";
        let mut exact_xml = String::with_capacity(MAX_XML_BYTES);
        exact_xml.push_str(prefix);
        exact_xml.push_str(&" ".repeat(MAX_XML_BYTES - prefix.len() - suffix.len()));
        exact_xml.push_str(suffix);
        assert_eq!(exact_xml.len(), MAX_XML_BYTES);
        assert!(parse_stack_xml(exact_xml.as_bytes()).is_ok());
        exact_xml.push(' ');
        assert_format_error(
            parse_stack_xml(exact_xml.as_bytes()).map(|_| ()),
            FormatError::LimitExceeded("ORA XML bytes"),
        );
    }

    #[test]
    fn central_directory_entry_count_limit_is_exact() {
        let entries = (0..MAX_ARCHIVE_ENTRIES)
            .map(|index| MetadataEntry::empty(format!("e{index}")))
            .collect::<Vec<_>>();
        let mut archive = metadata_zip(&entries);
        assert!(validate_central_paths(&archive).is_ok());
        let eocd = archive.len() - 22;
        let one_over = (MAX_ARCHIVE_ENTRIES as u16 + 1).to_le_bytes();
        archive[eocd + 8..eocd + 10].copy_from_slice(&one_over);
        archive[eocd + 10..eocd + 12].copy_from_slice(&one_over);
        assert_format_error(
            validate_central_paths(&archive),
            FormatError::LimitExceeded("ORA entry count"),
        );
    }

    #[test]
    fn central_directory_declared_size_limits_are_checked_without_allocation() {
        let exact_entry = metadata_zip(&[MetadataEntry::declared("exact", MAX_ENTRY_BYTES as u32)]);
        assert!(validate_central_paths(&exact_entry).is_ok());
        let over_entry =
            metadata_zip(&[MetadataEntry::declared("over", MAX_ENTRY_BYTES as u32 + 1)]);
        assert_format_error(
            validate_central_paths(&over_entry),
            FormatError::LimitExceeded("ORA entry bytes"),
        );

        let exact_expanded = metadata_zip(&[
            MetadataEntry::declared("a", MAX_ENTRY_BYTES as u32),
            MetadataEntry::declared("b", MAX_ENTRY_BYTES as u32),
        ]);
        assert!(validate_central_paths(&exact_expanded).is_ok());
        let over_expanded = metadata_zip(&[
            MetadataEntry::declared("a", MAX_ENTRY_BYTES as u32),
            MetadataEntry::declared("b", MAX_ENTRY_BYTES as u32),
            MetadataEntry::declared("c", 1),
        ]);
        assert_format_error(
            validate_central_paths(&over_expanded),
            FormatError::LimitExceeded("ORA expanded bytes"),
        );
    }

    #[test]
    fn central_directory_rejects_encryption_zip64_and_multidisk_markers() {
        let encrypted = metadata_zip(&[MetadataEntry {
            flags: 1,
            ..MetadataEntry::empty("encrypted")
        }]);
        assert_format_error(
            validate_central_paths(&encrypted),
            FormatError::UnsupportedFeature("encrypted ORA ZIP entry"),
        );

        let mut zip64_eocd = metadata_zip(&[MetadataEntry::empty("zip64-eocd")]);
        let eocd = zip64_eocd.len() - 22;
        zip64_eocd[eocd + 8..eocd + 12].fill(0xff);
        assert_format_error(
            validate_central_paths(&zip64_eocd),
            FormatError::UnsupportedFeature("multi-disk or ZIP64 ORA archive"),
        );

        let mut zip64_offset = metadata_zip(&[MetadataEntry::empty("zip64-offset")]);
        let eocd = zip64_offset.len() - 22;
        zip64_offset[eocd + 16..eocd + 20].fill(0xff);
        assert_format_error(
            validate_central_paths(&zip64_offset),
            FormatError::UnsupportedFeature("multi-disk or ZIP64 ORA archive"),
        );

        let mut zip64_entry = metadata_zip(&[MetadataEntry::empty("zip64-entry")]);
        let eocd = zip64_entry.len() - 22;
        let central = little_u32(&zip64_entry, eocd + 16).unwrap() as usize;
        zip64_entry[central + 24..central + 28].fill(0xff);
        assert_format_error(
            validate_central_paths(&zip64_entry),
            FormatError::UnsupportedFeature("multi-disk or ZIP64 ORA archive"),
        );

        let zip64_local_extra = metadata_zip(&[MetadataEntry {
            local_extra: vec![1, 0, 0, 0],
            ..MetadataEntry::empty("zip64-local-extra")
        }]);
        assert_format_error(
            validate_central_paths(&zip64_local_extra),
            FormatError::UnsupportedFeature("multi-disk or ZIP64 ORA archive"),
        );
        let zip64_central_extra = metadata_zip(&[MetadataEntry {
            central_extra: vec![1, 0, 0, 0],
            ..MetadataEntry::empty("zip64-central-extra")
        }]);
        assert_format_error(
            validate_central_paths(&zip64_central_extra),
            FormatError::UnsupportedFeature("multi-disk or ZIP64 ORA archive"),
        );

        for field in [4_usize, 6] {
            let mut multidisk = metadata_zip(&[MetadataEntry::empty("multidisk")]);
            let eocd = multidisk.len() - 22;
            multidisk[eocd + field..eocd + field + 2].copy_from_slice(&1_u16.to_le_bytes());
            assert_format_error(
                validate_central_paths(&multidisk),
                FormatError::UnsupportedFeature("multi-disk or ZIP64 ORA archive"),
            );
        }
        let mut split_counts = metadata_zip(&[MetadataEntry::empty("split-counts")]);
        let eocd = split_counts.len() - 22;
        split_counts[eocd + 8..eocd + 10].copy_from_slice(&0_u16.to_le_bytes());
        assert_format_error(
            validate_central_paths(&split_counts),
            FormatError::UnsupportedFeature("multi-disk or ZIP64 ORA archive"),
        );
    }

    #[test]
    fn central_directory_rejects_local_central_inconsistency() {
        for (offset, replacement) in [(6_usize, 2_u16.to_le_bytes()), (8, 8_u16.to_le_bytes())] {
            let mut mismatch = metadata_zip(&[MetadataEntry::empty("mismatch")]);
            mismatch[offset..offset + 2].copy_from_slice(&replacement);
            assert_format_error(
                validate_central_paths(&mismatch),
                FormatError::Malformed("ORA ZIP local/central mismatch"),
            );
        }
        for offset in [14_usize, 18, 22] {
            let mut mismatch = metadata_zip(&[MetadataEntry::empty("mismatch")]);
            mismatch[offset..offset + 4].copy_from_slice(&1_u32.to_le_bytes());
            assert_format_error(
                validate_central_paths(&mismatch),
                FormatError::Malformed("ORA ZIP local/central mismatch"),
            );
        }
        let mut name_mismatch = metadata_zip(&[MetadataEntry::empty("mismatch")]);
        name_mismatch[30] = b'M';
        assert_format_error(
            validate_central_paths(&name_mismatch),
            FormatError::Malformed("ORA ZIP local/central mismatch"),
        );

        let descriptor_mismatch = metadata_zip(&[MetadataEntry {
            flags: 0x0008,
            checksum: 1,
            ..MetadataEntry::empty("descriptor")
        }]);
        let mut descriptor_mismatch = descriptor_mismatch;
        descriptor_mismatch[14..18].copy_from_slice(&2_u32.to_le_bytes());
        assert_format_error(
            validate_central_paths(&descriptor_mismatch),
            FormatError::Malformed("ORA ZIP local/central mismatch"),
        );

        let payload_overrun = metadata_zip(&[MetadataEntry {
            compressed: 1,
            ..MetadataEntry::empty("overrun")
        }]);
        assert_format_error(
            validate_central_paths(&payload_overrun),
            FormatError::Malformed("ORA ZIP local/central mismatch"),
        );
    }
}
