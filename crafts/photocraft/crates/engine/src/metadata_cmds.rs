//! Image › Metadata: the viewer (every EXIF, XMP and IPTC tag the document carries) and the
//! editor (IPTC/XMP descriptive, rights, GPS and DICOM fields written into the XMP packet).
//!
//! * `metadata.view {}` → `{exif: [{tag, name, value}], xmp: [{property, value}], iptc: [{dataset,
//!   name, value}], text: [{key, value}]}`. EXIF is walked from the TIFF structure (IFD0, the Exif
//!   and GPS sub-IFDs); XMP is every simple property or container in the packet; IPTC-IIM comes
//!   from the PSD `1028` resource (datasets of record 2).
//! * `metadata.fields {}` → the editor's fields: `[{key, label, tab, property, kind}]`.
//! * `metadata.edit {<key>: value, …}` writes the given fields into the XMP packet (one history
//!   step; a null or empty value removes the property) and returns the fields as stored.

use serde_json::{Map, Value, json};

use crate::commands::CommandSpec;
use crate::{EngineError, Result, Session};

/// One editable field: key, English label, tab, XMP property, container kind ("" simple, "Alt",
/// "Bag", "Seq").
pub const FIELDS: &[(&str, &str, &str, &str, &str)] = &[
    // Description
    ("title", "Document Title", "Description", "dc:title", "Alt"),
    ("author", "Author", "Description", "dc:creator", "Seq"),
    ("authorTitle", "Author Title", "Description", "photoshop:AuthorsPosition", ""),
    ("description", "Description", "Description", "dc:description", "Alt"),
    ("descriptionWriter", "Description Writer", "Description", "photoshop:CaptionWriter", ""),
    ("keywords", "Keywords", "Description", "dc:subject", "Bag"),
    ("rating", "Rating", "Description", "xmp:Rating", ""),
    // Rights
    ("copyright", "Copyright Notice", "Rights", "dc:rights", "Alt"),
    ("copyrightStatus", "Copyright Status", "Rights", "xmpRights:Marked", ""),
    ("copyrightUrl", "Copyright Info URL", "Rights", "xmpRights:WebStatement", ""),
    ("usageTerms", "Usage Terms", "Rights", "xmpRights:UsageTerms", "Alt"),
    ("creditLine", "Credit Line", "Rights", "photoshop:Credit", ""),
    ("source", "Source", "Rights", "photoshop:Source", ""),
    // IPTC
    ("headline", "Headline", "IPTC", "photoshop:Headline", ""),
    ("instructions", "Instructions", "IPTC", "photoshop:Instructions", ""),
    ("jobIdentifier", "Job Identifier", "IPTC", "photoshop:TransmissionReference", ""),
    ("dateCreated", "Date Created", "IPTC", "photoshop:DateCreated", ""),
    ("intellectualGenre", "Intellectual Genre", "IPTC", "Iptc4xmpCore:IntellectualGenre", ""),
    ("sceneCode", "IPTC Scene Code", "IPTC", "Iptc4xmpCore:Scene", "Bag"),
    ("subjectCode", "IPTC Subject Code", "IPTC", "Iptc4xmpCore:SubjectCode", "Bag"),
    ("city", "City", "IPTC", "photoshop:City", ""),
    ("state", "State / Province", "IPTC", "photoshop:State", ""),
    ("country", "Country", "IPTC", "photoshop:Country", ""),
    ("countryCode", "Country Code", "IPTC", "Iptc4xmpCore:CountryCode", ""),
    ("sublocation", "Sublocation", "IPTC", "Iptc4xmpCore:Location", ""),
    ("creatorAddress", "Creator Address", "IPTC", "Iptc4xmpCore:CiAdrExtadr", ""),
    ("creatorCity", "Creator City", "IPTC", "Iptc4xmpCore:CiAdrCity", ""),
    ("creatorCountry", "Creator Country", "IPTC", "Iptc4xmpCore:CiAdrCtry", ""),
    ("creatorPostalCode", "Creator Postal Code", "IPTC", "Iptc4xmpCore:CiAdrPcode", ""),
    ("creatorPhone", "Creator Phone", "IPTC", "Iptc4xmpCore:CiTelWork", ""),
    ("creatorEmail", "Creator Email", "IPTC", "Iptc4xmpCore:CiEmailWork", ""),
    ("creatorUrl", "Creator Website", "IPTC", "Iptc4xmpCore:CiUrlWork", ""),
    // IPTC Extension
    ("personShown", "Person Shown", "IPTC Extension", "Iptc4xmpExt:PersonInImage", "Bag"),
    ("event", "Event", "IPTC Extension", "Iptc4xmpExt:Event", "Alt"),
    ("digitalSourceType", "Digital Source Type", "IPTC Extension", "Iptc4xmpExt:DigitalSourceType", ""),
    ("organisationName", "Organisation Name", "IPTC Extension", "Iptc4xmpExt:OrganisationInImageName", "Bag"),
    ("organisationCode", "Organisation Code", "IPTC Extension", "Iptc4xmpExt:OrganisationInImageCode", "Bag"),
    ("modelAge", "Model Age", "IPTC Extension", "Iptc4xmpExt:ModelAge", "Bag"),
    ("additionalModelInfo", "Additional Model Information", "IPTC Extension", "Iptc4xmpExt:AddlModelInfo", ""),
    // Categories
    ("category", "Category", "Categories", "photoshop:Category", ""),
    ("supplementalCategories", "Supplemental Categories", "Categories", "photoshop:SupplementalCategories", "Bag"),
    ("urgency", "Urgency", "Categories", "photoshop:Urgency", ""),
    // GPS
    ("gpsLatitude", "Latitude", "GPS", "exif:GPSLatitude", ""),
    ("gpsLongitude", "Longitude", "GPS", "exif:GPSLongitude", ""),
    ("gpsAltitude", "Altitude", "GPS", "exif:GPSAltitude", ""),
    ("gpsAltitudeRef", "Altitude Reference", "GPS", "exif:GPSAltitudeRef", ""),
    // DICOM
    ("dicomPatientName", "Patient Name", "DICOM", "DICOM:PatientName", ""),
    ("dicomPatientId", "Patient ID", "DICOM", "DICOM:PatientID", ""),
    ("dicomPatientDob", "Patient Date of Birth", "DICOM", "DICOM:PatientDOB", ""),
    ("dicomPatientSex", "Patient Sex", "DICOM", "DICOM:PatientSex", ""),
    ("dicomStudyId", "Study ID", "DICOM", "DICOM:StudyID", ""),
    ("dicomStudyPhysician", "Referring Physician", "DICOM", "DICOM:StudyPhysician", ""),
    ("dicomStudyDate", "Study Date", "DICOM", "DICOM:StudyDateTime", ""),
    ("dicomStudyDescription", "Study Description", "DICOM", "DICOM:StudyDescription", ""),
    ("dicomSeriesNumber", "Series Number", "DICOM", "DICOM:SeriesNumber", ""),
    ("dicomSeriesModality", "Series Modality", "DICOM", "DICOM:SeriesModality", ""),
    ("dicomSeriesDate", "Series Date", "DICOM", "DICOM:SeriesDateTime", ""),
    ("dicomSeriesDescription", "Series Description", "DICOM", "DICOM:SeriesDescription", ""),
    ("dicomEquipmentInstitution", "Equipment Institution", "DICOM", "DICOM:EquipmentInstitution", ""),
    ("dicomEquipmentManufacturer", "Equipment Manufacturer", "DICOM", "DICOM:EquipmentManufacturer", ""),
];

