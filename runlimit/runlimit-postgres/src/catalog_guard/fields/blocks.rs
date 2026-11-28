//! The query blocks of a statement, which bound where an alias is visible.
//!
//! A parenthesized query (a subquery, a CTE body, an `EXISTS` or `IN`
//! operand) is a block of its own, and so is each arm of a set operation,
//! a sibling of the arm before it. PostgreSQL resolves a qualified name in
//! its own block and then outward, never in a child or sibling block.

use std::collections::BTreeMap;

use super::{Token, from_items::opens_query, is_name};

/// The block id of the statement's top level; any other block's id is the
/// index of the token that opens it.
const ROOT: usize = usize::MAX;

/// The block each token sits in, and the block enclosing each block.
pub(super) struct Blocks {
    of: Vec<usize>,
    parent: BTreeMap<usize, usize>,
}

impl Blocks {
    pub(super) fn new(tokens: &[Token]) -> Self {
        let mut of = Vec::with_capacity(tokens.len());
        let mut parent = BTreeMap::new();
        // The block outside each open parenthesis, restored at its close.
        let mut outside = Vec::new();
        let mut current = ROOT;
        for (index, token) in tokens.iter().enumerate() {
            match token {
                Token::Punct('(') => {
                    of.push(current);
                    outside.push(current);
                    if opens_query(tokens, index) {
                        parent.insert(index, current);
                        current = index;
                    }
                }
                Token::Punct(')') => {
                    current = outside.pop().unwrap_or(ROOT);
                    of.push(current);
                }
                Token::Word(word)
                    if ["union", "intersect", "except"].contains(&word.as_str())
                        && !is_name(tokens, index) =>
                {
                    // The next arm shares the enclosing block of the last.
                    if let Some(&enclosing) = parent.get(&current) {
                        parent.insert(index, enclosing);
                    }
                    current = index;
                    of.push(current);
                }
                _ => of.push(current),
            }
        }
        Self { of, parent }
    }

    /// The block holding token `index`.
    pub(super) fn of(&self, index: usize) -> usize {
        self.of.get(index).copied().unwrap_or(ROOT)
    }

    /// Whether a name declared in block `declared` is visible in block
    /// `used`: the same block or one enclosing it.
    pub(super) fn sees(&self, used: usize, declared: usize) -> bool {
        let mut block = used;
        loop {
            if block == declared {
                return true;
            }
            match self.parent.get(&block) {
                Some(&enclosing) => block = enclosing,
                None => return false,
            }
        }
    }
}
