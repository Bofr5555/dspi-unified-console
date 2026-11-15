//! A blocking MQTT 3.1.1 client on one thread (WP P2MQC, OD-P2c).
//!
//! [`MqttClient::start`] spawns the thread and returns at once. The thread connects over a
//! plain [`TcpStream`] (no TLS, OD-P2f), sends CONNECT with the will and the credentials,
//! subscribes to every filter asked for so far, and then sends what [`MqttClient::publish`]
//! queued, answers QoS 1 publishes with PUBACK, keeps the connection alive with PINGREQ and
//! hands what arrives to the caller's handler. When the connection ends it reconnects with
//! a doubling backoff and subscribes again; QoS 1 publishes not yet acknowledged are sent
//! again with the dup flag.
//!
//! Bounded everything: the outgoing queue keeps at most `queue_cap` publishes and drops the
//! oldest when full (counted in [`MqttStatus::dropped`]); at most [`MAX_INFLIGHT`] QoS 1
//! publishes wait for their PUBACK; the receive buffer never holds more than one packet of
//! `max_packet` bytes plus one read; at most [`MAX_SUBSCRIPTIONS`] filters.
//!
//! The password is never written to a log, an event, an error or a `Debug` (OD-P2e).
//!
//! Lock order: the client's one lock is a leaf, never held across socket I/O or the handler.

use std::collections::VecDeque;
use std::fmt;
use std::io::{ErrorKind, Read, Write};
use std::net::{Shutdown, TcpStream, ToSocketAddrs};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use super::codec::{self, Connect, DecodeError, Packet, Publish, QoS, Will};

/// The MQTT port without TLS.
pub const DEFAULT_MQTT_PORT: u16 = 1883;
/// QoS 1 publishes sent and not yet acknowledged, at most.
pub const MAX_INFLIGHT: usize = 16;
/// Topic filters a client subscribes to, at most.
pub const MAX_SUBSCRIPTIONS: usize = 256;
/// Filters per SUBSCRIBE packet when the client subscribes again after a reconnect.
const FILTERS_PER_SUBSCRIBE: usize = 16;
/// SUBSCRIBEs waiting for their SUBACK that the client remembers, at most.
const MAX_PENDING_SUBACKS: usize = 64;
/// One read from the socket.
const READ_CHUNK: usize = 4096;

/// How to reach the broker and how the client behaves. Its `Debug` shows whether a
/// password is set, never the password.
#[derive(Clone)]
pub struct MqttConfig {
    /// The broker's host name or address.
    pub host: String,
    /// The broker's port ([`DEFAULT_MQTT_PORT`]).
    pub port: u16,
    /// The client identifier.
    pub client_id: String,
    /// The user name, if the broker wants one.
    pub username: Option<String>,
    /// The password; MQTT 3.1.1 sends one only with a user name.
    pub password: Option<String>,
    /// Keep-alive (whole seconds, at most 65 535; zero turns it off).
    pub keep_alive: Duration,
    /// The last will, published by the broker when the client goes away unannounced.
    pub will: Option<Will>,
    /// Publishes the outgoing queue keeps while the broker is away; the oldest go first.
    pub queue_cap: usize,
    /// The longest packet sent or taken, fixed header included.
    pub max_packet: usize,
    /// For the TCP connect and for the CONNACK, and the socket's write timeout.
    pub connect_timeout: Duration,
    /// The first wait after a failed or ended connection.
    pub backoff_min: Duration,
    /// The longest wait; each failure doubles the wait up to this.
    pub backoff_max: Duration,
    /// How often the thread looks at the queue while connected (the socket's read timeout).
    pub tick: Duration,
}

impl MqttConfig {
    /// A configuration with the defaults: no credentials, no will, keep-alive 30 s, 256
    /// queued publishes, 256 KiB packets, 5 s connect timeout, backoff 1 s to 60 s.
    pub fn new(host: impl Into<String>, port: u16, client_id: impl Into<String>) -> MqttConfig {
        MqttConfig {
            host: host.into(),
            port,
            client_id: client_id.into(),
            username: None,
            password: None,
            keep_alive: Duration::from_secs(30),
            will: None,
            queue_cap: 256,
            max_packet: 256 * 1024,
            connect_timeout: Duration::from_secs(5),
            backoff_min: Duration::from_secs(1),
            backoff_max: Duration::from_secs(60),
            tick: Duration::from_millis(25),
        }
    }
}