/// The editor's tabs in order.
pub const TABS: [&str; 7] = ["Description", "Rights", "IPTC", "IPTC Extension", "Categories", "GPS", "DICOM"];

/// Namespace URI of an XMP prefix used here.
fn namespace(prefix: &str) -> &'static str {
    match prefix {
        "dc" => "http://purl.org/dc/elements/1.1/",
        "photoshop" => "http://ns.adobe.com/photoshop/1.0/",
        "xmpRights" => "http://ns.adobe.com/xap/1.0/rights/",
        "xmp" => "http://ns.adobe.com/xap/1.0/",
        "Iptc4xmpCore" => "http://iptc.org/std/Iptc4xmpCore/1.0/xmlns/",
        "Iptc4xmpExt" => "http://iptc.org/std/Iptc4xmpExt/2008-02-29/",
        "exif" => "http://ns.adobe.com/exif/1.0/",
        "DICOM" => "http://ns.adobe.com/DICOM/",
        "tiff" => "http://ns.adobe.com/tiff/1.0/",
        _ => "",
    }
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

fn xml_unescape(s: &str) -> String {
    s.replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&apos;", "'").replace("&amp;", "&")
}

/// The element `<tag …>…</tag>` (first occurrence) as (start, end, inner).
fn find_element<'a>(xmp: &'a str, tag: &str) -> Option<(usize, usize, &'a str)> {
    let open = format!("<{tag}");
    let mut from = 0;
    while let Some(i) = xmp[from..].find(&open).map(|i| i + from) {
        let after = xmp[i + open.len()..].chars().next();
        if matches!(after, Some('>' | ' ' | '/' | '\n' | '\r' | '\t')) {
            let gt = xmp[i..].find('>')? + i;
            if xmp[..gt].ends_with('/') {
                return Some((i, gt + 1, ""));
            }
            let close = format!("</{tag}>");
            let c = xmp[gt..].find(&close)? + gt;
            return Some((i, c + close.len(), &xmp[gt + 1..c]));
        }
        from = i + open.len();
    }
    None
}

