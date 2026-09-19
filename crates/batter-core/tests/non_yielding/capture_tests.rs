use super::*;
use std::sync::mpsc;

const EVIDENCE: &[u8] = b"batter-fixture:evidence-ready\n";

enum End {
    Eof,
    Error,
    Panic,
}

struct ScriptedReader {
    reads: usize,
    end: End,
    dropped: mpsc::Sender<()>,
}

impl Read for ScriptedReader {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        self.reads += 1;
        match self.reads {
            1 => Err(io::ErrorKind::Interrupted.into()),
            2 => {
                buffer[..EVIDENCE.len()].copy_from_slice(EVIDENCE);
                Ok(EVIDENCE.len())
            }
            _ => match self.end {
                End::Eof => Ok(0),
                End::Error => Err(io::Error::new(
                    io::ErrorKind::BrokenPipe,
                    "deliberate capture read failure",
                )),
                End::Panic => panic!("deliberate capture reader panic"),
            },
        }
    }
}

impl Drop for ScriptedReader {
    fn drop(&mut self) {
        let _ = self.dropped.send(());
    }
}

fn capture(end: End) -> (Capture, mpsc::Receiver<()>) {
    let (dropped, receiver) = mpsc::channel();
    let capture = Capture::start(ScriptedReader {
        reads: 0,
        end,
        dropped,
    })
    .unwrap();
    (capture, receiver)
}

#[test]
fn interrupted_reads_resume_and_eof_joins_the_reader() {
    let (mut capture, dropped) = capture(End::Eof);
    let output = capture.finish().unwrap();
    output.validate(&["evidence-ready"], &[]).unwrap();
    assert_eq!(output.text().as_bytes(), EVIDENCE);
    dropped
        .try_recv()
        .expect("finish must join reader destruction");
}

#[test]
fn reader_io_error_is_preserved_after_partial_output_and_join() {
    let (mut capture, dropped) = capture(End::Error);
    let error = capture.finish().unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::BrokenPipe);
    assert_eq!(error.to_string(), "deliberate capture read failure");
    assert_eq!(capture.snapshot().text().as_bytes(), EVIDENCE);
    dropped
        .try_recv()
        .expect("finish must join reader destruction");
    drop(capture);
}

#[test]
fn reader_panic_becomes_an_io_error_after_partial_output_and_join() {
    let (mut capture, dropped) = capture(End::Panic);
    let error = capture.finish().unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::Other);
    assert_eq!(error.to_string(), "fixture output reader panicked");
    assert_eq!(capture.snapshot().text().as_bytes(), EVIDENCE);
    dropped
        .try_recv()
        .expect("finish must join reader destruction");
    drop(capture);
}
