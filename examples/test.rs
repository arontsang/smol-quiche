use std::time::Instant;
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



    if let Ok(mut http_client) = SmolHttp3Client::new(&quic, &scheduler).await {
        let req = vec![
            quiche::h3::Header::new(":method", "GET"),
            quiche::h3::Header::new(":scheme", "https"),
            quiche::h3::Header::new(":authority", "dns.google"),
            quiche::h3::Header::new(":path", "/"),
            quiche::h3::Header::new("user-agent", "quiche"),
        ];
    
        let start = Instant::now();
        let request = http_client.send_request(&req).await.unwrap();

        scheduler.run(async{

            loop {
                let foo = request.wait_events().await;

                match foo {
                    Err(_) => panic!(""),
                    Ok(quiche::h3::Event::Finished) => break,
                    Ok(quiche::h3::Event::Headers {list: headers, has_body: _}) => {
                        for header in headers {
                            let name = quiche::h3::NameValue::name(&header);
                            let value = quiche::h3::NameValue::value(&header);
                            println!("{} : {}", name, value);
                        }
                    },
                    Ok(quiche::h3::Event::Data) => {
                        let mut buffer: [u8; 3000] = [0; 3000];
                        match request.read_body(&mut buffer).await {
                            Ok(length) => {
                                let body = std::str::from_utf8(&buffer[0..length]).unwrap();
                                println!("{}", body);
                                let duration = start.elapsed();
                                println!("Time elapsed in expensive_function() is: {:?}", duration);
                            },
                            Err(quiche::h3::Error::Done) => (),
                            Err(er) =>
                                println!("{}", er)
                        };
                    }
                    Ok(quiche::h3::Event::Datagram) => panic!("Datagram not supported."),
                    Ok(quiche::h3::Event::GoAway) => break,
                }

                
            }
        }).await;
        std::mem::drop(http_client);
        std::mem::drop(quic);
    }



    Ok(())
}