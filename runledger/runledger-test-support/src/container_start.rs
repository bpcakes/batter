use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use testcontainers::bollard::{Docker, errors::Error, query_parameters::RemoveContainerOptions};
use testcontainers::core::{client::docker_client_instance, error::TestcontainersError};
use testcontainers::runners::AsyncRunner;
use testcontainers::{ContainerAsync, ContainerRequest, GenericImage, ImageExt};

use crate::container_lifecycle::should_remove_container;

const ATTEMPT_LABEL: &str = "org.runledger.test-start";
const CLEANUP_TIMEOUT: Duration = Duration::from_secs(30);
static ATTEMPT_COUNTER: AtomicU64 = AtomicU64::new(0);

pub(crate) async fn start_postgres(
    image: ContainerRequest<GenericImage>,
) -> ContainerAsync<GenericImage> {
    let image = image.with_host_config_modifier(|config| {
        // The fixture URL uses IPv4 loopback. Ask Docker to allocate and bind
        // there, avoiding an all-interface bind that can collide with unrelated
        // outbound host sockets on Docker Desktop. No local port reservation.
        config.publish_all_ports = Some(false);
        config.port_bindings.get_or_insert_default().insert(
            "5432/tcp".to_owned(),
            Some(vec![testcontainers::bollard::models::PortBinding {
                host_ip: Some("127.0.0.1".to_owned()),
                host_port: Some(String::new()),
            }]),
        );
    });
    let attempt = StartAttempt::new();
    let remove = should_remove_container();
    // Keep start/error cleanup alive if a OnceCell initializer's waiter is
    // cancelled. This requires the runtime to remain alive, like native cleanup.
    tokio::spawn(async move { attempt.start(image, remove).await })
        .await
        .expect("join PostgreSQL fixture startup")
        .unwrap_or_else(|error| panic!("{error}"))
}

struct StartAttempt {
    name: String,
    token: String,
    #[cfg(test)]
    cleanup_gate: Option<std::sync::Arc<tests::CleanupGate>>,
}

impl StartAttempt {
    fn new() -> Self {
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock must follow Unix epoch")
            .as_nanos();
        let counter = ATTEMPT_COUNTER.fetch_add(1, Ordering::Relaxed);
        let token = format!("{}-{timestamp}-{counter}", std::process::id());
        Self {
            name: format!("runledger-postgres-{token}"),
            token,
            #[cfg(test)]
            cleanup_gate: tests::CLEANUP_GATE.try_with(std::sync::Arc::clone).ok(),
        }
    }

    async fn start(
        self,
        image: ContainerRequest<GenericImage>,
        remove: bool,
    ) -> Result<ContainerAsync<GenericImage>, String> {
        // Use the same endpoint/configuration as testcontainers, without a CLI
        // prerequisite. Get it before any request can create a container.
        let docker = docker_client_instance()
            .await
            .map_err(|error| error.to_string())?;
        // Docker allocates the published port; no host socket is released ahead
        // of its create/pull/start sequence. Allocation can still fail at Docker.
        let result = image
            .with_container_name(&self.name)
            .with_label(ATTEMPT_LABEL, &self.token)
            .start()
            .await;
        match result {
            Ok(container) => Ok(container),
            Err(error) => Err(self.finish_failure(&docker, error, remove).await),
        }
    }

    async fn finish_failure(
        &self,
        docker: &Docker,
        error: TestcontainersError,
        remove: bool,
    ) -> String {
        #[cfg(test)]
        if let Some(gate) = &self.cleanup_gate {
            gate.pause(&self.name).await;
        }
        let cleanup = if remove {
            match tokio::time::timeout(CLEANUP_TIMEOUT, self.remove_owned(docker)).await {
                Ok(Ok(())) => "failed-start cleanup completed".to_owned(),
                Ok(Err(error)) => format!("failed-start cleanup also failed: {error}"),
                Err(_) => format!("failed-start cleanup exceeded {CLEANUP_TIMEOUT:?}"),
            }
        } else {
            "container retained by TESTCONTAINERS_COMMAND=keep".to_owned()
        };
        format!(
            "start PostgreSQL fixture {} failed: {error}; {cleanup}",
            self.name
        )
    }

    async fn remove_owned(&self, docker: &Docker) -> Result<(), String> {
        let container = match docker.inspect_container(&self.name, None).await {
            Ok(container) => container,
            Err(error) if is_not_found(&error) => return Ok(()),
            Err(error) => return Err(error.to_string()),
        };
        let label = container
            .config
            .as_ref()
            .and_then(|config| config.labels.as_ref())
            .and_then(|labels| labels.get(ATTEMPT_LABEL));
        if label != Some(&self.token) {
            return Err(
                "container name belongs to a different startup attempt; left untouched".into(),
            );
        }
        let id = container
            .id
            .ok_or("Docker inspection omitted container ID")?;
        // Delete by immutable ID after verifying the label, never by the name
        // which could be reused. `v` removes anonymous volumes from failed starts.
        match docker
            .remove_container(
                &id,
                Some(RemoveContainerOptions {
                    force: true,
                    v: true,
                    ..Default::default()
                }),
            )
            .await
        {
            Ok(()) => Ok(()),
            Err(error) if is_not_found(&error) => Ok(()),
            Err(error) => Err(error.to_string()),
        }
    }
}

fn is_not_found(error: &Error) -> bool {
    matches!(
        error,
        Error::DockerResponseServerError {
            status_code: 404,
            ..
        }
    )
}

#[cfg(test)]
mod tests;
