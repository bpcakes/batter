use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Duration;

use bollard::auth::DockerCredentials;
use bollard::errors::Error;
use bollard::models::{ContainerCreateBody, HostConfig, PortBinding};
use bollard::query_parameters::{CreateImageOptions, RemoveContainerOptions};
use bollard::{API_DEFAULT_VERSION, Docker};
use futures_util::TryStreamExt;
use tokio::sync::oneshot;

use crate::container_lifecycle::PROCESS_OWNER_LABEL;
use crate::postgres_container::{POSTGRES_DB, POSTGRES_PASSWORD, POSTGRES_USER};

pub(crate) const CLEANUP_TIMEOUT: Duration = Duration::from_secs(30);

pub(crate) async fn docker_client_instance() -> Result<Docker, Error> {
    // Preserve the Unix socket discovery used by the former Testcontainers
    // fixture, including Docker Desktop. An explicit DOCKER_HOST wins.
    let host = std::env::var("DOCKER_HOST").unwrap_or_else(|_| {
        let mut candidates = vec![PathBuf::from("/var/run/docker.sock")];
        if let Some(runtime) = std::env::var_os("XDG_RUNTIME_DIR") {
            candidates.push(PathBuf::from(runtime).join(".docker/run/docker.sock"));
        }
        if let Some(home) = std::env::var_os("HOME") {
            let directory = PathBuf::from(home);
            candidates.push(directory.join(".docker/run/docker.sock"));
            candidates.push(directory.join(".docker/desktop/docker.sock"));
        }
        candidates
            .into_iter()
            .find(|path| path.exists())
            .map_or_else(
                || "unix:///var/run/docker.sock".to_owned(),
                |path| format!("unix://{}", path.display()),
            )
    });
    let tls_verify = std::env::var("DOCKER_TLS_VERIFY").ok();
    connect_client(&host, tls_verify.as_deref())?
        .negotiate_version()
        .await
}

fn connect_client(host: &str, tls_verify: Option<&str>) -> Result<Docker, Error> {
    // Testcontainers interpreted only "1" as enabled. Bollard's default
    // connector interprets even "0" or an empty variable as enabling TLS.
    if (host.starts_with("tcp://") || host.starts_with("http://")) && tls_verify != Some("1") {
        Docker::connect_with_http(host, 120, API_DEFAULT_VERSION)
    } else {
        Docker::connect_with_host(host)
    }
}

pub(crate) struct PostgresImage {
    reference: String,
    pub(crate) command: Option<Vec<String>>,
    pub(crate) entrypoint: Option<Vec<String>>,
}

impl PostgresImage {
    pub(crate) fn new(repository: String, tag: String) -> Self {
        Self {
            reference: format!("{repository}:{tag}"),
            command: None,
            entrypoint: None,
        }
    }

    pub(crate) async fn pull_if_missing(&self, docker: &Docker) -> Result<(), String> {
        match docker.inspect_image(&self.reference).await {
            Ok(_) => return Ok(()),
            Err(error) if is_not_found(&error) => {}
            Err(error) => return Err(error.to_string()),
        }
        let credentials = image_credentials(&self.reference).await;
        let options = CreateImageOptions {
            from_image: Some(self.reference.clone()),
            platform: configured_platform(),
            ..Default::default()
        };
        let mut progress = docker.create_image(Some(options), None, credentials);
        while let Some(update) = progress
            .try_next()
            .await
            .map_err(|error| error.to_string())?
        {
            if let Some(error) = update.error_detail.and_then(|error| error.message) {
                return Err(error);
            }
        }
        Ok(())
    }

    pub(crate) fn configuration(&self) -> ContainerCreateBody {
        ContainerCreateBody {
            image: Some(self.reference.clone()),
            env: Some(vec![
                format!("POSTGRES_USER={POSTGRES_USER}"),
                format!("POSTGRES_PASSWORD={POSTGRES_PASSWORD}"),
                format!("POSTGRES_DB={POSTGRES_DB}"),
            ]),
            cmd: self.command.clone(),
            entrypoint: self.entrypoint.clone(),
            exposed_ports: Some(vec!["5432/tcp".to_owned()]),
            labels: Some(HashMap::from([(
                PROCESS_OWNER_LABEL.to_owned(),
                crate::container_lifecycle::process_owner_label_value(),
            )])),
            host_config: Some(HostConfig {
                publish_all_ports: Some(false),
                port_bindings: Some(HashMap::from([(
                    "5432/tcp".to_owned(),
                    Some(vec![PortBinding {
                        host_ip: Some("127.0.0.1".to_owned()),
                        host_port: Some(String::new()),
                    }]),
                )])),
                ..Default::default()
            }),
            ..Default::default()
        }
    }
}

pub(crate) fn configured_platform() -> String {
    std::env::var("DOCKER_DEFAULT_PLATFORM")
        .ok()
        .filter(|platform| !platform.trim().is_empty())
        .unwrap_or_default()
}

async fn image_credentials(reference: &str) -> Option<DockerCredentials> {
    let first = reference.split('/').next()?;
    let registry = if reference.contains('/')
        && (first.contains('.') || first.contains(':') || first == "localhost")
    {
        first.to_owned()
    } else {
        "https://index.docker.io/v1/".to_owned()
    };
    tokio::task::spawn_blocking(move || {
        let credential = match std::env::var("DOCKER_AUTH_CONFIG") {
            Ok(config) => {
                docker_credential::get_credential_from_reader(config.as_bytes(), &registry)
            }
            Err(_) => docker_credential::get_credential(&registry),
        }
        .ok()?;
        Some(match credential {
            docker_credential::DockerCredential::IdentityToken(token) => DockerCredentials {
                identitytoken: Some(token),
                serveraddress: Some(registry),
                ..Default::default()
            },
            docker_credential::DockerCredential::UsernamePassword(username, password) => {
                DockerCredentials {
                    username: Some(username),
                    password: Some(password),
                    serveraddress: Some(registry),
                    ..Default::default()
                }
            }
        })
    })
    .await
    .ok()
    .flatten()
}