impl fmt::Debug for MqttConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MqttConfig")
            .field("host", &self.host)
            .field("port", &self.port)
            .field("client_id", &self.client_id)
            .field("username", &self.username)
            .field(
                "password",
                &if self.password.is_some() {
                    "set"
                } else {
                    "not set"
                },
            )
            .field("keep_alive", &self.keep_alive)
            .field("will", &self.will)
            .field("queue_cap", &self.queue_cap)
            .field("max_packet", &self.max_packet)
            .field("connect_timeout", &self.connect_timeout)
            .field("backoff_min", &self.backoff_min)
            .field("backoff_max", &self.backoff_max)
            .field("tick", &self.tick)
            .finish()
    }
}

/// What the client tells its handler. The handler runs on the client's thread: it should
/// hand the event on and return, and it must not drop the [`MqttClient`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MqttEvent {
    /// The broker accepted the connection.
    Connected {
        /// The broker kept a session for this client.
        session_present: bool,
    },
    /// A connection ended, or could not be made; the client tries again after its backoff.
    Disconnected {
        /// Why, for the log and `mqtt.status`. Never holds the password.
        reason: String,
    },
    /// A message on a subscribed topic.
    Message {
        /// Its topic.
        topic: String,
        /// Its payload.
        payload: Vec<u8>,
        /// The broker's retained message for the topic, sent on subscribing.
        retain: bool,
    },
    /// The broker refused a subscription (SUBACK 0x80).
    SubscribeRefused {
        /// The filter.
        filter: String,
    },
}

/// The client's state, for `mqtt.status`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MqttStatus {
    /// Connected and accepted by the broker.
    pub connected: bool,
    /// Publishes waiting in the queue.
    pub queued: usize,
    /// QoS 1 publishes sent and not yet acknowledged.
    pub inflight: usize,
    /// Publishes dropped because the queue was full.
    pub dropped: u64,
    /// Connections the broker accepted since the start.
    pub connects: u64,
    /// Why the last connection ended or failed.
    pub last_error: Option<String>,
}

/// Why the client did not take a call.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum MqttError {
    /// The configuration cannot make a valid CONNECT.
    #[error("invalid configuration: {0}")]
    Config(String),
    /// Not a topic name to publish to (empty, a wildcard, NUL, too long).
    #[error("invalid topic name")]
    InvalidTopic,
    /// Not a valid topic filter.
    #[error("invalid topic filter")]
    InvalidFilter,
    /// The publish would be longer than `max_packet`.
    #[error("publish of {len} bytes is over the limit of {max}")]
    TooLarge {
        /// Its encoded length.
        len: usize,
        /// The limit.
        max: usize,
    },
    /// Already [`MAX_SUBSCRIPTIONS`] filters.
    #[error("too many subscriptions")]
    TooManySubscriptions,
    /// The thread could not start.
    #[error("could not start the client thread: {0}")]
    Spawn(String),
}

/// The outgoing queue: bounded, dropping the oldest when full.
#[derive(Debug)]
struct OutQueue {
    cap: usize,
    items: VecDeque<Publish>,
    dropped: u64,
}

impl OutQueue {
    fn new(cap: usize) -> OutQueue {
        OutQueue {
            cap,
            items: VecDeque::new(),
            dropped: 0,
        }
    }

    fn push(&mut self, p: Publish) {
        if self.cap == 0 {
            self.dropped = self.dropped.saturating_add(1);
            return;
        }
        while self.items.len() >= self.cap {
            self.items.pop_front();
            self.dropped = self.dropped.saturating_add(1);
        }
        self.items.push_back(p);
    }
}

/// The wait before the next connection attempt: doubling from `min` to `max`.
#[derive(Debug)]
struct Backoff {
    min: Duration,
    max: Duration,
    next: Duration,
}

