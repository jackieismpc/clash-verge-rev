use super::{Cancellation, Context, Controller, Duration, Result, Worker, bail, helper, json, parse_yaml};
use std::{collections::BTreeMap, path::PathBuf};
impl Worker {
    pub(super) fn helper_files(&self, yaml: &str) -> Result<BTreeMap<String, String>> {
        let document = parse_yaml(yaml)?;
        let mut files = BTreeMap::new();
        for category in ["proxy-providers", "rule-providers"] {
            if let Some(providers) = document.get(category).and_then(serde_yaml_ng::Value::as_mapping) {
                for (name, provider) in providers {
                    if provider["type"].as_str() == Some("file") {
                        let name = name.as_str().context("invalid provider name")?;
                        let path = PathBuf::from(provider["path"].as_str().context("file provider needs a path")?);
                        let path = if path.is_absolute() {
                            path
                        } else {
                            self.options.data_dir.join(path)
                        };
                        let path = path.canonicalize()?;
                        if !path.starts_with(self.options.data_dir.canonicalize()?) {
                            bail!("TUN file providers must be imported into the private data directory");
                        }
                        if std::fs::metadata(&path)?.len() > 1024 * 1024 {
                            bail!("TUN file provider exceeds 1 MiB");
                        }
                        files.insert(format!("{category}:{name}"), std::fs::read_to_string(path)?);
                    }
                }
            }
        }
        Ok(files)
    }
    pub(super) fn reload_helper(&self, yaml: &str) -> Result<()> {
        helper::request(
            &helper::HelperCommand::Reload {
                owner: self.network.owner().to_owned(),
                generation: self.generation(),
                runtime_yaml: yaml.to_owned(),
                provider_files: self.helper_files(yaml)?,
            },
            self.options.execution_mode == crate::config::ExecutionMode::IntegrationTest,
        )?;
        Ok(())
    }
    pub(super) async fn stop_helper(&mut self) -> Result<()> {
        if self.runtime_state["helperMode"] != true {
            return Ok(());
        }
        helper::request(
            &helper::HelperCommand::Stop {
                owner: self.network.owner().to_owned(),
                generation: self.generation(),
            },
            self.options.execution_mode == crate::config::ExecutionMode::IntegrationTest,
        )?;
        self.runtime_state["helperMode"] = false.into();
        self.controller = Controller::new(self.options.data_dir.join("c.sock"), self.secret.clone())?;
        self.controller.integration_test = self.options.execution_mode == crate::config::ExecutionMode::IntegrationTest;
        Ok(())
    }
    pub(super) async fn set_tun(&mut self, enabled: bool, id: &str, cancel: &Cancellation) -> Result<()> {
        if self.options.execution_mode == crate::config::ExecutionMode::IntegrationTest {
            bail!("integrationTest forbids TUN");
        }
        if enabled == (self.runtime_state["helperMode"] == true) {
            return Ok(());
        }
        if !enabled {
            let proxy_on = self.network.proxy_on;
            let dns_servers = self.network.dns_servers.clone();
            self.stop_core(id).await?;
            self.start_core(id, cancel).await?;
            self.restore_network_modes(proxy_on, dns_servers)?;
            self.publish_state(id);
            return Ok(());
        }
        if self.store.active_core.is_some() {
            bail!("restore the fixed bundled core before enabling privileged TUN");
        }
        let status = helper::request(&helper::HelperCommand::Status, false)?;
        if status.running && status.owner.as_deref() != Some(self.network.owner()) {
            bail!("native helper is owned by a different client");
        }
        let proxy_on = self.network.proxy_on;
        let dns_servers = self.network.dns_servers.clone();
        let previous_running = self.is_running();
        let yaml = self
            .store
            .owned_runtime_yaml(&self.controller.socket, &self.secret, &self.options)?;
        let provider_files = self.helper_files(&yaml)?;
        cancel.check()?;
        if self.is_running() {
            self.stop_core(id).await?;
        }
        let generation = self.generation().checked_add(1).context("generation exhausted")?;
        self.runtime_state["coreGeneration"] = generation.into();
        self.runtime_state["status"] = "starting".into();
        self.publish_state(id);
        let outcome = async {
            let status = helper::request(
                &helper::HelperCommand::Start {
                    owner: self.network.owner().to_owned(),
                    generation,
                    mixed_port: self.store.settings.mixed_port,
                    secret: self.secret.clone(),
                    runtime_yaml: yaml,
                    provider_files,
                },
                false,
            )?;
            self.runtime_state["helperMode"] = true.into();
            if !status.running
                || status.owner.as_deref() != Some(self.network.owner())
                || status.generation != Some(generation)
            {
                bail!("helper did not start the requested owned session");
            }
            let socket = PathBuf::from(status.private_socket.context("helper returned no controller")?);
            if !socket.starts_with(helper::HELPER_DATA)
                || socket.file_name().and_then(|s| s.to_str()) != Some("core.sock")
            {
                bail!("invalid private helper controller");
            }
            self.controller = Controller::new(socket, self.secret.clone())?;
            cancel
                .run(async {
                    tokio::time::timeout(Duration::from_secs(15), async {
                        loop {
                            if self.controller.request("GET", "/version", None).await.is_ok() {
                                break;
                            }
                            tokio::time::sleep(Duration::from_millis(100)).await;
                        }
                    })
                    .await
                    .context("TUN controller did not become ready")?;
                    Ok(())
                })
                .await?;
            self.runtime_state["status"] = "running".into();
            self.runtime_state["pid"] = json!(status.pid);
            self.runtime_state["socketPath"] = self.controller.socket.to_string_lossy().to_string().into();
            self.runtime_state
                .as_object_mut()
                .context("invalid runtime state")?
                .remove("error");
            self.restore_network_modes(proxy_on, dns_servers.clone())?;
            self.publish_state(id);
            self.restore_selections(id).await;
            Ok::<_, anyhow::Error>(())
        }
        .await;
        if let Err(error) = outcome {
            if self.runtime_state["helperMode"] != true
                && let Ok(status) = helper::request(&helper::HelperCommand::Status, false)
                && status.running
                && status.owner.as_deref() == Some(self.network.owner())
                && status.generation == Some(generation)
            {
                self.runtime_state["helperMode"] = true.into();
            }
            self.runtime_state["status"] = "failed".into();
            self.runtime_state["error"] = format!("{error:#}").into();
            if self.runtime_state["helperMode"] == true
                && let Err(stop_error) = self.stop_core(id).await
            {
                self.publish_state(id);
                return Err(error.context(format!("TUN cleanup requires recovery: {stop_error:#}")));
            }
            if previous_running {
                self.start_core(id, &Cancellation::default())
                    .await
                    .context(format!("TUN failed: {error:#}; prior core could not restart"))?;
                self.restore_network_modes(proxy_on, dns_servers)?;
            }
            self.publish_state(id);
            return Err(error);
        }
        Ok(())
    }
}
