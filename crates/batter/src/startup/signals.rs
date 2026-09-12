#[derive(Default)]
pub(super) struct SignalPolicy {
    selected: Option<&'static str>,
    repeated: bool,
}

impl SignalPolicy {
    pub(super) fn select(mut self, name: &'static str) -> Self {
        if self.selected.is_some() {
            self.repeated = true;
        } else {
            self.selected = Some(name);
        }
        self
    }

    pub(super) fn into_selection(self) -> Result<Option<&'static str>, ()> {
        if self.repeated {
            Err(())
        } else {
            Ok(self.selected)
        }
    }
}