fn find_attr<'a>(xmp: &'a str, prop: &str) -> Option<(usize, usize, &'a str)> {
    let key = format!(" {prop}=\"");
    let i = xmp.find(&key)?;
    let v0 = i + key.len();
    let v1 = xmp[v0..].find('"')? + v0;
    Some((i, v1 + 1, &xmp[v0..v1]))
}

fn li_values(inner: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = inner;
    while let Some((_, b, v)) = find_element(rest, "rdf:li") {
        out.push(xml_unescape(v.trim()));
        rest = &rest[b..];
    }
    if out.is_empty() && !inner.contains('<') && !inner.trim().is_empty() {
        out.push(xml_unescape(inner.trim()));
    }
    out
}

/// The values of property `prop` in `xmp` (element or attribute form).
pub fn read_property(xmp: &str, prop: &str) -> Vec<String> {
    match find_element(xmp, prop) {
        Some((_, _, inner)) => li_values(inner),
        None => find_attr(xmp, prop).map(|(_, _, v)| vec![xml_unescape(v)]).unwrap_or_default(),
    }
}

/// The editor's field values from an XMP packet.
pub fn read_fields(xmp: Option<&str>) -> Map<String, Value> {
    let x = xmp.unwrap_or("");
    let mut m = Map::new();
    for (key, _, _, prop, kind) in FIELDS {
        let vals = read_property(x, prop);
        let v = match *kind {
            "Bag" | "Seq" => json!(vals.join("; ")),
            _ => json!(vals.into_iter().next().unwrap_or_default()),
        };
        m.insert((*key).into(), v);
    }
    // Copyright status reads as a word, not XMP's True/False.
    if let Some(Value::String(s)) = m.get_mut("copyrightStatus") {
        *s = match s.as_str() {
            "True" => "copyrighted".into(),
            "False" => "publicDomain".into(),
            _ => "unknown".into(),
        };
    }
    m
}

fn empty_packet() -> String {
    "<?xpacket begin=\"\u{feff}\" id=\"W5M0MpCehiHzreSzNTczkc9d\"?>\n<x:xmpmeta xmlns:x=\"adobe:ns:meta/\">\n <rdf:RDF xmlns:rdf=\"http://www.w3.org/1999/02/22-rdf-syntax-ns#\">\n </rdf:RDF>\n</x:xmpmeta>\n<?xpacket end=\"w\"?>".to_string()
}

