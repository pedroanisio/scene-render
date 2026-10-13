//! Bounds on the scene documents a load accepts, in one place.
//!
//! A document is untrusted input: every limit here holds before the parser allocates for it or recurses into it,
//! and a document past one is refused with a diagnostic rather than exhausting memory or the stack. Each limit is far
//! above what a real document needs; the largest in the corpus is a few kilobytes and a few hundred nodes.

use std::io::Read as _;
use std::path::Path;

/// Bytes a scene document may hold (64 MiB). A file past it is refused before it is read; the bound also caps the
/// attribute text, which [`MAX_XML_NODES`] does not count.
pub const MAX_DOCUMENT_BYTES: u64 = 64 << 20;

/// XML nodes (elements, text, comments and processing instructions) a document may hold. A node program's output
/// may hold up to [`crate::program::MAX_ELEMENTS`] elements, and the document holding it is parsed again with the
/// output in place, so the bound leaves room for those, the text between them and the document around them.
pub const MAX_XML_NODES: u32 = 4_000_000;

/// Levels elements may nest below the root element (`S12`). The parser and the validators descend recursively, so
/// the nesting is measured on the raw text before the parse.
pub const MAX_DEPTH: usize = 256;

/// Levels an `<include>` may nest: a document including one that includes another, and so on (`E13`).
pub const MAX_INCLUDE_DEPTH: usize = 16;

/// The parse options every scene document is read with: no DTD, and at most [`MAX_XML_NODES`] nodes.
pub fn parsing_options() -> roxmltree::ParsingOptions {
    roxmltree::ParsingOptions { allow_dtd: false, nodes_limit: MAX_XML_NODES }
}

/// The byte offset of the first start tag nested more than `max` levels below the root element, found without
/// recursion. Comments, CDATA sections, processing instructions and declarations are skipped, and a `>` inside a
/// quoted attribute value does not end its tag. Malformed text is measured as far as it reads; the parse reports it.
pub fn too_deep(xml: &str, max: usize) -> Option<usize> {
    let b = xml.as_bytes();
    let find = |from: usize, pat: &[u8]| {
        b.get(from..).and_then(|r| r.windows(pat.len()).position(|w| w == pat)).map(|p| from + p + pat.len())
    };
    let mut depth = 0usize;
    let mut i = 0;
    while let Some(p) = b.get(i..).and_then(|r| r.iter().position(|&c| c == b'<')).map(|p| i + p) {
        let rest = &b[p..];
        let end = if rest.starts_with(b"<!--") {
            find(p + 4, b"-->")
        } else if rest.starts_with(b"<![CDATA[") {
            find(p + 9, b"]]>")
        } else if rest.starts_with(b"<?") {
            find(p + 2, b"?>")
        } else if rest.starts_with(b"<!") {
            find(p + 2, b">")
        } else if rest.starts_with(b"</") {
            depth = depth.saturating_sub(1);
            find(p + 2, b">")
        } else {
            let mut quote = None;
            let mut j = p + 1;
            while j < b.len() {
                match (quote, b[j]) {
                    (Some(q), c) if c == q => quote = None,
                    (None, c @ (b'"' | b'\'')) => quote = Some(c),
                    (None, b'>') => break,
                    _ => {}
                }
                j += 1;
            }
            if j >= b.len() {
                return None;
            }
            if b[j - 1] != b'/' {
                depth += 1;
                // the root element is depth 0
                if depth > max + 1 {
                    return Some(p);
                }
            }
            Some(j + 1)
        };
        i = end?;
    }
    None
}

/// Reads a file of at most `limit` bytes. A larger file is refused with [`std::io::ErrorKind::FileTooLarge`] before
/// its content is read, and a file that grows while it is read is refused once it passes the limit.
pub fn read_capped(path: &Path, limit: u64) -> std::io::Result<Vec<u8>> {
    let file = std::fs::File::open(path)?;
    let too_large =
        || std::io::Error::new(std::io::ErrorKind::FileTooLarge, format!("the file is larger than {limit} bytes"));
    let len = file.metadata()?.len();
    if len > limit {
        return Err(too_large());
    }
    let mut bytes = Vec::with_capacity(len as usize);
    file.take(limit + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > limit {
        return Err(too_large());
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::too_deep;

    #[test]
    fn depth_counts_open_elements_only() {
        let xml = "<a><!-- <b><b> --><?p <b> ?><![CDATA[<b><b>]]><b x='>' y=\"/>\"><c/></b><b/></a>";
        assert_eq!(too_deep(xml, 1), None);
        assert_eq!(too_deep(xml, 0), Some(xml.find("<b x").unwrap()));
        assert_eq!(too_deep("<a><b><c/></b></a>", 0), Some(3));
        assert_eq!(too_deep("<a><b></b><b></b></a>", 1), None);
        assert_eq!(too_deep("<a><b", 0), None);
    }
}
