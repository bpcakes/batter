//! Turning parsed arguments into a validated plan, before any connection.

use batter_flags_core::{FlagKey, SwitchState};
use batter_flags_postgres::{Archive, FlipPolicy, Request, Variant, Write};

use crate::commands::{Command, Guard, KillState, ListFilter};

/// A command whose arguments were all validated before any connection.
pub(crate) enum Plan {
    List(ListFilter),
    Show(FlagKey),
    History(FlagKey),
    Fleet {
        version: Option<u64>,
        flag: Option<FlagKey>,
    },
    Write {
        name: &'static str,
        key: FlagKey,
        request: Request,
        flip: FlipPolicy,
        after: AfterWrite,
    },
}

/// What a write reports beyond its outcome.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum AfterWrite {
    Nothing,
    /// A released switch: count the Live processes whose settings floor
    /// keeps it engaged.
    FloorWarning,
}

impl Plan {
    pub(crate) fn name(&self) -> &'static str {
        match self {
            Self::List(_) => "list",
            Self::Show(_) => "show",
            Self::History(_) => "history",
            Self::Fleet { .. } => "fleet",
            Self::Write { name, .. } => name,
        }
    }
}

/// Validates every argument of `command`, so a bad one is a usage error
/// whatever the database's state.
pub(crate) fn plan(command: Command) -> Result<Plan, String> {
    let name = command.name();
    let key =
        |text: &str| FlagKey::new(text).map_err(|_| "the key is not a valid flag key".to_owned());
    let write = |key: FlagKey, request: Request, guard: &Guard, after| Plan::Write {
        name,
        key,
        request,
        flip: guard.flip(),
        after,
    };
    Ok(match command {
        Command::List { filter } => Plan::List(filter),
        Command::Show { key: text } => Plan::Show(key(&text)?),
        Command::History { key: text } => Plan::History(key(&text)?),
        Command::Fleet { version, flag } => Plan::Fleet {
            version,
            flag: flag.as_deref().map(key).transpose()?,
        },
        Command::Set {
            key: text,
            variant,
            guard,
        } => {
            let flag = key(&text)?;
            let variant =
                Variant::new(&variant).map_err(|_| "the variant is not valid".to_owned())?;
            write(
                flag.clone(),
                Request::Write(Write::set(flag, variant, guard.change()?)),
                &guard,
                AfterWrite::Nothing,
            )
        }
        Command::Kill {
            switch,
            state,
            guard,
        } => {
            let flag = key(&switch)?;
            let (target, after) = match state {
                KillState::Engage => (SwitchState::Engaged, AfterWrite::Nothing),
                KillState::Release => (SwitchState::Released, AfterWrite::FloorWarning),
            };
            write(
                flag.clone(),
                Request::Write(Write::kill(flag, target, guard.change()?)),
                &guard,
                after,
            )
        }
        Command::Archive { key: text, guard } => {
            let flag = key(&text)?;
            write(
                flag.clone(),
                Request::Archive(Archive::new(flag, guard.change()?)),
                &guard,
                AfterWrite::Nothing,
            )
        }
        Command::Restore { key: text, guard } => {
            let flag = key(&text)?;
            write(
                flag.clone(),
                Request::Restore(Archive::new(flag, guard.change()?)),
                &guard,
                AfterWrite::Nothing,
            )
        }
    })
}