/// Writes `fields` (key → value) into the packet: each named property is removed, then written
/// again unless its value is empty.
pub fn write_fields(xmp: Option<&str>, fields: &Map<String, Value>) -> String {
    let mut x = xmp.filter(|s| s.contains("</rdf:RDF>")).map(str::to_string).unwrap_or_else(empty_packet);
    let mut body = String::new();
    let mut namespaces: Vec<&str> = Vec::new();
    for (key, _, _, prop, kind) in FIELDS {
        let Some(v) = fields.get(*key) else { continue };
        while let Some((a, b, _)) = find_element(&x, prop) {
            x.replace_range(a..b, "");
        }
        while let Some((a, b, _)) = find_attr(&x, prop) {
            x.replace_range(a..b, "");
        }
        let mut vals: Vec<String> = match v {
            Value::Array(a) => a.iter().filter_map(Value::as_str).map(str::trim).filter(|s| !s.is_empty()).map(str::to_string).collect(),
            Value::String(s) if *kind == "Bag" || *kind == "Seq" => s.split([';', ',']).map(str::trim).filter(|s| !s.is_empty()).map(str::to_string).collect(),
            Value::String(s) if !s.trim().is_empty() => vec![s.trim().to_string()],
            Value::Number(n) => vec![n.to_string()],
            Value::Bool(b) => vec![if *b { "True".into() } else { "False".into() }],
            _ => Vec::new(),
        };
        if *key == "copyrightStatus" {
            vals = match vals.first().map(String::as_str) {
                Some("copyrighted" | "True" | "true") => vec!["True".into()],
                Some("publicDomain" | "False" | "false") => vec!["False".into()],
                _ => Vec::new(),
            };
        }
        if vals.is_empty() {
            continue;
        }
        let prefix = prop.split(':').next().unwrap_or("");
        if !namespaces.contains(&prefix) {
            namespaces.push(prefix);
        }
        if kind.is_empty() {
            body.push_str(&format!("   <{prop}>{}</{prop}>\n", xml_escape(&vals[0])));
        } else {
            let items: String = vals
                .iter()
                .map(|v| {
                    if *kind == "Alt" {
                        format!("<rdf:li xml:lang=\"x-default\">{}</rdf:li>", xml_escape(v))
                    } else {
                        format!("<rdf:li>{}</rdf:li>", xml_escape(v))
                    }
                })
                .collect();
            body.push_str(&format!("   <{prop}><rdf:{kind}>{items}</rdf:{kind}></{prop}>\n"));
        }
    }
    if !body.is_empty() {
        let xmlns: String = namespaces.iter().map(|p| format!(" xmlns:{p}=\"{}\"", namespace(p))).collect();
        let desc = format!("  <rdf:Description rdf:about=\"\"{xmlns}>\n{body}  </rdf:Description>\n ");
        let at = x.rfind("</rdf:RDF>").unwrap_or(x.len());
        x.insert_str(at, &desc);
    }
    x
}

// ------------------------------------------------------------------ viewer

/// Every XMP property in the packet: `(property, value)`, containers joined with "; ".
pub fn xmp_properties(xmp: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut rest = xmp;
    while let Some(i) = rest.find("<rdf:Description") {
        let Some(gt) = rest[i..].find('>') else { break };
        let head = &rest[i..i + gt];
        // Attribute-form properties.
        for part in head.split_whitespace().skip(1) {
            if let Some((name, val)) = part.split_once("=\"")
                && !name.starts_with("xmlns:")
                && !name.starts_with("rdf:")
            {
                out.push((name.to_string(), xml_unescape(val.trim_end_matches(['"', '/', '>']))));
            }
        }
        let after = &rest[i + gt + 1..];
        let end = after.find("</rdf:Description>").unwrap_or(after.len());
        let inner = &after[..end];
        let mut pos = 0;
        while let Some(lt) = inner[pos..].find('<').map(|k| k + pos) {
            let tag_end = inner[lt..].find(['>', ' ', '/']).map_or(inner.len(), |k| k + lt);
            let tag = &inner[lt + 1..tag_end];
            if tag.is_empty() || tag.starts_with('/') || tag.starts_with("rdf:") || tag.starts_with('?') || tag.starts_with('!') {
                pos = tag_end.max(lt + 1);
                continue;
            }
            match find_element(&inner[lt..], tag) {
                Some((_, b, body)) => {
                    let vals = li_values(body);
                    let v = if vals.is_empty() { body.trim().to_string() } else { vals.join("; ") };
                    out.push((tag.to_string(), v));
                    pos = lt + b;
                }
                None => pos = tag_end.max(lt + 1),
            }
        }
        rest = &after[end..];
    }
    out
}

