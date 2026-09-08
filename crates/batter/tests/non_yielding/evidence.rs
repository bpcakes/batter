use std::{ops::Range, time::Instant};

const OUTPUT_LIMIT: usize = 64 * 1024;
const EVENT_LIMIT: usize = 64;
const EVENT_PREFIX: &[u8] = b"batter-fixture:";

#[derive(Clone, Copy, Debug)]
enum CaptureLimit {
    Bytes,
    Events,
}

#[derive(Clone)]
struct Event {
    name: Range<usize>,
    observed_at: Instant,
}

#[derive(Clone, Default)]
pub struct Output {
    bytes: Vec<u8>,
    exceeded: Option<CaptureLimit>,
    parsed: usize,
    events: Vec<Event>,
}

impl std::fmt::Debug for Output {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Output")
            .field("text", &self.text())
            .field("exceeded", &self.exceeded)
            .field("event_count", &self.events.len())
            .finish()
    }
}

impl Output {
    pub fn record(&mut self, bytes: &[u8], observed_at: Instant) {
        let retained = bytes.len().min(OUTPUT_LIMIT - self.bytes.len());
        self.bytes.extend_from_slice(&bytes[..retained]);
        if retained < bytes.len() {
            self.exceeded.get_or_insert(CaptureLimit::Bytes);
        }
        while let Some(newline) = self.bytes[self.parsed..]
            .iter()
            .position(|byte| *byte == b'\n')
        {
            let end = self.parsed + newline;
            let line = &self.bytes[self.parsed..end];
            let line = line.strip_suffix(b"\r").unwrap_or(line);
            if let Some(name) = line.strip_prefix(EVENT_PREFIX)
                && !name.is_empty()
                && std::str::from_utf8(name).is_ok()
                && !self
                    .events
                    .iter()
                    .any(|event| self.bytes[event.name.clone()] == *name)
            {
                if self.events.len() == EVENT_LIMIT {
                    self.exceeded.get_or_insert(CaptureLimit::Events);
                } else {
                    let start = self.parsed + EVENT_PREFIX.len();
                    self.events.push(Event {
                        name: start..start + name.len(),
                        observed_at,
                    });
                }
            }
            self.parsed = end + 1;
        }
    }

    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.bytes).into_owned()
    }

    fn check_capture_limit(&self) -> Result<(), String> {
        if let Some(limit) = self.exceeded {
            let reason = match limit {
                CaptureLimit::Bytes => format!("{OUTPUT_LIMIT} bytes"),
                CaptureLimit::Events => format!("{EVENT_LIMIT} distinct events"),
            };
            return Err(format!(
                "child output exceeded capture limit ({reason}): {self:?}"
            ));
        }
        Ok(())
    }

    pub fn check(&self) -> Result<(), String> {
        self.check_capture_limit()?;
        if self.text().contains("panicked at") {
            return Err(format!("child panicked: {self:?}"));
        }
        Ok(())
    }

    pub(super) fn observed(&self, name: &str) -> Option<Instant> {
        self.events
            .iter()
            .find(|event| &self.bytes[event.name.clone()] == name.as_bytes())
            .map(|event| event.observed_at)
    }

    pub fn event_by(
        &self,
        name: &str,
        deadline: Instant,
        now: Instant,
    ) -> Result<Option<Instant>, String> {
        self.check_capture_limit()?;
        match self.observed(name) {
            // Evidence time, not the scheduling of the polling thread, decides
            // whether startup succeeded. Later panic diagnostics do not erase
            // that event; final validation still rejects the child's panic.
            Some(at) if at <= deadline => Ok(Some(at)),
            Some(_) => Err(format!("child emitted {name} after its deadline: {self:?}")),
            None => {
                // With no startup record, fail promptly on a known child panic.
                self.check()?;
                if now >= deadline {
                    Err(format!("child did not emit {name}: {self:?}"))
                } else {
                    Ok(None)
                }
            }
        }
    }

    pub fn validate(&self, required: &[&str], forbidden: &[&str]) -> Result<(), String> {
        self.check()?;
        // A kill can interrupt emission. An unfinished protocol record cannot
        // prove absence of a forbidden event, even if earlier evidence exists.
        let tail = &self.bytes[self.parsed..];
        if !tail.is_empty() && (tail.starts_with(EVENT_PREFIX) || EVENT_PREFIX.starts_with(tail)) {
            return Err(format!("incomplete fixture record: {self:?}"));
        }
        for event in required {
            if self.observed(event).is_none() {
                return Err(format!("missing event {event}: {self:?}"));
            }
        }
        for event in forbidden {
            if self.observed(event).is_some() {
                return Err(format!("unexpected event {event}: {self:?}"));
            }
        }
        Ok(())
    }
}

#[path = "evidence_tests.rs"]
mod tests;
