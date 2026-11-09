//! The name checks a multi-part registration makes before it registers.

use super::Supervisor;
use crate::RegistrationError;

impl Supervisor {
    /// Refuses every name a registration of `components` and `cleanups`
    /// would refuse, before anything is registered.
    pub(crate) fn check_names(
        &self,
        components: &[&'static str],
        cleanups: &[&'static str],
    ) -> Result<(), RegistrationError> {
        for (index, name) in components.iter().enumerate() {
            self.check_component_name(name)?;
            if components[..index].contains(name) {
                return Err(RegistrationError::Duplicate(name));
            }
        }
        for (index, name) in cleanups.iter().enumerate() {
            self.cleanup.check_name(name)?;
            if cleanups[..index].contains(name) {
                return Err(RegistrationError::Duplicate(name));
            }
        }
        Ok(())
    }
}
