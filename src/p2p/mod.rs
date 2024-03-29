use chrono::DateTime;

use {
    crate::{chain::Chain, storage::Storage},
    bincode::Options,
    chrono::Utc,
    ed25519_consensus::{Signature, SigningKey, VerificationKey, VerificationKeyBytes},
    rand::{prelude::SliceRandom, thread_rng},
    rayon::{
        iter::{IntoParallelIterator, ParallelIterator},
        ThreadPool, ThreadPoolBuilder,
    },
    serde_derive::{Deserialize, Serialize},
    std::{
        collections::{HashMap, HashSet},
        io::{self, Read, Write},
        net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream, UdpSocket},
        sync::{
            atomic::{AtomicBool, Ordering},
            mpsc::{channel, Receiver, RecvTimeoutError, SendError, Sender},
            Arc,
        },
        thread::{self, JoinHandle},
        time::Duration,
    },
    thiserror::Error,
};

const GOSSIP_BUFFER_SIZE: usize = 2_usize.pow(16);
const RECEIVER_BUFSIZE: usize = 1024;
const RECV_TIMEOUT: Duration = Duration::from_secs(1);
const BLOCK_SYNC_VOTERS: usize = 10;

#[derive(Debug, Error)]
enum P2PError {
    #[error("The receiver timed out")]
    ReceiverTimeout(#[from] RecvTimeoutError),
    #[error("The receiver disconnected")]
    ReceiverDisconnect,
    #[error("The sender could not send")]
    Sender,
    #[error("The serializer could not serialize {0}")]
    Serialize(bincode::Error),
    #[error("We could not discover nodes")]
    CannotDiscover,
    #[error("Tcp error")]
    Tcp,
    #[error("IO error")]
    IOError(#[from] std::io::Error),
}

impl<T> From<SendError<T>> for P2PError {
    fn from(_: SendError<T>) -> Self {
        Self::Sender
    }
}

#[derive(Debug, Serialize, Deserialize)]
enum Protocol {
    GossipPush {},
}

#[derive(Debug, Serialize, Deserialize)]
struct Message {
    pubkey: VerificationKeyBytes,
    signature: Signature,
    data: Vec<u8>,
    timestamp: i64,
}

impl Message {
    pub fn new(
        pubkey: VerificationKeyBytes,
        signature: Signature,
        data: Vec<u8>,
        timestamp: i64,
    ) -> Self {
        Self {
            pubkey,
            signature,
            data,
            timestamp,
        }
    }

    pub fn verify(self) -> Option<Self> {
        let sig_data = [self.data.as_slice(), &self.timestamp.to_le_bytes()].concat();

        if let Ok(key) = VerificationKey::try_from(self.pubkey) {
            match key.verify(&self.signature, &sig_data) {
                Ok(_) => Some(self),
                Err(_) => None,
            }
        } else {
            None
        }
    }
}

fn serialize<T: serde::Serialize>(value: T) -> bincode::Result<Vec<u8>> {
    bincode::serialize(&value)
}

fn deserialize<T>(data: &[u8]) -> bincode::Result<T>
where
    T: serde::de::DeserializeOwned,
{
    bincode::options()
        .with_limit(GOSSIP_BUFFER_SIZE as u64)
        .with_fixint_encoding()
        .allow_trailing_bytes()
        .deserialize_from(data)
}

type BufferedSender<T> = Sender<Vec<T>>;
type BufferedReceiver<T> = Receiver<Vec<T>>;

fn discover(
    listener: TcpListener,
    cluster_info: Arc<ClusterInfo>,
    target: usize,
) -> Result<HashSet<SocketAddr>, P2PError> {
    const TIMEOUT: Duration = Duration::from_secs(2);
    let mut discovered = HashSet::new();

    let (send, recv) = channel();
    let exit = Arc::new(AtomicBool::new(false));
    let receiver_handle = tcp_receiver(listener, send, &exit, "discover-receiver");

    while discovered.len() < target {
        let addr = cluster_info.get_discovery_node().unwrap(); // TODO: find a pretty way so that we do not dial the same peer more than once, and that if it errors out, we retry.

        let stream = &mut TcpStream::connect_timeout(addr, TIMEOUT);
        match stream {
            Ok(stream) => {
                let _ = send_tcp(stream, cluster_info.new_discovery_message());
            }
            Err(err) => tracing::debug!("error connecting to {:?}: {:?}", addr, err),
        }

        if let Ok(message_bytes) = recv.recv_timeout(TIMEOUT) {
            if let Ok(received_contacts) = deserialize::<Vec<SocketAddr>>(&message_bytes) {
                for contact in received_contacts {
                    discovered.insert(contact);
                }
            }
        }
    }
    exit.store(true, Ordering::Relaxed);
    receiver_handle.join().unwrap();

    Ok(discovered)
}

fn block_sync(
    listener: TcpListener,
    since: DateTime<Utc>,
    cluster_info: Arc<ClusterInfo>,
    chain: &mut Chain,
) -> Result<(), P2PError> {
    let contacts: Vec<SocketAddr> = discover(listener.try_clone().unwrap(), cluster_info, 100)?
        .into_iter()
        .collect();

    let (send, recv) = channel();
    let exit = Arc::new(AtomicBool::new(false));
    let receiver_handle = tcp_receiver(listener, send, &exit, "sync-reciever");

    let voters: Vec<&SocketAddr> = contacts
        .choose_multiple(&mut thread_rng(), BLOCK_SYNC_VOTERS)
        .collect(); // TODO: maybe weight with the staking distribution?

    Ok(())
}

fn send_udp(socket: &UdpSocket, addr: &SocketAddr, message: Message) -> io::Result<usize> {
    socket.send_to(&serialize(message).unwrap(), addr)
}

fn send_tcp(stream: &mut TcpStream, message: Message) -> io::Result<usize> {
    stream.write(&serialize(message).unwrap())
}

pub struct ClusterInfo {
    keypair: Arc<SigningKey>,
    contact_list: Vec<SocketAddr>,
    boot_nodes: Vec<SocketAddr>,
}

impl ClusterInfo {
    pub fn new(keypair: Arc<SigningKey>, storage: Arc<dyn Storage>) -> Self {
        let contact_bytes = storage.get_or_set(b"contact_list", b"{}");
        let contact_list = contact_bytes
            .chunks_exact(6)
            .map(Self::ipv4_from_bytes)
            .collect();

        Self {
            keypair,
            contact_list,
            boot_nodes: vec![],
        }
    }

    fn ipv4_from_bytes(bytes: &[u8]) -> SocketAddr {
        let ip = Ipv4Addr::new(bytes[0], bytes[1], bytes[2], bytes[3]);
        let port = ((bytes[4] as u16) << 8) | bytes[5] as u16;
        SocketAddr::new(ip.into(), port)
    }

    fn get_discovery_node(&self) -> Option<&SocketAddr> {
        let rng = &mut thread_rng();
        if self.contact_list.is_empty() {
            self.boot_nodes.choose(rng)
        } else {
            self.contact_list.choose(rng)
        }
    }

    fn new_discovery_message(&self) -> Message {
        let timestamp = Utc::now().timestamp_millis();
        let msg = r#"{"service": "discovery"}"#.as_bytes();
        Message::new(
            VerificationKeyBytes::from(self.keypair.verification_key()),
            self.keypair.sign(msg),
            msg.to_vec(),
            timestamp,
        )
    }

    fn new_initiate_sync_message(&self, since: DateTime<Utc>) -> Message {
        // maybe message should be an enum and then we could just match on the deserialized message?
        let timestamp = Utc::now().timestamp_millis();
        let msg = format!(r#"{{"service":"block_sync","since":{}}}"#, since);
        Message::new(
            VerificationKeyBytes::from(self.keypair.verification_key()),
            self.keypair.sign(msg.as_bytes()),
            msg.into_bytes(),
            timestamp,
        )
    }
}

pub struct GossipMessage {
    author: [u8; 32],
    message: Vec<u8>,
}

pub struct GossipService {
    threads: Vec<JoinHandle<()>>,
}

impl GossipService {
    pub fn new(
        cluster_info: Arc<ClusterInfo>,
        socket: UdpSocket,
        exit: &Arc<AtomicBool>,
    ) -> (Self, Receiver<GossipMessage>) {
        let socket = Arc::new(socket);

        let mut gossip = GossipService { threads: vec![] };

        tracing::info!("Listening on {}.", socket.local_addr().unwrap());

        let (req_send, req_recv) = channel();

        let exit = exit.clone();
        let h_receiver = udp_receiver(socket, req_send, &exit, "gossip");

        let (consume_send, consume_recv) = channel();
        let h_socket_consume = Self::signature_verifier(consume_send, req_recv, exit.clone());

        let (validator_send, validator_recv) = channel();
        let h_listener = Self::listen(consume_recv, validator_send, exit);
        gossip.threads = vec![h_receiver, h_socket_consume, h_listener];

        (gossip, validator_recv)
    }

    fn listen(
        receiver: BufferedReceiver<Message>,
        sender: Sender<GossipMessage>,
        exit: Arc<AtomicBool>,
    ) -> JoinHandle<()> {
        thread::Builder::new()
            .name("listen".to_string())
            .spawn(move || {
                let mut logs = HashMap::new();

                const PURGE_TIME: i64 = 120 * 1000;
                while !exit.load(Ordering::Relaxed) {
                    if let Ok(messages) = receiver.recv_timeout(RECV_TIMEOUT) {
                        let valid_messages: Vec<_> = messages
                            .iter()
                            .filter_map(|msg| {
                                if Utc::now().timestamp_millis() - msg.timestamp < PURGE_TIME
                                    && !logs.contains_key(&msg.timestamp)
                                {
                                    logs.insert(msg.timestamp, msg.signature);
                                    Some((&msg.data, msg.pubkey.to_bytes()))
                                } else {
                                    None
                                }
                            })
                            .collect();

                        valid_messages.iter().for_each(|data| {
                            sender
                                .send(GossipMessage {
                                    author: data.1,
                                    message: data.0.to_vec(),
                                })
                                .unwrap()
                        });
                    }
                }
            })
            .unwrap()
    }

    fn signature_verifier(
        sender: BufferedSender<Message>,
        receiver: BufferedReceiver<Vec<u8>>,
        exit: Arc<AtomicBool>,
    ) -> JoinHandle<()> {
        let thread_pool = ThreadPoolBuilder::new()
            .num_threads(8)
            .thread_name(|i| format!("teral-socket-consume({})", i))
            .build()
            .unwrap();

        thread::Builder::new()
            .name("socket-consume".to_string())
            .spawn(move || {
                while !exit.load(Ordering::Relaxed) {
                    match Self::signature_verifier_thread(&thread_pool, &sender, &receiver) {
                        Err(P2PError::ReceiverTimeout(_)) => tracing::debug!("timeout somehow"),
                        Err(P2PError::Sender) => break,
                        Err(P2PError::ReceiverDisconnect) => break,
                        Err(err) => tracing::error!("socket-consume: {:?}", err),
                        Ok(()) => (),
                    }
                }
            })
            .unwrap()
    }

    fn signature_verifier_thread(
        thread_pool: &ThreadPool,
        sender: &BufferedSender<Message>,
        receiver: &BufferedReceiver<Vec<u8>>,
    ) -> Result<(), P2PError> {
        let verify_sig = |data: Vec<u8>| {
            let message: bincode::Result<Message> = deserialize(&data);
            match message {
                Ok(message) => Some(message.verify()?),
                Err(_) => None,
            }
        };

        let packets = receiver.recv_timeout(RECV_TIMEOUT)?;
        let packets: Vec<_> =
            thread_pool.install(|| packets.into_par_iter().filter_map(verify_sig).collect());

        Ok(sender.send(packets)?)
    }

    pub fn join(self) -> thread::Result<()> {
        for t in self.threads {
            t.join()?;
        }
        Ok(())
    }
}

fn udp_receiver(
    socket: Arc<UdpSocket>,
    channel: BufferedSender<Vec<u8>>,
    exit: &Arc<AtomicBool>,
    name: &str,
) -> JoinHandle<()> {
    let exit = exit.clone();

    thread::Builder::new()
        .name(String::from(name))
        .spawn(move || {
            let _ = udp_recv_loop(&socket, channel, exit.clone());
        })
        .unwrap()
}

fn udp_recv_loop(
    socket: &UdpSocket,
    channel: BufferedSender<Vec<u8>>,
    exit: Arc<AtomicBool>,
) -> Result<(), P2PError> {
    socket.set_read_timeout(Some(RECV_TIMEOUT)).unwrap();
    loop {
        let mut msg_buf = Vec::new();
        msg_buf.reserve(RECEIVER_BUFSIZE);
        while msg_buf.len() < RECEIVER_BUFSIZE {
            if exit.load(Ordering::Relaxed) {
                return Ok(());
            }

            let mut buf = [0; GOSSIP_BUFFER_SIZE];
            match socket.recv_from(&mut buf) {
                Ok((len, _)) if len > 0 => msg_buf.push(buf[..len].to_vec()),
                _ => {}
            }
        }
        channel.send(msg_buf).unwrap();
    }
}

fn tcp_receiver(
    listener: TcpListener,
    channel: Sender<Vec<u8>>,
    exit: &Arc<AtomicBool>,
    name: &str,
) -> JoinHandle<()> {
    let exit = exit.clone();

    thread::Builder::new()
        .name(String::from(name))
        .spawn(move || {
            let _ = tcp_recv_loop(listener, channel, exit);
        })
        .unwrap()
}

fn tcp_recv_loop(
    listener: TcpListener,
    channel: Sender<Vec<u8>>,
    exit: Arc<AtomicBool>,
) -> Result<(), P2PError> {
    listener.set_nonblocking(true)?;
    loop {
        if exit.load(Ordering::Relaxed) {
            return Ok(());
        }
        if let Ok((mut stream, _)) = listener.accept() {
            let _ = stream.set_read_timeout(Some(RECV_TIMEOUT));
            let mut buf = Vec::new();
            if stream.read_to_end(&mut buf).is_ok() {
                channel.send(buf).unwrap();
            }
        }
    }
}
