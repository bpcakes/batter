//! Routers indexed by the request paths their route patterns match.

use super::{Segment, parse};
use std::collections::HashMap;

/// The route patterns of several routers, indexed segment by segment.
///
/// [`Index::candidate`] names, in router order, each router with a pattern that
/// matches a request path under the module's matchit 0.8.4 rules. It knows each
/// pattern alone, not route priority, so it may name a router whose own routing
/// sends the path to another route; it never omits a router with a matching
/// pattern. A pattern outside the analyzed syntax, which assembly rejects
/// before indexing, would make its router a candidate for every path.
///
/// A lookup's stack holds at most one frame per capture, which parsing limits
/// to 25 per pattern, and teardown uses constant stack, however many literal
/// segments a pattern or path has.
pub(in crate::boundary) struct Index {
    root: Node,
    unanalyzed: Vec<usize>,
}

/// The patterns that continue after the segments leading to this node.
///
/// Routers are indexed in order, so every router list here is ascending.
#[derive(Default)]
struct Node {
    /// Routers with a pattern ending in the segment that leads here.
    ends: Vec<usize>,
    /// Next nodes by literal segment.
    literals: HashMap<Vec<u8>, Node>,
    /// Next nodes by the literal prefix of a capture segment.
    captures: Vec<(Vec<u8>, Node)>,
    /// Routers by the literal prefix of the wildcard ending their pattern.
    wildcards: Vec<(Vec<u8>, Vec<usize>)>,
}

impl Index {
    /// Index each router's patterns under its position in `routers`.
    pub(in crate::boundary) fn new<R, P>(routers: R) -> Self
    where
        R: IntoIterator,
        R::Item: IntoIterator<Item = P>,
        P: AsRef<str>,
    {
        let mut index = Self {
            root: Node::default(),
            unanalyzed: Vec::new(),
        };
        for (router, patterns) in routers.into_iter().enumerate() {
            for pattern in patterns {
                match parse(pattern.as_ref()) {
                    Some(segments) => index.root.insert(segments, router),
                    None => index.unanalyzed.push(router),
                }
            }
        }
        index
    }

    /// The lowest position, from `from` on, of a router with a pattern
    /// matching `path`.
    pub(in crate::boundary) fn candidate(&self, path: &str, from: usize) -> Option<usize> {
        let mut lowest = None;
        lower(&mut lowest, &self.unanalyzed, from);
        if let Some(rest) = path.as_bytes().strip_prefix(b"/") {
            self.root.collect(rest, from, &mut lowest);
        }
        lowest
    }
}

/// Lower `lowest` to the first of the ascending `routers` from `from` on.
fn lower(lowest: &mut Option<usize>, routers: &[usize], from: usize) {
    if let Some(&router) = routers.iter().find(|&&router| router >= from) {
        *lowest = Some(lowest.map_or(router, |lowest| lowest.min(router)));
    }
}

impl Node {
    fn insert(&mut self, segments: Vec<Segment>, router: usize) {
        let mut node = self;
        for segment in segments {
            node = match segment {
                Segment::Literal(literal) => node.literals.entry(literal).or_default(),
                Segment::Capture(prefix) => {
                    let position =
                        match node.captures.iter().position(|(known, _)| *known == prefix) {
                            Some(position) => position,
                            None => {
                                node.captures.push((prefix, Node::default()));
                                node.captures.len() - 1
                            }
                        };
                    &mut node.captures[position].1
                }
                // A wildcard ends its pattern.
                Segment::Wildcard(prefix) => {
                    match node
                        .wildcards
                        .iter_mut()
                        .find(|(known, _)| *known == prefix)
                    {
                        Some((_, routers)) => routers.push(router),
                        None => node.wildcards.push((prefix, vec![router])),
                    }
                    return;
                }
            };
        }
        node.ends.push(router);
    }

    /// Lower `lowest` to the routers from `from` on with a pattern matching
    /// `rest`, the path after the `/` that starts the next segment.
    ///
    /// Each node is reached through one segment of the path, so a lookup
    /// visits a node at most once. Literal segments are followed in a loop and
    /// only a capture recurses, so the depth of recursion is at most the
    /// number of captures in one pattern.
    fn collect(&self, mut rest: &[u8], from: usize, lowest: &mut Option<usize>) {
        let mut node = self;
        loop {
            for (prefix, routers) in &node.wildcards {
                // The wildcard takes the prefix and at least one more byte, `/` included.
                if rest.len() > prefix.len() && rest.starts_with(prefix) {
                    lower(lowest, routers, from);
                }
            }
            let (segment, next) = match rest.iter().position(|&byte| byte == b'/') {
                Some(end) => (&rest[..end], Some(&rest[end + 1..])),
                None => (rest, None),
            };
            for (prefix, capture) in &node.captures {
                if !segment.starts_with(prefix) {
                    continue;
                }
                match next {
                    Some(next) => capture.collect(next, from, lowest),
                    // A capture's remainder may be empty unless the capture ends the pattern.
                    None if segment.len() > prefix.len() => lower(lowest, &capture.ends, from),
                    None => {}
                }
            }
            let Some(literal) = node.literals.get(segment) else {
                return;
            };
            match next {
                Some(next) => (node, rest) = (literal, next),
                None => return lower(lowest, &literal.ends, from),
            }
        }
    }

    /// Move this node's children to `pending`, leaving it without any.
    fn release_children(&mut self, pending: &mut Vec<Node>) {
        pending.extend(self.literals.drain().map(|(_, node)| node));
        pending.extend(self.captures.drain(..).map(|(_, node)| node));
    }
}

impl Drop for Node {
    /// Dismantle the subtree from a work list: dropping nested children in
    /// place would recurse once per literal segment of the deepest pattern.
    fn drop(&mut self) {
        let mut pending = Vec::new();
        self.release_children(&mut pending);
        while let Some(mut node) = pending.pop() {
            node.release_children(&mut pending);
        }
    }
}
