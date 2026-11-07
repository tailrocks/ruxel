use super::{Execution, Mode, transport};
use crate::transport::{AgentConnection, LocalConnection};
use anyhow::{Context, Result};
use ruxel_proto::v1;
use serde_json::Value;
use std::sync::Arc;

pub(super) enum Connection {
    Local(Box<LocalConnection>),
    Remote(Box<AgentConnection>),
    #[cfg(test)]
    Test(super::effect_tests::TestConnection),
}
impl Connection {
    pub(super) async fn send(&mut self, message: &v1::Envelope) -> Result<()> {
        match self {
            Self::Local(c) => c.send(message).await,
            Self::Remote(c) => c.send(message).await,
            #[cfg(test)]
            Self::Test(c) => c.send(message),
        }
    }
    pub(super) async fn next(&mut self) -> Result<Option<v1::Event>> {
        match self {
            Self::Local(c) => c.next_event().await,
            Self::Remote(c) => c.next_event().await,
            #[cfg(test)]
            Self::Test(c) => c.next().await,
        }
    }
    pub(super) async fn shutdown(self) -> Result<()> {
        match self {
            Self::Local(c) => c.shutdown().await,
            Self::Remote(c) => c.shutdown().await,
            #[cfg(test)]
            Self::Test(_) => Ok(()),
        }
    }
}
impl Execution {
    #[expect(
        clippy::too_many_lines,
        reason = "connection setup validates and records handshake state"
    )]
    pub(super) async fn connect(&self, host: &str) -> Result<v1::Facts> {
        let _gate = self.source_gate.write().await;
        let mut connections = self.connections.lock().await;
        if connections.contains_key(host) {
            return Ok(v1::Facts::default());
        }
        let spec = self.inventory.get(host);
        let controller_local = host == "localhost" && spec.is_none();
        anyhow::ensure!(
            spec.is_some() || controller_local,
            "selected host is missing"
        );
        let check = matches!(self.mode, Mode::Check | Mode::Diff);
        let diff = self.mode == Mode::Diff;
        let (connection, gathered, sensitive) =
            if self.runtime.local_state.is_some() || controller_local {
                let state = self
                    .runtime
                    .local_state
                    .as_ref()
                    .or(self.runtime.controller_state.as_ref())
                    .context("localhost delegation requires --controller-state-dir")?;
                let binary = if self.runtime.local_state.is_some() {
                    &self.runtime.agent
                } else {
                    self.runtime
                        .local_agent
                        .as_ref()
                        .context("localhost delegation requires --local-agent")?
                };
                let path = state.join(blake3::hash(host.as_bytes()).to_hex().as_str());
                let (c, f, sensitive) = transport::LocalConnection::connect(
                    binary,
                    &path,
                    "pkl-local",
                    check,
                    diff,
                    self.runtime.no_cache,
                )
                .await?;
                (Connection::Local(Box::new(c)), f, sensitive)
            } else {
                let spec = spec.context("selected host is missing")?;
                let address = spec
                    .get("address")
                    .and_then(Value::as_str)
                    .context("host address is missing")?;
                let destination = match spec.get("user").and_then(Value::as_str) {
                    Some(user) => format!("{user}@{address}"),
                    None => address.to_owned(),
                };
                let port = spec
                    .get("port")
                    .and_then(Value::as_u64)
                    .map(u16::try_from)
                    .transpose()?;
                let transport_options = transport::ConnectOptions {
                    diff_mode: diff,
                    no_cache: self.runtime.no_cache,
                    port,
                    ..self.runtime.connection.clone()
                };
                let (c, f) = transport::connect_with(
                    &destination,
                    &self.runtime.agent,
                    "pkl",
                    check,
                    &transport_options,
                )
                .await?;
                (
                    Connection::Remote(Box::new(c)),
                    f.facts,
                    f.sensitive_effects,
                )
            };
        if sensitive {
            self.conservative_sensitive
                .store(true, std::sync::atomic::Ordering::Release);
            let tracker = self
                .resources
                .lock()
                .await
                .entry(host.to_owned())
                .or_default()
                .clone();
            tracker.lock().await.mark_unknown();
        }
        connections.insert(
            host.to_owned(),
            Arc::new(tokio::sync::Mutex::new(connection)),
        );
        Ok(gathered)
    }
}