/// Known EXIF tag names (TIFF, Exif and GPS IFDs).
pub fn exif_tag_name(ifd: &str, tag: u16) -> &'static str {
    match (ifd, tag) {
        ("GPS", 0x0000) => "GPSVersionID",
        ("GPS", 0x0001) => "GPSLatitudeRef",
        ("GPS", 0x0002) => "GPSLatitude",
        ("GPS", 0x0003) => "GPSLongitudeRef",
        ("GPS", 0x0004) => "GPSLongitude",
        ("GPS", 0x0005) => "GPSAltitudeRef",
        ("GPS", 0x0006) => "GPSAltitude",
        ("GPS", 0x0007) => "GPSTimeStamp",
        ("GPS", 0x0012) => "GPSMapDatum",
        ("GPS", 0x001d) => "GPSDateStamp",
        ("GPS", _) => "",
        (_, 0x0100) => "ImageWidth",
        (_, 0x0101) => "ImageLength",
        (_, 0x0102) => "BitsPerSample",
        (_, 0x0103) => "Compression",
        (_, 0x0106) => "PhotometricInterpretation",
        (_, 0x010e) => "ImageDescription",
        (_, 0x010f) => "Make",
        (_, 0x0110) => "Model",
        (_, 0x0112) => "Orientation",
        (_, 0x011a) => "XResolution",
        (_, 0x011b) => "YResolution",
        (_, 0x0128) => "ResolutionUnit",
        (_, 0x0131) => "Software",
        (_, 0x0132) => "DateTime",
        (_, 0x013b) => "Artist",
        (_, 0x8298) => "Copyright",
        (_, 0x8769) => "ExifIFDPointer",
        (_, 0x8825) => "GPSInfoIFDPointer",
        (_, 0x829a) => "ExposureTime",
        (_, 0x829d) => "FNumber",
        (_, 0x8822) => "ExposureProgram",
        (_, 0x8827) => "ISOSpeedRatings",
        (_, 0x9000) => "ExifVersion",
        (_, 0x9003) => "DateTimeOriginal",
        (_, 0x9004) => "DateTimeDigitized",
        (_, 0x9201) => "ShutterSpeedValue",
        (_, 0x9202) => "ApertureValue",
        (_, 0x9204) => "ExposureBiasValue",
        (_, 0x9205) => "MaxApertureValue",
        (_, 0x9207) => "MeteringMode",
        (_, 0x9209) => "Flash",
        (_, 0x920a) => "FocalLength",
        (_, 0x927c) => "MakerNote",
        (_, 0x9286) => "UserComment",
        (_, 0xa001) => "ColorSpace",
        (_, 0xa002) => "PixelXDimension",
        (_, 0xa003) => "PixelYDimension",
        (_, 0xa402) => "ExposureMode",
        (_, 0xa403) => "WhiteBalance",
        (_, 0xa405) => "FocalLengthIn35mmFilm",
        (_, 0xa406) => "SceneCaptureType",
        (_, 0xa432) => "LensSpecification",
        (_, 0xa433) => "LensMake",
        (_, 0xa434) => "LensModel",
        _ => "",
    }
}

struct Tiff<'a> {
    b: &'a [u8],
    le: bool,
}

impl Tiff<'_> {
    fn u16(&self, at: usize) -> Option<u16> {
        let s = self.b.get(at..at + 2)?;
        Some(if self.le { u16::from_le_bytes([s[0], s[1]]) } else { u16::from_be_bytes([s[0], s[1]]) })
    }
    fn u32(&self, at: usize) -> Option<u32> {
        let s = self.b.get(at..at + 4)?;
        Some(if self.le { u32::from_le_bytes([s[0], s[1], s[2], s[3]]) } else { u32::from_be_bytes([s[0], s[1], s[2], s[3]]) })
    }
    fn i32(&self, at: usize) -> Option<i32> {
        self.u32(at).map(|v| v as i32)
    }
    fn value(&self, typ: u16, count: u32, at: usize) -> String {
        let n = count.min(64) as usize;
        match typ {
            1 | 7 => self
                .b
                .get(at..at + n)
                .map(|s| if typ == 7 && n > 16 { format!("{n} bytes") } else { s.iter().map(|v| v.to_string()).collect::<Vec<_>>().join(" ") })
                .unwrap_or_default(),
            2 => self.b.get(at..at + count as usize).map(|s| String::from_utf8_lossy(s).trim_end_matches('\0').trim().to_string()).unwrap_or_default(),
            3 => (0..n).filter_map(|i| self.u16(at + i * 2)).map(|v| v.to_string()).collect::<Vec<_>>().join(" "),
            4 => (0..n).filter_map(|i| self.u32(at + i * 4)).map(|v| v.to_string()).collect::<Vec<_>>().join(" "),
            9 => (0..n).filter_map(|i| self.i32(at + i * 4)).map(|v| v.to_string()).collect::<Vec<_>>().join(" "),
            5 | 10 => (0..n)
                .filter_map(|i| {
                    let (a, b) = if typ == 5 {
                        (f64::from(self.u32(at + i * 8)?), f64::from(self.u32(at + i * 8 + 4)?))
                    } else {
                        (f64::from(self.i32(at + i * 8)?), f64::from(self.i32(at + i * 8 + 4)?))
                    };
                    Some(if b == 0.0 {
                        "0".into()
                    } else if (a / b).fract() == 0.0 {
                        format!("{}", a / b)
                    } else {
                        format!("{:.4}", a / b).trim_end_matches('0').to_string()
                    })
                })
                .collect::<Vec<_>>()
                .join(" "),
            _ => format!("type {typ} × {count}"),
        }
    }
}

