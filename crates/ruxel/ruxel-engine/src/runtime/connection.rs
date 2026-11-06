use crate::transport::{AgentConnection, LocalConnection};
use anyhow::Result;
use ruxel_proto::v1;

pub(super) enum Connection {
    Local(Box<LocalConnection>),
    Remote(Box<AgentConnection>),
}
impl Connection {
    pub(super) async fn send(&mut self, message: &v1::Envelope) -> Result<()> {
        match self {
            Self::Local(c) => c.send(message).await,
            Self::Remote(c) => c.send(message).await,
        }
    }
    pub(super) async fn next(&mut self) -> Result<Option<v1::Event>> {
        match self {
            Self::Local(c) => c.next_event().await,
            Self::Remote(c) => c.next_event().await,
        }
    }
    pub(super) async fn shutdown(self) -> Result<()> {
        match self {
            Self::Local(c) => c.shutdown().await,
            Self::Remote(c) => c.shutdown().await,
        }
    }
}