impl Backoff {
    fn new(min: Duration, max: Duration) -> Backoff {
        let max = max.max(min);
        Backoff {
            min,
            max,
            next: min,
        }
    }

    fn next_delay(&mut self) -> Duration {
        let d = self.next;
        self.next = self.next.saturating_mul(2).min(self.max);
        d
    }

    fn reset(&mut self) {
        self.next = self.min;
    }
}

#[derive(Debug)]
struct Shared {
    queue: OutQueue,
    subs: Vec<(String, QoS)>,
    new_subs: Vec<(String, QoS)>,
    stop: bool,
    status: MqttStatus,
}

struct Inner {
    state: Mutex<Shared>,
    wake: Condvar,
}

impl Inner {
    fn lock(&self) -> MutexGuard<'_, Shared> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// The hub's MQTT client: a handle on its thread. Dropping it disconnects and stops it.
pub struct MqttClient {
    inner: Arc<Inner>,
    thread: Option<JoinHandle<()>>,
    max_packet: usize,
}

impl fmt::Debug for MqttClient {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MqttClient")
            .field("status", &self.status())
            .finish()
    }
}

impl MqttClient {
    /// Checks the configuration and starts the client's thread, which connects and keeps
    /// connecting. `handler` gets every [`MqttEvent`] on that thread.
    pub fn start(
        config: MqttConfig,
        handler: impl FnMut(MqttEvent) + Send + 'static,
    ) -> Result<MqttClient, MqttError> {
        if u16::try_from(config.keep_alive.as_secs()).is_err() {
            return Err(MqttError::Config("keep-alive over 65 535 s".into()));
        }
        if config.password.is_some() && config.username.is_none() {
            return Err(MqttError::Config("a password needs a user name".into()));
        }
        if config.tick.is_zero() {
            return Err(MqttError::Config("tick must not be zero".into()));
        }
        // The CONNECT must encode, and fit.
        let connect = connect_packet(&config);
        let len = codec::encode(&connect)
            .map_err(|e| MqttError::Config(e.to_string()))?
            .len();
        if len > config.max_packet {
            return Err(MqttError::Config("CONNECT over max_packet".into()));
        }
        let inner = Arc::new(Inner {
            state: Mutex::new(Shared {
                queue: OutQueue::new(config.queue_cap),
                subs: Vec::new(),
                new_subs: Vec::new(),
                stop: false,
                status: MqttStatus::default(),
            }),
            wake: Condvar::new(),
        });
        let max_packet = config.max_packet;
        let worker = Worker {
            backoff: Backoff::new(config.backoff_min, config.backoff_max),
            cfg: config,
            inner: Arc::clone(&inner),
            handler: Box::new(handler),
            inflight: VecDeque::new(),
            pending_subacks: VecDeque::new(),
            next_id: 1,
        };
        let thread = std::thread::Builder::new()
            .name("dspi-mqtt".into())
            .spawn(move || worker.run())
            .map_err(|e| MqttError::Spawn(e.to_string()))?;
        Ok(MqttClient {
            inner,
            thread: Some(thread),
            max_packet,
        })
    }

    /// Queues a publish. It goes out when connected; while the broker is away the queue
    /// keeps the newest `queue_cap` publishes.
    pub fn publish(
        &self,
        topic: &str,
        payload: &[u8],
        qos: QoS,
        retain: bool,
    ) -> Result<(), MqttError> {
        if !codec::valid_topic(topic) {
            return Err(MqttError::InvalidTopic);
        }
        let p = Publish {
            topic: topic.to_owned(),
            payload: payload.to_vec(),
            qos,
            retain,
            dup: false,
            packet_id: 0,
        };
        let len = publish_len(&p);
        if len > self.max_packet {
            return Err(MqttError::TooLarge {
                len,
                max: self.max_packet,
            });
        }
        let mut s = self.inner.lock();
        s.queue.push(p);
        s.status.queued = s.queue.items.len();
        s.status.dropped = s.queue.dropped;
        Ok(())
    }