fn type_size(typ: u16) -> usize {
    match typ {
        1 | 2 | 6 | 7 => 1,
        3 | 8 => 2,
        4 | 9 | 11 => 4,
        5 | 10 | 12 => 8,
        _ => 1,
    }
}

/// Walks an IFD, pushing `(ifd, tag, name, value)` rows and returning the Exif/GPS sub-IFD offsets.
fn walk_ifd(t: &Tiff, ifd: &str, offset: usize, out: &mut Vec<Value>, subs: &mut Vec<(&'static str, usize)>) {
    let Some(n) = t.u16(offset) else { return };
    for i in 0..n.min(512) as usize {
        let e = offset + 2 + i * 12;
        let (Some(tag), Some(typ), Some(count), Some(raw)) = (t.u16(e), t.u16(e + 2), t.u32(e + 4), t.u32(e + 8)) else { return };
        let size = type_size(typ).saturating_mul(count as usize);
        let at = if size <= 4 { e + 8 } else { raw as usize };
        match tag {
            0x8769 if ifd == "IFD0" => subs.push(("Exif", raw as usize)),
            0x8825 if ifd == "IFD0" => subs.push(("GPS", raw as usize)),
            _ => {}
        }
        let value = t.value(typ, count, at);
        out.push(json!({"ifd": ifd, "tag": format!("0x{tag:04x}"), "name": exif_tag_name(ifd, tag), "value": value}));
    }
}

/// Every EXIF tag of a TIFF-structured EXIF block (with or without the `Exif\0\0` header).
pub fn exif_tags(exif: &[u8]) -> Vec<Value> {
    let b = exif.strip_prefix(b"Exif\0\0").unwrap_or(exif);
    let le = match b.get(0..2) {
        Some(b"II") => true,
        Some(b"MM") => false,
        _ => return Vec::new(),
    };
    let t = Tiff { b, le };
    let Some(first) = t.u32(4) else { return Vec::new() };
    let mut out = Vec::new();
    let mut subs = Vec::new();
    walk_ifd(&t, "IFD0", first as usize, &mut out, &mut subs);
    for (name, off) in subs {
        walk_ifd(&t, name, off, &mut out, &mut Vec::new());
    }
    out
}

/// IPTC-IIM record 2 dataset names.
pub fn iptc_name(dataset: u8) -> &'static str {
    match dataset {
        5 => "Object Name",
        7 => "Edit Status",
        10 => "Urgency",
        15 => "Category",
        20 => "Supplemental Category",
        25 => "Keywords",
        40 => "Special Instructions",
        55 => "Date Created",
        60 => "Time Created",
        80 => "By-line",
        85 => "By-line Title",
        90 => "City",
        92 => "Sublocation",
        95 => "Province/State",
        100 => "Country Code",
        101 => "Country",
        103 => "Original Transmission Reference",
        105 => "Headline",
        110 => "Credit",
        115 => "Source",
        116 => "Copyright Notice",
        118 => "Contact",
        120 => "Caption/Abstract",
        122 => "Writer/Editor",
        _ => "",
    }
}

