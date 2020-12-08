
use smol::LocalExecutor;
use std::sync::Arc;
use smol_quiche::{SmolQuic, SmolHttp3Client};

const MAX_DATAGRAM_SIZE: u64 = 1350;
fn main() {

    let scheduler = Arc::new(
        LocalExecutor::new()
    );


    smol::block_on(scheduler.run(main_async(&scheduler))).unwrap();
}

async fn main_async<'a>(scheduler: &LocalExecutor<'a>) -> quiche::Result<()> {
    let mut config = quiche::Config::new(quiche::PROTOCOL_VERSION)?;

    config
        .set_application_protos(quiche::h3::APPLICATION_PROTOCOL)
        .unwrap();

    config.set_max_idle_timeout(5000);
    config.set_max_udp_payload_size(MAX_DATAGRAM_SIZE);
    config.set_initial_max_data(10_000_000);
    config.set_initial_max_stream_data_bidi_local(1_000_000);
    config.set_initial_max_stream_data_bidi_remote(1_000_000);
    config.set_initial_max_stream_data_uni(1_000_000);
    config.set_initial_max_streams_bidi(100);
    config.set_initial_max_streams_uni(100);
    config.set_disable_active_migration(true);
    let quic: SmolQuic = SmolQuic::new(&mut config, &scheduler).await?;



    let http_client = SmolHttp3Client::new(quic).await;

    Ok(())
}