    /// Subscribes to `filter`, now if connected and again after every reconnect. Asking for
    /// a filter already subscribed with the same QoS does nothing.
    pub fn subscribe(&self, filter: &str, qos: QoS) -> Result<(), MqttError> {
        if !codec::valid_filter(filter) {
            return Err(MqttError::InvalidFilter);
        }
        let mut s = self.inner.lock();
        if let Some(entry) = s.subs.iter_mut().find(|(f, _)| f == filter) {
            if entry.1 == qos {
                return Ok(());
            }
            entry.1 = qos;
        } else {
            if s.subs.len() >= MAX_SUBSCRIPTIONS {
                return Err(MqttError::TooManySubscriptions);
            }
            s.subs.push((filter.to_owned(), qos));
        }
        s.new_subs.retain(|(f, _)| f != filter);
        s.new_subs.push((filter.to_owned(), qos));
        Ok(())
    }

    /// The client's state now.
    pub fn status(&self) -> MqttStatus {
        self.inner.lock().status.clone()
    }

    /// Sends DISCONNECT if connected (so the broker does not publish the will) and stops
    /// the thread; waits for it unless called on that thread.
    pub fn stop(&mut self) {
        self.inner.lock().stop = true;
        self.inner.wake.notify_all();
        if let Some(t) = self.thread.take()
            && t.thread().id() != std::thread::current().id()
        {
            let _ = t.join();
        }
    }
}

impl Drop for MqttClient {
    fn drop(&mut self) {
        self.stop();
    }
}

/// A publish's encoded length, without encoding it.
fn publish_len(p: &Publish) -> usize {
    let id = match p.qos {
        QoS::AtMostOnce => 0,
        QoS::AtLeastOnce => 2,
    };
    let rem = p
        .topic
        .len()
        .saturating_add(2)
        .saturating_add(id)
        .saturating_add(p.payload.len());
    let header = match rem {
        0..=127 => 2usize,
        128..=16_383 => 3,
        16_384..=2_097_151 => 4,
        _ => 5,
    };
    rem.saturating_add(header)
}

fn connect_packet(cfg: &MqttConfig) -> Packet {
    Packet::Connect(Connect {
        client_id: cfg.client_id.clone(),
        keep_alive: u16::try_from(cfg.keep_alive.as_secs()).unwrap_or(u16::MAX),
        clean_session: true,
        will: cfg.will.clone(),
        username: cfg.username.clone(),
        password: cfg.password.as_ref().map(|p| p.as_bytes().to_vec()),
    })
}

