//! Exit codes, the scripting contract's first field.

/// What the command's exit status says.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Exit {
    /// Done, or nothing to do.
    Ok,
    /// Failed with nothing written, including a database rejection of COMMIT
    /// and an interrupt before it.
    Failed,
    /// The command line or environment is unusable.
    Usage,
    /// A precondition was not met: nothing was written.
    Precondition,
    /// COMMIT was sent and its outcome is unknown.
    Uncertain,
}

impl Exit {
    pub(crate) fn code(self) -> u8 {
        match self {
            Self::Ok => 0,
            Self::Failed => 1,
            Self::Usage => 2,
            Self::Precondition => 3,
            Self::Uncertain => 4,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Exit;

    #[test]
    fn every_exit_has_its_documented_code() {
        let codes: Vec<u8> = [
            Exit::Ok,
            Exit::Failed,
            Exit::Usage,
            Exit::Precondition,
            Exit::Uncertain,
        ]
        .into_iter()
        .map(Exit::code)
        .collect();
        assert_eq!(codes, [0, 1, 2, 3, 4]);
    }
}
