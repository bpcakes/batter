//! Request paths shared by two Axum route patterns.
//!
//! This mirrors the matching rules of matchit 0.8.4, which the pinned Axum
//! 0.8.9 router uses. A pattern splits at `/` into segments, and `{{` and `}}`
//! are literal braces. A capture `prefix{name}` ends its segment and matches
//! `prefix` followed by any bytes except `/`; that remainder may be empty unless
//! the capture ends the pattern. A wildcard `prefix{*name}` ends the pattern and
//! matches `prefix` followed by at least one byte, including `/`.

#[cfg(test)]
mod tests;

/// Filler for capture and wildcard remainders in a constructed path.
const FILLER: u8 = b'x';

/// Whether two route patterns can match one request path.
#[derive(Debug, Eq, PartialEq)]
pub(super) enum SharedPath {
    /// No request path matches both patterns.
    Disjoint,
    /// A request path that both patterns match.
    Witness(String),
    /// A pattern is outside the syntax Axum accepts, so disjointness is unproven.
    Unanalyzed,
}

/// Construct a request path matched by both patterns, when one exists.
pub(super) fn shared_path(first: &str, second: &str) -> SharedPath {
    let (Some(first), Some(second)) = (parse(first), parse(second)) else {
        return SharedPath::Unanalyzed;
    };
    match witness(&first, &second).map(String::from_utf8) {
        None => SharedPath::Disjoint,
        Some(Ok(path)) => SharedPath::Witness(path),
        Some(Err(_)) => SharedPath::Unanalyzed,
    }
}

#[derive(Debug, Eq, PartialEq)]
enum Segment {
    /// Unescaped literal bytes.
    Literal(Vec<u8>),
    /// A literal prefix followed by a capture ending the segment.
    Capture(Vec<u8>),
    /// A literal prefix followed by a wildcard ending the pattern.
    Wildcard(Vec<u8>),
}

fn parse(pattern: &str) -> Option<Vec<Segment>> {
    let mut raw = pattern
        .as_bytes()
        .strip_prefix(b"/")?
        .split(|byte| *byte == b'/')
        .peekable();
    let mut segments = Vec::new();
    while let Some(bytes) = raw.next() {
        let segment = parse_segment(bytes)?;
        if matches!(segment, Segment::Wildcard(_)) && raw.peek().is_some() {
            return None;
        }
        segments.push(segment);
    }
    Some(segments)
}

fn parse_segment(bytes: &[u8]) -> Option<Segment> {
    let mut literal = Vec::new();
    let mut index = 0;
    while let Some(&byte) = bytes.get(index) {
        if matches!(byte, b'{' | b'}') && bytes.get(index + 1) == Some(&byte) {
            literal.push(byte);
            index += 2;
        } else if byte == b'{' {
            // A parameter must close with the segment's final byte.
            if closing_brace(bytes, index + 1)? + 1 != bytes.len() {
                return None;
            }
            return Some(if bytes.get(index + 1) == Some(&b'*') {
                Segment::Wildcard(literal)
            } else {
                Segment::Capture(literal)
            });
        } else if byte == b'}' {
            return None;
        } else {
            literal.push(byte);
            index += 1;
        }
    }
    Some(Segment::Literal(literal))
}

/// Find the first unescaped `}` after a parameter's opening brace.
fn closing_brace(bytes: &[u8], mut index: usize) -> Option<usize> {
    while let Some(&byte) = bytes.get(index) {
        if byte == b'}' {
            if bytes.get(index + 1) != Some(&b'}') {
                return Some(index);
            }
            index += 1;
        }
        index += 1;
    }
    None
}

fn witness(first: &[Segment], second: &[Segment]) -> Option<Vec<u8>> {
    let mut path = Vec::new();
    for index in 0.. {
        let pair = (first.get(index), second.get(index));
        match pair {
            (None, None) => return Some(path),
            (Some(Segment::Wildcard(a)), Some(Segment::Wildcard(b))) => {
                path.push(b'/');
                path.extend_from_slice(longer_compatible(a, b)?);
                path.push(FILLER);
                return Some(path);
            }
            (Some(Segment::Wildcard(prefix)), Some(_)) => {
                return wildcard_tail(path, prefix, &second[index..]);
            }
            (Some(_), Some(Segment::Wildcard(prefix))) => {
                return wildcard_tail(path, prefix, &first[index..]);
            }
            (Some(a), Some(b)) => {
                let last = index + 1 == first.len();
                path.push(b'/');
                path.extend(segment_witness(a, b, last)?);
            }
            (None, Some(_)) | (Some(_), None) => return None,
        }
    }
    None
}

/// One segment matched by two non-wildcard segments at the same position.
///
/// `last` is exact for any pair that can still succeed: both patterns must
/// end at the same position when neither reaches a wildcard.
fn segment_witness(first: &Segment, second: &Segment, last: bool) -> Option<Vec<u8>> {
    match (first, second) {
        (Segment::Literal(a), Segment::Literal(b)) => (a == b).then(|| a.clone()),
        (Segment::Literal(literal), Segment::Capture(prefix))
        | (Segment::Capture(prefix), Segment::Literal(literal)) => {
            let remainder_allowed = !last || literal.len() > prefix.len();
            (literal.starts_with(prefix) && remainder_allowed).then(|| literal.clone())
        }
        (Segment::Capture(a), Segment::Capture(b)) => {
            let mut segment = longer_compatible(a, b)?.to_vec();
            segment.push(FILLER);
            Some(segment)
        }
        (Segment::Wildcard(_), _) | (_, Segment::Wildcard(_)) => None,
    }
}

/// Complete a path whose remainder a wildcard with `prefix` must match.
fn wildcard_tail(mut path: Vec<u8>, prefix: &[u8], rest: &[Segment]) -> Option<Vec<u8>> {
    let (head, tail) = rest.split_first()?;
    path.push(b'/');
    match head {
        Segment::Literal(literal) => {
            // The wildcard needs at least one byte after its prefix.
            if !literal.starts_with(prefix) || (literal.len() == prefix.len() && tail.is_empty()) {
                return None;
            }
            path.extend_from_slice(literal);
        }
        Segment::Capture(capture) | Segment::Wildcard(capture) => {
            path.extend_from_slice(longer_compatible(capture, prefix)?);
            path.push(FILLER);
        }
    }
    for segment in tail {
        path.push(b'/');
        match segment {
            Segment::Literal(literal) => path.extend_from_slice(literal),
            Segment::Capture(prefix) | Segment::Wildcard(prefix) => {
                path.extend_from_slice(prefix);
                path.push(FILLER);
            }
        }
    }
    Some(path)
}

fn longer_compatible<'a>(first: &'a [u8], second: &'a [u8]) -> Option<&'a [u8]> {
    if first.starts_with(second) {
        Some(first)
    } else if second.starts_with(first) {
        Some(second)
    } else {
        None
    }
}