/// The IPTC-IIM datasets of a `1028` resource block.
pub fn iptc_datasets(data: &[u8]) -> Vec<Value> {
    let mut out = Vec::new();
    let mut i = 0;
    while i + 5 <= data.len() {
        if data[i] != 0x1c {
            i += 1;
            continue;
        }
        let record = data[i + 1];
        let dataset = data[i + 2];
        let mut len = usize::from(u16::from_be_bytes([data[i + 3], data[i + 4]]));
        let mut at = i + 5;
        if len & 0x8000 != 0 {
            // Extended length: the low bits say how many length bytes follow.
            let n = len & 0x7fff;
            if n == 0 || n > 4 || at + n > data.len() {
                break;
            }
            len = data[at..at + n].iter().fold(0usize, |acc, b| (acc << 8) | usize::from(*b));
            at += n;
        }
        let Some(bytes) = data.get(at..at + len) else { break };
        let value = String::from_utf8_lossy(bytes).trim_end_matches('\0').to_string();
        let name = if record == 2 { iptc_name(dataset) } else { "" };
        out.push(json!({"record": record, "dataset": dataset, "name": name, "value": value}));
        i = at + len;
    }
    out
}

fn has_doc(s: &Session) -> std::result::Result<(), String> {
    s.active().map(|_| ()).ok_or_else(|| "no document open".into())
}

fn view(s: &mut Session, _: &Value) -> Result<Value> {
    let d = s.active().ok_or(EngineError::NoDocument)?;
    let m = &d.doc.metadata;
    let exif = m.exif.as_deref().map(|e| exif_tags(e)).unwrap_or_default();
    let xmp: Vec<Value> = m.xmp.as_deref().map(xmp_properties).unwrap_or_default().into_iter().map(|(p, v)| json!({"property": p, "value": v})).collect();
    let iptc: Vec<Value> = m.psd_resources.iter().filter(|(id, _, _)| *id == 1028).flat_map(|(_, _, data)| iptc_datasets(data)).collect();
    let text: Vec<Value> = m.text.iter().map(|(k, v)| json!({"key": k, "value": v})).collect();
    Ok(json!({"exif": exif, "xmp": xmp, "iptc": iptc, "text": text, "hasExif": m.exif.is_some(), "hasXmp": m.xmp.is_some()}))
}

fn fields(_: &mut Session, _: &Value) -> Result<Value> {
    let f: Vec<Value> = FIELDS.iter().map(|(k, l, tab, prop, kind)| json!({"key": k, "label": l, "tab": tab, "property": prop, "kind": kind})).collect();
    Ok(json!({"tabs": TABS, "fields": f}))
}