/// Why a connection ended. Its text goes to events and the status; it never holds the
/// password.
#[derive(Debug, thiserror::Error)]
enum Dropped {
    #[error("{0}")]
    Io(String),
    #[error("the broker closed the connection")]
    Closed,
    #[error("the broker refused the connection: {}", codec::connack_reason(*.0))]
    Refused(u8),
    #[error("no CONNACK in time")]
    ConnackTimeout,
    #[error("no answer to PINGREQ in time")]
    KeepAlive,
    #[error("protocol error: {0}")]
    Protocol(&'static str),
    #[error("{0}")]
    Decode(#[from] DecodeError),
}

impl From<std::io::Error> for Dropped {
    fn from(e: std::io::Error) -> Dropped {
        Dropped::Io(e.to_string())
    }
}

/// The bytes read and not yet decoded: at most one unfinished packet plus one read.
struct RxBuf {
    buf: Vec<u8>,
    max_packet: usize,
}

impl RxBuf {
    /// One read; a timeout is not an error.
    fn fill(&mut self, stream: &mut TcpStream) -> Result<(), Dropped> {
        let mut chunk = [0u8; READ_CHUNK];
        match stream.read(&mut chunk) {
            Ok(0) => Err(Dropped::Closed),
            Ok(n) => {
                self.buf.extend_from_slice(chunk.get(..n).unwrap_or(&[]));
                Ok(())
            }
            Err(e)
                if matches!(
                    e.kind(),
                    ErrorKind::WouldBlock | ErrorKind::TimedOut | ErrorKind::Interrupted
                ) =>
            {
                Ok(())
            }
            Err(e) => Err(e.into()),
        }
    }

    fn take(&mut self) -> Result<Option<Packet>, Dropped> {
        match codec::decode(&self.buf, self.max_packet)? {
            Some((p, used)) => {
                // `decode` never reports more than it was given.
                self.buf.drain(..used.min(self.buf.len()));
                Ok(Some(p))
            }
            None => Ok(None),
        }
    }
}

struct Worker {
    cfg: MqttConfig,
    inner: Arc<Inner>,
    handler: Box<dyn FnMut(MqttEvent) + Send>,
    /// QoS 1 publishes sent and not acknowledged, oldest first; kept across reconnects.
    inflight: VecDeque<Publish>,
    /// SUBSCRIBE packet ids waiting for SUBACK, with their filters.
    pending_subacks: VecDeque<(u16, Vec<String>)>,
    next_id: u16,
    backoff: Backoff,
}

impl Worker {
    fn run(mut self) {
        loop {
            if self.inner.lock().stop {
                return;
            }
            let reason = match self.session() {
                Ok(()) => return,
                Err(e) => e.to_string(),
            };
            {
                let mut s = self.inner.lock();
                s.status.connected = false;
                s.status.last_error = Some(reason.clone());
            }
            log::info!("mqtt: {}:{}: {reason}", self.cfg.host, self.cfg.port);
            (self.handler)(MqttEvent::Disconnected { reason });
            let delay = self.backoff.next_delay();
            let until = Instant::now().checked_add(delay);
            let mut s = self.inner.lock();
            while !s.stop {
                let left = until.map_or(delay, |u| u.saturating_duration_since(Instant::now()));
                if left.is_zero() {
                    break;
                }
                s = self
                    .inner
                    .wake
                    .wait_timeout(s, left)
                    .unwrap_or_else(PoisonError::into_inner)
                    .0;
            }
        }
    }

    fn open(&self) -> Result<TcpStream, Dropped> {
        let mut last = Dropped::Io(format!("{}: no address", self.cfg.host));
        for addr in (self.cfg.host.as_str(), self.cfg.port).to_socket_addrs()? {
            match TcpStream::connect_timeout(&addr, self.cfg.connect_timeout) {
                Ok(s) => return Ok(s),
                Err(e) => last = e.into(),
            }
        }
        Err(last)
    }

    fn send(stream: &mut TcpStream, packet: &Packet) -> Result<(), Dropped> {
        let bytes =
            codec::encode(packet).map_err(|_| Dropped::Protocol("a packet did not encode"))?;
        stream.write_all(&bytes)?;
        Ok(())
    }

    fn take_id(&mut self) -> u16 {
        for _ in 0..=u16::MAX {
            let id = self.next_id;
            self.next_id = self.next_id.checked_add(1).unwrap_or(1);
            let used = self.inflight.iter().any(|p| p.packet_id == id)
                || self.pending_subacks.iter().any(|(p, _)| *p == id);
            if id != 0 && !used {
                return id;
            }
        }
        // Unreachable: at most MAX_INFLIGHT + MAX_PENDING_SUBACKS ids are in use.
        1
    }

    fn subscribe(&mut self, stream: &mut TcpStream, subs: &[(String, QoS)]) -> Result<(), Dropped> {
        for chunk in subs.chunks(FILTERS_PER_SUBSCRIBE) {
            let packet_id = self.take_id();
            Self::send(
                stream,
                &Packet::Subscribe {
                    packet_id,
                    filters: chunk.to_vec(),
                },
            )?;
            while self.pending_subacks.len() >= MAX_PENDING_SUBACKS {
                self.pending_subacks.pop_front();
            }
            self.pending_subacks
                .push_back((packet_id, chunk.iter().map(|(f, _)| f.clone()).collect()));
        }
        Ok(())
    }

    /// One connection, until it fails (`Err`) or the client stops (`Ok`).
    fn session(&mut self) -> Result<(), Dropped> {
        let mut stream = self.open()?;
        stream.set_read_timeout(Some(self.cfg.tick))?;
        stream.set_write_timeout(Some(self.cfg.connect_timeout))?;
        stream.set_nodelay(true)?;
        let mut rx = RxBuf {
            buf: Vec::new(),
            max_packet: self.cfg.max_packet,
        };
        Self::send(&mut stream, &connect_packet(&self.cfg))?;

        let deadline = Instant::now().checked_add(self.cfg.connect_timeout);
        let session_present = loop {
            if self.inner.lock().stop {
                let _ = stream.shutdown(Shutdown::Both);
                return Ok(());
            }
            if deadline.is_some_and(|d| Instant::now() >= d) {
                return Err(Dropped::ConnackTimeout);
            }
            rx.fill(&mut stream)?;
            match rx.take()? {
                Some(Packet::Connack {
                    session_present,
                    return_code: 0,
                }) => break session_present,
                Some(Packet::Connack { return_code, .. }) => {
                    return Err(Dropped::Refused(return_code));
                }
                Some(_) => return Err(Dropped::Protocol("a packet before CONNACK")),
                None => {}
            }
        };

        self.backoff.reset();
        let subs = {
            let mut s = self.inner.lock();
            s.status.connected = true;
            s.status.connects = s.status.connects.saturating_add(1);
            s.status.last_error = None;
            s.new_subs.clear();
            s.subs.clone()
        };
        (self.handler)(MqttEvent::Connected { session_present });
        self.pending_subacks.clear();
        self.subscribe(&mut stream, &subs)?;
        for p in self.inflight.iter_mut() {
            p.dup = true;
        }
        for p in self.inflight.clone() {
            Self::send(&mut stream, &Packet::Publish(p))?;
        }

        let keep_alive = Duration::from_secs(self.cfg.keep_alive.as_secs());
        let mut last_sent = Instant::now();
        let mut ping_sent: Option<Instant> = None;
        loop {
            let (stop, new_subs, batch) = {
                let mut s = self.inner.lock();
                let new_subs = std::mem::take(&mut s.new_subs);
                let mut batch = Vec::new();
                let mut room = MAX_INFLIGHT.saturating_sub(self.inflight.len());
                while let Some(front) = s.queue.items.front() {
                    if front.qos == QoS::AtLeastOnce {
                        if room == 0 {
                            break;
                        }
                        room = room.saturating_sub(1);
                    }
                    if let Some(p) = s.queue.items.pop_front() {
                        batch.push(p);
                    }
                }
                s.status.queued = s.queue.items.len();
                (s.stop, new_subs, batch)
            };
            if stop {
                let _ = Self::send(&mut stream, &Packet::Disconnect);
                let _ = stream.shutdown(Shutdown::Both);
                return Ok(());
            }
            if !new_subs.is_empty() {
                self.subscribe(&mut stream, &new_subs)?;
                last_sent = Instant::now();
            }
            for mut p in batch {
                if p.qos == QoS::AtLeastOnce {
                    p.packet_id = self.take_id();
                    self.inflight.push_back(p.clone());
                }
                Self::send(&mut stream, &Packet::Publish(p))?;
                last_sent = Instant::now();
            }
            self.inner.lock().status.inflight = self.inflight.len();

            let now = Instant::now();
            if !keep_alive.is_zero() {
                match ping_sent {
                    Some(t) if now.saturating_duration_since(t) >= keep_alive => {
                        return Err(Dropped::KeepAlive);
                    }
                    None if now.saturating_duration_since(last_sent) >= keep_alive => {
                        Self::send(&mut stream, &Packet::Pingreq)?;
                        last_sent = now;
                        ping_sent = Some(now);
                    }
                    _ => {}
                }
            }

            rx.fill(&mut stream)?;
            while let Some(packet) = rx.take()? {
                match packet {
                    Packet::Publish(p) => {
                        if p.qos == QoS::AtLeastOnce {
                            Self::send(
                                &mut stream,
                                &Packet::Puback {
                                    packet_id: p.packet_id,
                                },
                            )?;
                            last_sent = Instant::now();
                        }
                        (self.handler)(MqttEvent::Message {
                            topic: p.topic,
                            payload: p.payload,
                            retain: p.retain,
                        });
                    }
                    Packet::Puback { packet_id } => {
                        self.inflight.retain(|p| p.packet_id != packet_id);
                    }
                    Packet::Suback {
                        packet_id,
                        return_codes,
                    } => {
                        let pos = self
                            .pending_subacks
                            .iter()
                            .position(|(id, _)| *id == packet_id);
                        if let Some((_, filters)) = pos.and_then(|i| self.pending_subacks.remove(i))
                        {
                            for (filter, code) in filters.into_iter().zip(return_codes) {
                                if code == 0x80 {
                                    (self.handler)(MqttEvent::SubscribeRefused { filter });
                                }
                            }
                        }
                    }
                    Packet::Pingresp => ping_sent = None,
                    Packet::Connack { .. } => return Err(Dropped::Protocol("a second CONNACK")),
                    Packet::Connect(_)
                    | Packet::Subscribe { .. }
                    | Packet::Pingreq
                    | Packet::Disconnect => {
                        return Err(Dropped::Protocol("a client packet from the broker"));
                    }
                }
            }
        }
    }
}

#[cfg(test)]
#[allow(clippy::arithmetic_side_effects)]
mod tests_wp_p2mqc {
    use super::*;

    fn publish(n: u8) -> Publish {
        Publish {
            topic: "t".into(),
            payload: vec![n],
            qos: QoS::AtMostOnce,
            retain: false,
            dup: false,
            packet_id: 0,
        }
    }

    #[test]
    fn wp_p2mqc_queue_drops_the_oldest() {
        let mut q = OutQueue::new(3);
        for n in 0..5 {
            q.push(publish(n));
        }
        let left: Vec<u8> = q.items.iter().map(|p| p.payload[0]).collect();
        assert_eq!(left, vec![2, 3, 4]);
        assert_eq!(q.dropped, 2);
        let mut none = OutQueue::new(0);
        none.push(publish(1));
        assert_eq!((none.items.len(), none.dropped), (0, 1));
    }

    #[test]
    fn wp_p2mqc_backoff_doubles_to_the_cap_and_resets() {
        let s = Duration::from_secs;
        let mut b = Backoff::new(s(1), s(5));
        let got: Vec<Duration> = (0..5).map(|_| b.next_delay()).collect();
        assert_eq!(got, vec![s(1), s(2), s(4), s(5), s(5)]);
        b.reset();
        assert_eq!(b.next_delay(), s(1));
    }

    #[test]
    fn wp_p2mqc_publish_len_matches_encode() {
        for (topic_len, payload_len, qos) in [
            (1usize, 0usize, QoS::AtMostOnce),
            (3, 122, QoS::AtMostOnce),
            (3, 123, QoS::AtMostOnce),
            (10, 16_370, QoS::AtLeastOnce),
            (10, 16_371, QoS::AtLeastOnce),
            (5, 2_097_200, QoS::AtMostOnce),
        ] {
            let p = Publish {
                topic: "x".repeat(topic_len),
                payload: vec![0; payload_len],
                qos,
                retain: false,
                dup: false,
                packet_id: if qos == QoS::AtLeastOnce { 1 } else { 0 },
            };
            assert_eq!(
                publish_len(&p),
                codec::encode(&Packet::Publish(p.clone())).unwrap().len()
            );
        }
    }

    #[test]
    fn wp_p2mqc_config_debug_hides_password() {
        let mut c = MqttConfig::new("broker", DEFAULT_MQTT_PORT, "hub");
        c.username = Some("u".into());
        c.password = Some("hunter2".into());
        let d = format!("{c:?}");
        assert!(!d.contains("hunter2"));
        assert!(d.contains("\"set\""));
    }

    #[test]
    fn wp_p2mqc_start_refuses_bad_config() {
        let mut c = MqttConfig::new("127.0.0.1", 1, "hub");
        c.password = Some("p".into());
        assert!(matches!(
            MqttClient::start(c, |_| {}),
            Err(MqttError::Config(_))
        ));
        let mut c = MqttConfig::new("127.0.0.1", 1, "hub");
        c.keep_alive = Duration::from_secs(70_000);
        assert!(matches!(
            MqttClient::start(c, |_| {}),
            Err(MqttError::Config(_))
        ));
    }
}
