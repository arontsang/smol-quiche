
use async_net::UdpSocket;
use smol::lock::Mutex;
use smol::LocalExecutor;
use std::collections::HashMap;
use std::pin::Pin;
use std::rc::Rc;
use std::net::{IpAddr, Ipv4Addr};

mod scheduler;

type RcLock<T> = Rc::<Mutex::<T>>;
type PinnedQuicConnection = Pin::<Box::<quiche::Connection>>;

pub struct SmolQuic {
    connection: RcLock::<PinnedQuicConnection>,
    dispatch_table: Rc::<Mutex::<HashMap::<u64, scheduler::RaiseEvent>>>,
    _task: smol::Task::<()>,
}

impl SmolQuic {

    pub async fn new(config: &mut quiche::Config, scheduler: &LocalExecutor<'_>) -> Result<SmolQuic, quiche::Error> {
        
        let destination: &str = "dns.google";
        let destination: Option<&str> = Some(destination);
        let scid = [0xba; quiche::MAX_CONN_ID_LEN];

        let connection = quiche::connect(destination, &scid, config)?;
        let connection = Mutex::new(connection);
        let connection = Rc::new(connection);

        let (send_packet_event_sender, send_packet_event_receiver) = scheduler::auto_reset_event();
        let (timeout_event_sender, timeout_event) = scheduler::auto_reset_event();
        let (received_packet_event_sender, received_packet_event_receiver) = scheduler::auto_reset_event();

        let dispatch_table = Rc::new(Mutex::new(HashMap::<u64, scheduler::RaiseEvent>::new()));

        let task = {    
            let dispatch_table = dispatch_table.clone();        
            let connection = connection.clone();
            scheduler.spawn(async move {
                let socket = UdpSocket::bind("0.0.0.0:0").await;
                let socket: UdpSocket = socket.unwrap();
                let priority = scheduler::PriorityExecutor::new();
                let _timeout_loop = {
                    let connection = connection.clone();
                    let send_packet_event_sender = send_packet_event_sender.clone();
                    priority.spawn(scheduler::Priority::Low, async move {
                        {
                            let mut connection = connection.lock().await;
                            connection.on_timeout();
                            send_packet_event_sender.Reset();
                        }
                        loop {
                            let next_timeout = {
                                let connection = connection.lock().await;
                                connection.timeout()
                            };
                            match next_timeout {
                                Some(timeout) => {
                                    smol::Timer::after(timeout).await;                                
                                    {
                                        let mut connection = connection.lock().await;
                                        connection.on_timeout();
                                    }
                                    send_packet_event_sender.Reset();
                                },
                                None => {
                                    timeout_event.WaitOnce().await;
                                }
                            }
                        }
                    })
                };

                let _recv_loop = {
                    let socket = socket.clone();
                    let connection = connection.clone();
                    priority.spawn(scheduler::Priority::High, async move {
                        let mut recv_buffer: [u8; 1500] = [0; 1500];
                        let received_packet_event_sender = received_packet_event_sender.clone();
                        loop {
                            let (length, _address) = match socket.recv_from(&mut recv_buffer).await {
                                Err(_) => continue,
                                Ok(res) => res
                            };
                            {
                                let mut connection = connection.lock().await;
                                connection.recv(&mut recv_buffer[0..length]).unwrap();
                            }
                            received_packet_event_sender.Reset();
                        }
                    })
                };

                let _send_loop = {
                    let socket = socket.clone();
                    let connection = connection.clone();
                    priority.spawn(scheduler::Priority::Medium, async move {
                        let destination = async_std::net::SocketAddr::new(IpAddr::V4(Ipv4Addr::new(8, 8, 8, 8)), 443);
        
                        let mut send_buffer: [u8; 1500] = [0; 1500];
                        loop {                            
                            loop {
                                let mut connection = connection.lock().await;
                                match connection.send(&mut send_buffer) {
                                    Ok(length) => {
                                        socket.send_to(&send_buffer[0..length], destination).await
                                    },
                                    Err(quiche::Error::Done) => break,                               
                                    Err(_) => break,
                                }.unwrap();
        
                            }
                            timeout_event_sender.Reset();
                            send_packet_event_receiver.WaitOnce().await;
                        }
                    })
                };

                let _dispatch_receiver_task = {
                    let dispatch_table = dispatch_table.clone();
                    let connection = connection.clone();
                    priority.spawn(scheduler::Priority::High, async move {
                        loop {
                            let readable = {
                                let connection = connection.lock().await;
                                connection.readable()
                            };
                            let dispatch_table = dispatch_table.lock().await;
                            for stream_id in readable {
                                match dispatch_table.get(&stream_id) {
                                    Some(update) => update.Reset(),
                                    None => (),
                                };
                            }

                            received_packet_event_receiver.WaitOnce().await;
                        }
                    })
                };
                
                priority.run().await
            })
        };



        {
            let connection = connection.clone();
            scheduler.run(async move {
                loop {
                    let is_connected = {
                        let connection = connection.lock().await;
                        connection.is_established()
                    };
                    if is_connected{
                        break;
                    }
                    smol::Timer::after(std::time::Duration::from_millis(100)).await;
                }
            })
        }.await;

        let ret = SmolQuic{
            connection,
            dispatch_table,
            _task: task
        };

        Ok(ret)
    }
}