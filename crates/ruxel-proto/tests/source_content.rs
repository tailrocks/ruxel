use prost::Message;
use ruxel_proto::v1;

#[test]
fn source_bytes_roundtrip_preserves_empty_and_binary() {
    for source_content in [None, Some(Vec::new()), Some(vec![0xff, 0x80, 0x00])] {
        let iteration = v1::Iteration {
            source_content,
            ..Default::default()
        };
        let encoded = iteration.encode_to_vec();
        let decoded = v1::Iteration::decode(encoded.as_slice()).expect("iteration decodes");
        assert_eq!(decoded, iteration);
    }
}
