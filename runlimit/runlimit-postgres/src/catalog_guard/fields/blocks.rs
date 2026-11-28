//! The query blocks of a statement, which bound where an alias is visible.
//!
//! A parenthesized query (a subquery, a CTE body, an `EXISTS` or `IN`
//! operand) is a block of its own, and so is each arm of a set operation,
//! which shares the enclosing block of the first arm. PostgreSQL resolves a
//! qualified name in its own block and then outward, never in a child or
//! sibling block. A FROM-item subquery without `LATERAL` skips the block
//! holding its FROM list, whose items it may not read, and a CTE body sees
//! no enclosing block at all.

use std::collections::BTreeMap;

use super::{Token, from_items::opens_query, is_name};

/// The block id of the statement's top level; any other block's id is the
/// index of the token that opens it.
const ROOT: usize = usize::MAX;

/// The block each token sits in, and the block enclosing each block.
pub(super) struct Blocks {
    of: Vec<usize>,
    /// The block enclosing each parenthesized block.
    parent: BTreeMap<usize, usize>,
    /// The first arm of each later set-operation arm: the parenthesized
    /// block or `ROOT` whose enclosing block the arm shares.
    arms: BTreeMap<usize, usize>,
}

impl Blocks {
    pub(super) fn new(tokens: &[Token]) -> Self {
        let mut blocks = Self {
            of: Vec::with_capacity(tokens.len()),
            parent: BTreeMap::new(),
            arms: BTreeMap::new(),
        };
        // The block outside each open parenthesis, restored at its close.
        let mut outside = Vec::new();
        let mut current = ROOT;
        for (index, token) in tokens.iter().enumerate() {
            match token {
                Token::Punct('(') => {
                    blocks.of.push(current);
                    outside.push(current);
                    if opens_query(tokens, index) {
                        blocks.parent.insert(index, current);
                        current = index;
                    }
                }
                Token::Punct(')') => {
                    current = outside.pop().unwrap_or(ROOT);
                    blocks.of.push(current);
                }
                Token::Word(word)
                    if ["union", "intersect", "except"].contains(&word.as_str())
                        && !is_name(tokens, index) =>
                {
                    let first = blocks.arms.get(&current).copied().unwrap_or(current);
                    blocks.arms.insert(index, first);
                    current = index;
                    blocks.of.push(current);
                }
                _ => blocks.of.push(current),
            }
        }
        blocks
    }

    /// The block holding token `index`.
    pub(super) fn of(&self, index: usize) -> usize {
        self.of.get(index).copied().unwrap_or(ROOT)
    }

    /// The block enclosing `block`, if any.
    fn enclosing(&self, block: usize) -> Option<usize> {
        let first = self.arms.get(&block).copied().unwrap_or(block);
        self.parent.get(&first).copied()
    }

    /// Makes the FROM-item subquery opened at `open`, which lacks
    /// `LATERAL`, see past the block holding its FROM list.
    pub(super) fn skip_level(&mut self, open: usize) {
        if let Some(holding) = self.parent.get(&open).copied() {
            match self.enclosing(holding) {
                Some(outer) => self.parent.insert(open, outer),
                None => self.parent.remove(&open),
            };
        }
    }

    /// Makes the CTE body opened at `open` see no enclosing block.
    pub(super) fn isolate(&mut self, open: usize) {
        self.parent.remove(&open);
    }

    /// Whether a name declared in block `declared` is visible in block
    /// `used`: the same block or one enclosing it.
    pub(super) fn sees(&self, used: usize, declared: usize) -> bool {
        let mut block = used;
        loop {
            if block == declared {
                return true;
            }
            match self.enclosing(block) {
                Some(outer) => block = outer,
                None => return false,
            }
        }
    }
}
