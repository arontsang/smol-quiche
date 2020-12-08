
use async_net::UdpSocket;
use crossfire::mpmc::{TxFuture, RxFuture, SharedFutureBoth};
use smol::channel::{Sender, Receiver, RecvError};
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
    dispatch_table: Rc::<Mutex::<HashMap::<u64, Sender<quiche::h3::Event>>>>,
    send_packet_event_sender: scheduler::RaiseEvent,
    received_packet_event_receiver: RxFuture::<(), SharedFutureBoth>,
    timeout_event_sender: scheduler::RaiseEvent, 
    _task: smol::Task::<()>,
}

pub struct QuicClientRequest {
    stream_id: u64,
    stream_event_receiver: Receiver<quiche::h3::Event>,
    connection: RcLock::<PinnedQuicConnection>,
    http_client: RcLock::<quiche::h3::Connection>,
}

impl SmolQuic {

    pub async fn new(config: &mut quiche::Config, scheduler: &LocalExecutor<'_>) -> Result<SmolQuic, quiche::Error> {
        
        let destination: &str = "quic.tech";
        let destination: Option<&str> = Some(destination);
        let scid = [0xba; quiche::MAX_CONN_ID_LEN];

        let connection = quiche::connect(destination, &scid, config)?;
        let connection = Mutex::new(connection);
        let connection = Rc::new(connection);

        let (send_packet_event_sender, send_packet_event_receiver) = scheduler::auto_reset_event();
        let (timeout_event_sender, timeout_event) = scheduler::auto_reset_event();
        let (received_packet_event_sender, received_packet_event_receiver) = crossfire::mpmc::bounded_future_both::<()>(1);

        let dispatch_table = Rc::new(Mutex::new(HashMap::<u64, Sender<quiche::h3::Event>>::new()));

        let task = {          
            let connection = connection.clone();
            let timeout_event_sender = timeout_event_sender.clone();
            let send_packet_event_sender = send_packet_event_sender.clone();
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
                        }
                        send_packet_event_sender.reset();
                        loop {
                            let next_timeout = {
                                let connection = connection.lock().await;
                                connection.timeout()
                            };
                            match next_timeout {
                                Some(timeout) => {
                                    println!("Hello!");
                                    smol::Timer::after(timeout).await;                                
                                    {
                                        let mut connection = connection.lock().await;
                                        connection.on_timeout();
                                    }
                                    send_packet_event_sender.reset();
                                },
                                None => {
                                    timeout_event.wait_once().await;
                                }
                            }
                        }
                    })
                };

                let _recv_loop = {
                    let socket = socket.clone();
                    let connection = connection.clone();
                    let received_packet_event_sender = received_packet_event_sender.clone();
                    let timeout_event_sender = timeout_event_sender.clone();
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
                            timeout_event_sender.reset();

                            //received_packet_event_sender.send(()).await.unwrap();
                            if let Ok(_) = received_packet_event_sender.try_send(()) { () };
                        }
                    })
                };

                let _send_loop = {
                    let socket = socket.clone();
                    let connection = connection.clone();
                    let timeout_event_sender = timeout_event_sender.clone();
                    priority.spawn(scheduler::Priority::High, async move {
                        let destination = async_std::net::SocketAddr::new(IpAddr::V4(Ipv4Addr::new(45, 77, 96, 66)), 8443);
        
                        let mut send_buffer: [u8; 1500] = [0; 1500];
                        loop {                            
                            'send_loop: loop {
                                let mut connection = connection.lock().await;
                                match connection.send(&mut send_buffer) {
                                    Ok(length) => {
                                        match socket.send_to(&send_buffer[0..length], destination).await {
                                            Ok(_) => (),
                                            Err(err) => println!("{}", err)
                                        }
                                    },
                                    Err(quiche::Error::Done) => break 'send_loop,                               
                                    Err(_) => break 'send_loop,
                                };
        
                            }
                            timeout_event_sender.reset();
                            send_packet_event_receiver.wait_once().await;
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
                    smol::Timer::after(std::time::Duration::from_millis(1)).await;
                }
            })
        }.await;

        let ret = SmolQuic{
            connection,
            dispatch_table,
            received_packet_event_receiver,
            send_packet_event_sender,
            
            timeout_event_sender: timeout_event_sender,
            _task: task
        };

        Ok(ret)
    }
}