fn edit(s: &mut Session, p: &Value) -> Result<Value> {
    let given: Map<String, Value> = p
        .as_object()
        .map(|o| o.iter().filter(|(k, _)| FIELDS.iter().any(|f| f.0 == k.as_str())).map(|(k, v)| (k.clone(), v.clone())).collect())
        .unwrap_or_default();
    if !given.is_empty() {
        let xmp = s.active().ok_or(EngineError::NoDocument)?.doc.metadata.xmp.clone();
        let new = write_fields(xmp.as_deref(), &given);
        s.edit("Edit Metadata", |doc, _| {
            doc.metadata.xmp = Some(new);
            Ok(())
        })?;
    }
    let d = s.active().ok_or(EngineError::NoDocument)?;
    Ok(Value::Object(read_fields(d.doc.metadata.xmp.as_deref())))
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec {
            id: "image.metadata.view",
            label: "View Metadata…",
            menu: &["Image", "Metadata"],
            shortcut: None,
            params: "{} → {exif: [{ifd, tag, name, value}], xmp: [{property, value}], iptc: [{record, dataset, name, value}], text: [{key, value}]}",
            enabled: has_doc,
            run: view,
            journal: false,
        },
        CommandSpec {
            id: "image.metadata.edit",
            label: "Edit Metadata…",
            menu: &["Image", "Metadata"],
            shortcut: None,
            params: r##"{<field key>: string|[string]|null, …} (keys from image.metadata.fields: description, rights, IPTC, IPTC Extension, categories, GPS and DICOM fields; no keys: read) → the fields as stored"##,
            enabled: has_doc,
            run: edit,
            journal: true,
        },
        CommandSpec {
            id: "image.metadata.fields",
            label: "Metadata Fields",
            menu: &[],
            shortcut: None,
            params: "{} → {tabs, fields: [{key, label, tab, property, kind}]}",
            enabled: crate::commands::always,
            run: fields,
            journal: false,
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn edit_writes_and_reads_descriptive_gps_and_dicom_fields() {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 4, "height": 4})).unwrap();
        let r = s
            .execute(
                "image.metadata.edit",
                json!({"title": "Sunset", "keywords": "sky; sea", "personShown": ["Ann", "Bo"], "gpsLatitude": "37,33.2N", "dicomPatientName": "Doe^Jane", "copyrightStatus": "copyrighted", "rating": 4}),
            )
            .unwrap();
        assert_eq!(r["title"], "Sunset");
        assert_eq!(r["keywords"], "sky; sea");
        assert_eq!(r["personShown"], "Ann; Bo");
        assert_eq!(r["gpsLatitude"], "37,33.2N");
        assert_eq!(r["dicomPatientName"], "Doe^Jane");
        assert_eq!(r["copyrightStatus"], "copyrighted");
        assert_eq!(r["rating"], "4");
        let xmp = s.active().unwrap().doc.metadata.xmp.clone().unwrap();
        assert!(xmp.contains("xmlns:DICOM=\"http://ns.adobe.com/DICOM/\"") && xmp.contains("<exif:GPSLatitude>37,33.2N</exif:GPSLatitude>"));
        // The File Info dialog sees the same title.
        let info = s.execute("file.fileInfo", json!({})).unwrap();
        assert_eq!(info["title"], "Sunset");
        // Clearing removes the property; the viewer lists what is left.
        let r = s.execute("image.metadata.edit", json!({"title": ""})).unwrap();
        assert_eq!(r["title"], "");
        let v = s.execute("image.metadata.view", json!({})).unwrap();
        let props: Vec<&str> = v["xmp"].as_array().unwrap().iter().filter_map(|p| p["property"].as_str()).collect();
        assert!(props.contains(&"dc:subject") && props.contains(&"DICOM:PatientName") && !props.contains(&"dc:title"), "{props:?}");
        let f = s.execute("image.metadata.fields", json!({})).unwrap();
        assert_eq!(f["tabs"].as_array().unwrap().len(), 7);
    }

    #[test]
    fn exif_and_iptc_viewers_decode_the_raw_blocks() {
        // A little-endian TIFF with one IFD holding Make (ASCII) and Orientation (SHORT).
        let mut b = b"II\x2a\x00\x08\x00\x00\x00".to_vec();
        b.extend_from_slice(&2u16.to_le_bytes());
        // Make: tag 0x010f, ASCII, count 5, offset 0x26.
        b.extend_from_slice(&0x010fu16.to_le_bytes());
        b.extend_from_slice(&2u16.to_le_bytes());
        b.extend_from_slice(&5u32.to_le_bytes());
        b.extend_from_slice(&0x26u32.to_le_bytes());
        // Orientation: tag 0x0112, SHORT, count 1, value 6.
        b.extend_from_slice(&0x0112u16.to_le_bytes());
        b.extend_from_slice(&3u16.to_le_bytes());
        b.extend_from_slice(&1u32.to_le_bytes());
        b.extend_from_slice(&6u32.to_le_bytes());
        b.extend_from_slice(&0u32.to_le_bytes());
        assert_eq!(b.len(), 0x26);
        b.extend_from_slice(b"Acme\0");
        let tags = exif_tags(&b);
        assert_eq!(tags.len(), 2);
        assert_eq!(tags[0]["name"], "Make");
        assert_eq!(tags[0]["value"], "Acme");
        assert_eq!(tags[1]["name"], "Orientation");
        assert_eq!(tags[1]["value"], "6");
        let mut iptc = vec![0x1c, 2, 105];
        iptc.extend_from_slice(&5u16.to_be_bytes());
        iptc.extend_from_slice(b"Hello");
        iptc.extend_from_slice(&[0x1c, 2, 25]);
        iptc.extend_from_slice(&3u16.to_be_bytes());
        iptc.extend_from_slice(b"sky");
        let d = iptc_datasets(&iptc);
        assert_eq!(d.len(), 2);
        assert_eq!(d[0]["name"], "Headline");
        assert_eq!(d[0]["value"], "Hello");
        assert_eq!(d[1]["name"], "Keywords");
    }
}
