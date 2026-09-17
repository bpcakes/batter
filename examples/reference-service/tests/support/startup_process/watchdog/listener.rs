use super::*;
use std::{
    os::unix::{fs::DirBuilderExt, net::UnixDatagram},
    path::PathBuf,
};

/// Own the receiver before spawn; drop removes only this private fixture path.
pub(super) struct Announcement {
    socket: UnixDatagram,
    directory: PathBuf,
}

impl Announcement {
    pub(super) fn new(command: &mut Command) -> io::Result<Self> {
        let directory = PathBuf::from(format!("/tmp/batter-listener-{}", uuid::Uuid::now_v7()));
        std::fs::DirBuilder::new().mode(0o700).create(&directory)?;
        let path = directory.join("socket");
        let socket = match UnixDatagram::bind(&path) {
            Ok(socket) => socket,
            Err(error) => {
                let _ = std::fs::remove_dir(&directory);
                return Err(error);
            }
        };
        let receiver = Self { socket, directory };
        receiver.socket.set_nonblocking(true)?;
        command.env("BATTER_LISTENER_ANNOUNCEMENT_PATH", path);
        Ok(receiver)
    }

    fn reported(&self) -> Result<Option<SocketAddr>, String> {
        let mut message = [0; 128];
        let length = match self.socket.recv(&mut message) {
            Ok(length) => length,
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => return Ok(None),
            Err(_) => return Err("listener announcement receive failed".into()),
        };
        parse(&message[..length]).map(Some)
    }
}

impl Drop for Announcement {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(self.directory.join("socket"));
        let _ = std::fs::remove_dir(&self.directory);
    }
}

pub(super) fn parse(message: &[u8]) -> Result<SocketAddr, String> {
    let address: SocketAddr = std::str::from_utf8(message)
        .ok()
        .and_then(|text| text.parse().ok())
        .ok_or_else(|| "production child reported an invalid listener address".to_owned())?;
    if !address.ip().is_loopback() || address.port() == 0 {
        return Err("production child reported an unusable loopback listener".into());
    }
    Ok(address)
}

impl SeparateChild {
    pub(super) fn wait_for_listener(
        &mut self,
        announcement: &Announcement,
        limit: Duration,
    ) -> Result<SocketAddr, String> {
        let deadline = Instant::now() + limit;
        loop {
            self.stderr
                .inspect(|output, _| safe_check(output, "child stderr"))?;
            self.stdout
                .inspect(|output, _| safe_check(output, "child stdout"))?;
            if let Some(address) = announcement.reported()? {
                return Ok(address);
            }
            if self
                .child
                .try_wait()
                .map_err(|error| error.to_string())?
                .is_some()
            {
                return Err(format!(
                    "{} exited before reporting its listener; {}",
                    self.label,
                    self.diagnostics()
                ));
            }
            if Instant::now() >= deadline {
                return Err(format!(
                    "{} did not report its listener before the deadline; {}",
                    self.label,
                    self.diagnostics()
                ));
            }
            std::thread::sleep(POLL);
        }
    }
}

impl SeparateRun {
    pub(super) fn production_running_child(&self, forbidden: &[&str]) -> Result<(), String> {
        self.check(forbidden)?;
        if !self.status.success() {
            return Err(format!(
                "running production executable did not exit successfully: {self:?}"
            ));
        }
        if !self.stdout.text().is_empty() || !self.stderr.text().is_empty() {
            return Err(format!(
                "running production executable emitted diagnostics: {self:?}"
            ));
        }
        Ok(())
    }

    pub(super) fn production_crash(&self, forbidden: &[&str]) -> Result<(), String> {
        self.check(forbidden)?;
        if self.status.signal() != Some(9) {
            return Err(format!(
                "production executable did not exit from SIGKILL: {self:?}"
            ));
        }
        if !self.stdout.text().is_empty() || !self.stderr.text().is_empty() {
            return Err(format!(
                "crashed production executable emitted diagnostics: {self:?}"
            ));
        }
        Ok(())
    }
}
