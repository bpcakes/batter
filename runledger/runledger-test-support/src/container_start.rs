use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use bollard::{
    Docker,
    query_parameters::{CreateContainerOptions, RemoveContainerOptions},
};

use crate::container_lifecycle::should_remove_container;
use crate::docker::{
    CLEANUP_TIMEOUT, DockerContainer, PostgresImage, configured_platform, docker_client_instance,
    is_not_found,
};

const ATTEMPT_LABEL: &str = "org.runledger.test-start";
static ATTEMPT_COUNTER: AtomicU64 = AtomicU64::new(0);

pub(crate) async fn start_postgres(image: PostgresImage) -> DockerContainer {
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

    async fn start(self, image: PostgresImage, remove: bool) -> Result<DockerContainer, String> {
        // Get the configured API client before any request can create a container.
        let docker = docker_client_instance()
            .await
            .map_err(|error| error.to_string())?;
        // Docker allocates the published port; no host socket is released ahead
        // of its create/pull/start sequence. Allocation can still fail at Docker.
        let result = self.create_and_start(&docker, image).await;
        match result {
            Ok(id) => {
                let container = DockerContainer::new(docker, id, remove);
                #[cfg(test)]
                if let Some(gate) = &self.cleanup_gate {
                    gate.pause(&self.name).await;
                }
                Ok(container)
            }
            Err(error) => Err(self.finish_failure(&docker, error, remove).await),
        }
    }

    async fn create_and_start(
        &self,
        docker: &Docker,
        image: PostgresImage,
    ) -> Result<String, String> {
        image.pull_if_missing(docker).await?;
        let mut config = image.configuration();
        config
            .labels
            .get_or_insert_default()
            .insert(ATTEMPT_LABEL.to_owned(), self.token.clone());
        let created = docker
            .create_container(
                Some(CreateContainerOptions {
                    name: Some(self.name.clone()),
                    platform: configured_platform(),
                }),
                config,
            )
            .await
            .map_err(|error| error.to_string())?;
        docker
            .start_container(&created.id, None)
            .await
            .map_err(|error| error.to_string())?;
        Ok(created.id)
    }

    async fn finish_failure(&self, docker: &Docker, error: String, remove: bool) -> String {
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

#[cfg(test)]
mod tests;
