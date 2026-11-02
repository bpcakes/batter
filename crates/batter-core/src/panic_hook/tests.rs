use super::{LINE_CAPACITY, Site, emit};
use std::io::{self, Write};

/// Records every `write` call separately, so a test can count system writes.
#[derive(Default)]
struct Writes(Vec<Vec<u8>>);

impl Writes {
    fn joined(&self) -> String {
        String::from_utf8(self.0.concat()).unwrap()
    }
}

impl Write for Writes {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0.push(buf.to_vec());
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// Fails every write, as a closed or full descriptor would.
struct Refuses(usize);

impl Write for Refuses {
    fn write(&mut self, _: &[u8]) -> io::Result<usize> {
        self.0 += 1;
        Err(io::Error::from(io::ErrorKind::BrokenPipe))
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// Accepts nothing, so `write_all` reports `WriteZero`.
struct Full(usize);

impl Write for Full {
    fn write(&mut self, _: &[u8]) -> io::Result<usize> {
        self.0 += 1;
        Ok(0)
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[test]
fn a_located_panic_is_one_line_in_one_write() {
    let mut writes = Writes::default();
    emit(&mut writes, Some(Site::new("src/root.rs", 12, 5)));
    assert_eq!(
        writes.0,
        [b"\na thread panicked at src/root.rs:12:5; panic payload redacted\n".to_vec()]
    );
}

#[test]
fn a_panic_without_a_location_is_still_reported() {
    let mut writes = Writes::default();
    emit(&mut writes, None);
    assert_eq!(
        writes.0,
        [b"\na thread panicked at an unknown location; panic payload redacted\n".to_vec()]
    );
}

#[test]
fn a_line_longer_than_the_stack_buffer_is_written_whole_in_pieces() {
    let file = format!("src/{}.rs", "nested/".repeat(LINE_CAPACITY / 7 + 1));
    assert!(file.len() > LINE_CAPACITY);
    let mut writes = Writes::default();
    emit(&mut writes, Some(Site::new(&file, u32::MAX, u32::MAX)));
    assert_eq!(
        writes.joined(),
        format!(
            "\na thread panicked at {file}:{}:{}; panic payload redacted\n",
            u32::MAX,
            u32::MAX
        )
    );
    assert!(writes.0.len() > 1, "the fallback writes directly");
}

#[test]
fn the_longest_line_that_fits_still_uses_one_write() {
    let frame = "\na thread panicked at :1:1; panic payload redacted\n".len();
    let file = "f".repeat(LINE_CAPACITY - frame);
    let mut writes = Writes::default();
    emit(&mut writes, Some(Site::new(&file, 1, 1)));
    assert_eq!(writes.0.len(), 1);
    assert_eq!(writes.0[0].len(), LINE_CAPACITY);
}

#[test]
fn write_failures_are_ignored_without_retrying_or_panicking() {
    let site = Some(Site::new("src/root.rs", 1, 1));
    let mut refuses = Refuses(0);
    emit(&mut refuses, site);
    assert_eq!(refuses.0, 1);
    let mut full = Full(0);
    emit(&mut full, site);
    assert_eq!(full.0, 1);
}
