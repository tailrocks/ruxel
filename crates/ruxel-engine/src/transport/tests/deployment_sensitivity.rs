use super::Fixture;

#[tokio::test]
async fn controller_rejects_missing_sensitivity_handshake_state() -> anyhow::Result<()> {
    let fixture = Fixture::new()?;
    let ack = ruxel_proto::v1::Event {
        msg: Some(ruxel_proto::v1::event::Msg::HelloAck(
            ruxel_proto::v1::HelloAck {
                proto_version: ruxel_proto::PROTO_VERSION,
                ..Default::default()
            },
        )),
    };
    std::fs::write(
        fixture.root.join("ack"),
        ruxel_proto::frame::encode_frame(&ack),
    )?;
    let error = tokio::time::timeout(std::time::Duration::from_secs(15), fixture.connect())
        .await?
        .unwrap_err();
    assert!(error.to_string().contains("sensitivity state"));
    Ok(())
}
