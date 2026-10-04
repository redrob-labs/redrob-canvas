// SPDX-License-Identifier: GPL-3.0-or-later

//! PostScript and Encapsulated PostScript (M.8) — **detected, and rendering refused.**
//!
//! Re-derived from `plug-ins/common/file-ps.c` (GPL-3.0-or-later), pinned in
//! `docs/upstream-sources.toml`.
//!
//! # Why this refuses, and why that is not the same as being unfinished
//!
//! **Upstream renders PostScript by invoking Ghostscript** and reading back what it writes —
//! `pbmraw`, `pgmraw`, `pnmraw` or `ppmraw` for a PS, `pngalpha` for an EPS. So its PostScript
//! support is a PNM reader bolted to a subprocess. This product will not spawn an external GPL
//! binary: that is a deployment dependency rather than a codec, and a format that works only when
//! someone else's program happens to be installed is worse than one that says plainly it does not.
//!
//! **A pure-Rust interpreter DOES exist and is named rather than wished away**: `stet 0.8.3`,
//! *"Pure-Rust PostScript Level 3 interpreter — renders PostScript and EPS to raster images or
//! PDF, with no Ghostscript required."* It is not adopted, and the reason is this product's own
//! precedent rather than a doubt about that crate. `crate::pdf` (H.15) already faced the smaller
//! version of this question and wrote the answer down:
//!
//! > A PDF page is a program — a content stream of drawing operators over fonts, shadings and
//! > transparency groups — and running it is a graphics engine, not a file reader. That engine is
//! > not attempted here, and saying so is the point: a half-written interpreter renders
//! > *something* for every file, and a page that silently loses its text looks like a bug in this
//! > product rather than a feature it never had.
//!
//! **PostScript is more of a program than PDF, not less** — it is Turing-complete, with a stack, a
//! dictionary and user-defined operators. If a rasteriser is the wrong thing to half-build for
//! PDF, it is the wrong thing to adopt wholesale for PostScript, where "did it render correctly"
//! is not a question this repository can answer for arbitrary input.
//!
//! **And not reading the embedded preview is PARITY, not a shortfall.** Upstream registers the DOS
//! EPS binary magic but never reads the preview it points at; its own thumbnail loader carries the
//! admission: *"We should look for an embedded preview but for now we just load the document at a
//! small resolution and the first page only."* It runs Ghostscript even for a thumbnail.
//!
//! So what IS implemented is detection, which is the part that can be re-derived and tested — and
//! a file that is recognised and refused by name is strictly better than one that is not
//! recognised at all.
//!
//! # What upstream registers, and the rule worth having
//!
//! Both the PS and the EPS procedure register the SAME magics: `0,string,%!,0,long,0xc5d0d3c6` —
//! either `%!` at offset 0, or the DOS EPS binary header `C5 D0 D3 C6`.
//!
//! **Telling EPS from PS is a DISTANCE test, not a substring test**, and that is the find of this
//! item. `ps_read_header` does:
//!
//! ```text
//! adobe = strstr (hdr, "PS-Adobe-");
//! epsf  = strstr (hdr, "EPSF-");
//! if ((adobe != NULL) && (epsf != NULL))
//!   ds = epsf - adobe;
//! *is_epsf = ((ds >= 11) && (ds <= 15));
//! ```
//!
//! Both substrings present is NOT enough — `"EPSF-"` must begin **11 to 15 bytes** after
//! `"PS-Adobe-"`. The window exists because the real header is `%!PS-Adobe-3.0 EPSF-3.0`, so what
//! sits between them is a version and a space; a document merely *mentioning* EPSF further down
//! its header falls outside the window and is a plain PS. Only the first **512 bytes** are
//! examined, which is upstream's own `hdr[512]`.
//!
//! A DOS EPS binary header sets it unconditionally, without the distance test.

use crate::{FormatError, Result};

/// The DOS EPS binary file header, as upstream spells it: `{ 0xc5, 0xd0, 0xd3, 0xc6, 0 }`.
const DOS_EPS_MAGIC: [u8; 4] = [0xc5, 0xd0, 0xd3, 0xc6];

/// Upstream reads exactly this much into `hdr[512]` and looks no further.
const HEADER_WINDOW: usize = 512;

/// Is this PostScript at all — either flavour?
pub(crate) fn looks_like_postscript(bytes: &[u8]) -> bool {
    bytes.starts_with(b"%!") || bytes.starts_with(&DOS_EPS_MAGIC)
}

/// Is it EPS specifically?
///
/// The distance rule above, plus the unconditional DOS EPS case. Returns false for a plain PS, so
/// a caller gets the two apart the way upstream's two procedures are apart.
pub(crate) fn is_encapsulated(bytes: &[u8]) -> bool {
    if bytes.starts_with(&DOS_EPS_MAGIC) {
        return true;
    }
    let window = &bytes[..bytes.len().min(HEADER_WINDOW)];
    // Upstream works on a `char[512]` with `strstr`, so a non-text byte simply ends the search.
    // Treating the window as bytes rather than requiring valid UTF-8 matches that: a PostScript
    // header is ASCII, and a stray high byte later in the window must not make the file
    // undetectable.
    let Some(adobe) = find(window, b"PS-Adobe-") else {
        return false;
    };
    let Some(epsf) = find(window, b"EPSF-") else {
        return false;
    };
    // `ds = epsf - adobe` in upstream, which is signed there; a negative distance cannot satisfy
    // `>= 11` so an `EPSF-` appearing BEFORE `PS-Adobe-` is not EPS either.
    let Some(distance) = epsf.checked_sub(adobe) else {
        return false;
    };
    (11..=15).contains(&distance)
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

/// Rendering, which is refused.
///
/// The message names the flavour, because "we detected your file and will not draw it" is more
/// useful when it says which of the two it decided you had.
pub(crate) fn refuse_render(bytes: &[u8]) -> FormatError {
    if is_encapsulated(bytes) {
        FormatError::UnsupportedFeature(
            "Encapsulated PostScript needs an interpreter; this product detects but does not render it",
        )
    } else {
        FormatError::UnsupportedFeature(
            "PostScript needs an interpreter; this product detects but does not render it",
        )
    }
}

/// Export, likewise refused, and for a second reason on top of the first.
///
/// Upstream can WRITE PostScript, and writing it needs no interpreter — so this refusal is a scope
/// decision rather than a capability one. It is listed with the read refusal so neither is mistaken
/// for an oversight.
pub(crate) fn refuse_export() -> Result<Vec<u8>> {
    Err(FormatError::UnsupportedFeature("PostScript export (not implemented)").into())
}