pub struct SmolHttp3Client {

    connection: RcLock::<PinnedQuicConnection>,
    http_client: RcLock::<quiche::h3::Connection>,
    send_packet_event_sender: scheduler::RaiseEvent,
    dispatch_table: Rc::<Mutex::<HashMap::<u64, Sender<quiche::h3::Event>>>>,
    timeout_event_sender: scheduler::RaiseEvent,
    _dispatching_task: smol::Task::<()>
}

impl SmolHttp3Client {
    pub async fn new(connection: &SmolQuic, scheduler: &LocalExecutor<'_>) -> Result<SmolHttp3Client, quiche::h3::Error> {
        let mut h3_config = quiche::h3::Config::new()?;
        h3_config.set_max_header_list_size(128);
        h3_config.set_qpack_blocked_streams(100000);
        h3_config.set_qpack_max_table_capacity(16);

        let http_client = {
            let mut connection = connection.connection.lock().await;
            quiche::h3::Connection::with_transport(&mut connection, &h3_config)?
        };

        let http_client = Rc::new(Mutex::new(http_client));

        let _dispatching_task = {
            let received_packet_event_receiver = connection.received_packet_event_receiver.clone();
            let http_client = http_client.clone();
            let dispatch_table = connection.dispatch_table.clone();
            let connection = connection.connection.clone();
            scheduler.spawn(async move {
                loop {
                    {

                        
                        loop {     
                            let evnt = {
                                let mut connection_state = connection.lock().await;
                                let mut http_client = http_client.lock().await;
                                http_client.poll(&mut connection_state)
                            };
                            
                            match evnt {
                                Ok((stream_id, evnt)) => {
                                    let dispatch_table = dispatch_table.lock().await;
                                    if let Some(foo) = dispatch_table.get(&stream_id) {
                                        foo.send(evnt).await.unwrap();
                                    }
                                },
                                Err(quiche::h3::Error::Done) => {
                                    break;
                                }
                                Err(er) => {
                                    println!("{}", er);
                                    break;
                                }

                            }

                        }

                        
                        
                    }
                    
                    match received_packet_event_receiver.recv().await {
                        Err(x) => {
                            ()
                        },
                        _ => ()
                    };
                }
            })
        };

        let ret = SmolHttp3Client{
            connection: connection.connection.clone(),
            http_client: http_client,
            dispatch_table: connection.dispatch_table.clone(),
            send_packet_event_sender: connection.send_packet_event_sender.clone(),
            timeout_event_sender: connection.timeout_event_sender.clone(),
            _dispatching_task: _dispatching_task
        };

        Ok(ret)
    }

    pub async fn send_request<T: quiche::h3::NameValue>(
        &mut self, 
        headers: &[T],
        
    ) -> quiche::h3::Result<QuicClientRequest> {
        let stream_id = {
            let mut connection = self.connection.lock().await;
            let mut http_client = self.http_client.lock().await;
            let stream_id = http_client.send_request(&mut connection, &headers, true)?;
            //let empty : [u8; 0] = [0;0];
            //http_client.send_body(&mut connection, stream_id, &empty, true)?;
            
            stream_id
        };

        
        let (stream_event_sender, stream_event_receiver) = smol::channel::bounded::<quiche::h3::Event>(8);
        {
            let mut dispatch_table = self.dispatch_table.lock().await;
            dispatch_table.insert(stream_id, stream_event_sender);
        }

        let ret = QuicClientRequest {
            stream_id,
            stream_event_receiver: stream_event_receiver,
            connection: self.connection.clone(),
            http_client: self.http_client.clone(),
        };

        self.send_packet_event_sender.reset();
        self.timeout_event_sender.reset();

        Ok(ret)
    }
}

impl QuicClientRequest {
    pub async fn wait_events(&self) -> Result<quiche::h3::Event, RecvError> {
        self.stream_event_receiver.recv().await
    }

    pub async fn read_body(&self, buffer: &mut [u8]) -> Result<usize, quiche::h3::Error> {
        
        let mut connection = self.connection.lock().await;
        let mut http_client = self.http_client.lock().await;

        http_client.recv_body(&mut connection, self.stream_id, buffer)
    }
}