// A dropped handle releases the cleanup owner, including when delivery to a
// cancelled startup waiter fails. Cleanup requires the runtime to remain alive;
// the separate process reaper covers process exit when its CLI is available.
#[derive(Debug)]
pub(crate) struct DockerContainer {
    docker: Docker,
    id: String,
    _cleanup: Option<oneshot::Sender<oneshot::Sender<Result<(), String>>>>,
}

impl DockerContainer {
    pub(crate) fn new(docker: Docker, id: String, remove: bool) -> Self {
        let (cleanup, released) = oneshot::channel::<oneshot::Sender<Result<(), String>>>();
        let cleanup_docker = docker.clone();
        let cleanup_id = id.clone();
        tokio::spawn(async move {
            let reply = released.await;
            if remove || reply.is_ok() {
                let result = remove_container(&cleanup_docker, &cleanup_id).await;
                if let Ok(reply) = reply {
                    let _ = reply.send(result);
                } else if let Err(error) = result {
                    eprintln!("PostgreSQL fixture cleanup failed: {error}");
                }
            }
        });
        Self {
            docker,
            id,
            _cleanup: Some(cleanup),
        }
    }

    pub(crate) fn id(&self) -> &str {
        &self.id
    }

    #[cfg(test)]
    pub(crate) fn docker(&self) -> &Docker {
        &self.docker
    }

    #[cfg(test)]
    pub(crate) async fn rm(mut self) -> Result<(), String> {
        let (reply, result) = oneshot::channel();
        self._cleanup
            .take()
            .expect("container cleanup owner")
            .send(reply)
            .map_err(|_| "container cleanup owner stopped".to_owned())?;
        result
            .await
            .map_err(|_| "container cleanup result lost".to_owned())?
    }

    pub(crate) async fn get_host_port_ipv4(&self, internal_port: u16) -> Result<u16, String> {
        self.docker
            .inspect_container(&self.id, None)
            .await
            .map_err(|error| error.to_string())?
            .network_settings
            .and_then(|network| network.ports)
            .and_then(|mut ports| ports.remove(&format!("{internal_port}/tcp")))
            .flatten()
            .into_iter()
            .flatten()
            .find(|binding| binding.host_ip.as_deref() == Some("127.0.0.1"))
            .and_then(|binding| binding.host_port?.parse().ok())
            .ok_or_else(|| "Docker has not published the IPv4 loopback port".to_owned())
    }
}

pub(crate) async fn remove_container(docker: &Docker, id: &str) -> Result<(), String> {
    let remove = docker.remove_container(
        id,
        Some(RemoveContainerOptions {
            force: true,
            v: true,
            ..Default::default()
        }),
    );
    match tokio::time::timeout(CLEANUP_TIMEOUT, remove).await {
        Ok(Ok(())) => Ok(()),
        Ok(Err(error)) if is_not_found(&error) => Ok(()),
        Ok(Err(error)) => Err(error.to_string()),
        Err(_) => Err(format!("container removal exceeded {CLEANUP_TIMEOUT:?}")),
    }
}

pub(crate) fn is_not_found(error: &Error) -> bool {
    matches!(
        error,
        Error::DockerResponseServerError {
            status_code: 404,
            ..
        }
    )
}

#[cfg(test)]
mod tests {
    use std::process::Command;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    use super::*;

    #[test]
    fn disabled_tls_settings_negotiate_over_plain_http() {
        let output = Command::new(std::env::current_exe().expect("test executable"))
            .args(["--exact", "docker::tests::plain_http_child", "--ignored", "--nocapture"])
            // A regression delegating to Bollard's default connector must see
            // a set variable and attempt TLS, rather than pass by accident.
            .env("DOCKER_TLS_VERIFY", "0")
            .output().expect("launch HTTP Docker fixture control");
        assert!(
            output.status.success(),
            "disabled TLS control: {}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }

    #[tokio::test]
    #[ignore = "helper entrypoint launched explicitly by the disabled TLS control"]
    async fn plain_http_child() {
        for tls_verify in [None, Some(""), Some("0")] {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
                .await
                .expect("HTTP Docker fixture");
            let host = format!("tcp://{}", listener.local_addr().expect("fixture address"));
            let client = connect_client(&host, tls_verify).expect("plaintext Docker client");
            let fixture = async {
                let (mut socket, _) = listener.accept().await.expect("accept Docker negotiation");
                let mut request = Vec::new();
                while !request.ends_with(b"\r\n\r\n") {
                    request.push(socket.read_u8().await.expect("read HTTP request"));
                }
                assert!(request.starts_with(b"GET /version HTTP/1.1\r\n"));
                let body = r#"{"ApiVersion":"1.53"}"#;
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                socket
                    .write_all(response.as_bytes())
                    .await
                    .expect("Docker version response");
            };
            tokio::time::timeout(Duration::from_secs(2), async {
                let (client, ()) = tokio::join!(client.negotiate_version(), fixture);
                client.expect("disabled TLS must permit HTTP version negotiation");
            })
            .await
            .expect("HTTP negotiation deadline");
        }
    }
}
