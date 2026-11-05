//! Reading one declaration's setting: applicability and strict literals.

use std::{env::VarError, ffi::OsStr};

use batter_core::settings::SettingsSource;
use batter_flags_core::{
    Absence, Declared, FlagValue, SettingName, SwitchState, Toggle, ValueType,
};

use super::{
    applicability::{Applicability, Process},
    error::{DeclaredError, ValueProblem},
};

/// What a setting supplied, already checked against its declaration.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Literal {
    /// A release or mode flag's variant.
    Variant(&'static str),
    /// A kill switch's state.
    Switch(SwitchState),
}

/// What reading a declaration's setting found.
pub(super) struct Read {
    pub(super) applicability: Applicability,
    pub(super) literal: Option<Literal>,
}

/// Reads the setting a declaration names.
///
/// A value presented where none may be fails before it is parsed, so a blank
/// or mistyped value for a flag the process does not own reports the
/// ownership, which is what an operator has to fix. Absence is judged only
/// where a value may be presented.
pub(super) fn read(
    source: &SettingsSource,
    declared: &Declared<'_>,
    process: Process<'_>,
) -> Result<Read, DeclaredError> {
    let name = declared.setting_name();
    let applicability = Applicability::of(declared, process);
    let literal = match (source.get(name.as_str()), &applicability) {
        (None, Applicability::Presentable) if declared.absence() == Absence::Required => {
            return Err(DeclaredError::Missing {
                key: declared.key().clone(),
                name,
            });
        }
        (None, _) => None,
        (Some(_), Applicability::NotOwned) => {
            return Err(DeclaredError::NotOwned {
                key: declared.key().clone(),
                name,
                kind: process.kind.name().to_owned(),
            });
        }
        (Some(_), Applicability::OutsideClass(class)) => {
            return Err(DeclaredError::OutsideClass {
                key: declared.key().clone(),
                name,
                class: class.name().to_owned(),
            });
        }
        (Some(raw), Applicability::Presentable) => Some(parse(declared, name, raw)?),
    };
    Ok(Read {
        applicability,
        literal,
    })
}

fn parse(
    declared: &Declared<'_>,
    name: SettingName,
    raw: &OsStr,
) -> Result<Literal, DeclaredError> {
    let invalid = |problem: ValueProblem, cause: Option<VarError>| DeclaredError::InvalidValue {
        key: declared.key().clone(),
        name: name.clone(),
        problem,
        cause,
    };
    let Some(text) = raw.to_str() else {
        let cause = VarError::NotUnicode(raw.to_owned());
        return Err(invalid(ValueProblem::NotUnicode, Some(cause)));
    };
    if text.trim().is_empty() {
        return Err(invalid(ValueProblem::Blank, None));
    }
    match declared.value_type() {
        ValueType::Toggle => Toggle::from_setting(text)
            .map(|toggle| Literal::Variant(toggle.variant()))
            .ok_or_else(|| invalid(ValueProblem::NotAToggle, None)),
        ValueType::Enum => declared
            .variants()
            .iter()
            .copied()
            .find(|variant| *variant == text)
            .map(Literal::Variant)
            .ok_or_else(|| invalid(ValueProblem::NotADeclaredVariant, None)),
        ValueType::Switch => match text {
            "engaged" => Ok(Literal::Switch(SwitchState::Engaged)),
            "released" => Ok(Literal::Switch(SwitchState::Released)),
            _ => Err(invalid(ValueProblem::NotASwitchState, None)),
        },
    }
}
