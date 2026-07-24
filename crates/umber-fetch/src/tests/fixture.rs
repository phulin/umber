//! Cross-platform HTTP fixture transport that avoids opening sockets.
//!
//! Keep the dependency on ureq's unversioned connector API isolated here so
//! production acquisition continues to use ureq's default TCP/TLS transport.

use std::collections::{BTreeMap, VecDeque};
use std::fmt;
use std::io;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, SyncSender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use ureq::unversioned::resolver::DefaultResolver;
use ureq::unversioned::transport::{
    Buffers, ConnectionDetails, Connector, LazyBuffers, NextTimeout, Transport,
};
use ureq::{Agent, Error};

use crate::FetchCancellation;
use crate::fetch::agent_config;

#[derive(Clone, Debug)]
pub(super) struct Reply {
    pub(super) status: u16,
    pub(super) body: Vec<u8>,
    pub(super) content_length: Option<u64>,
    pub(super) delay: Duration,
    pub(super) wait_for_cancellation: Option<FetchCancellation>,
}

impl Reply {
    pub(super) fn ok(body: &[u8]) -> Self {
        Self {
            status: 200,
            body: body.to_vec(),
            content_length: Some(body.len() as u64),
            delay: Duration::ZERO,
            wait_for_cancellation: None,
        }
    }
}

enum Replies {
    Sequential(Mutex<VecDeque<Reply>>),
    Routed(BTreeMap<String, Reply>),
}

struct FixtureState {
    replies: Replies,
    requests: Arc<AtomicUsize>,
    active: Arc<AtomicUsize>,
    maximum_active: Arc<AtomicUsize>,
    handlers: Mutex<Vec<thread::JoinHandle<()>>>,
}

#[derive(Clone)]
struct FixtureConnector {
    state: Arc<FixtureState>,
}

impl fmt::Debug for FixtureConnector {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("FixtureConnector")
    }
}

impl Connector for FixtureConnector {
    type Out = InMemoryTransport;

    fn connect(
        &self,
        details: &ConnectionDetails<'_>,
        chained: Option<()>,
    ) -> Result<Option<Self::Out>, Error> {
        debug_assert!(chained.is_none());
        let reply = match &self.state.replies {
            Replies::Sequential(replies) => replies
                .lock()
                .expect("fixture reply mutex poisoned")
                .pop_front()
                .expect("fixture has a reply for every request"),
            Replies::Routed(replies) => {
                let object = details
                    .uri
                    .path()
                    .rsplit('/')
                    .next()
                    .expect("fixture request object");
                replies
                    .get(object)
                    .unwrap_or_else(|| panic!("fixture has a reply for {object}"))
                    .clone()
            }
        };
        let (request_sender, request_receiver) = mpsc::sync_channel(10);
        let (response_sender, response_receiver) = mpsc::sync_channel(10);
        self.state.requests.fetch_add(1, Ordering::SeqCst);
        let active = Arc::clone(&self.state.active);
        let maximum_active = Arc::clone(&self.state.maximum_active);
        let handler = thread::spawn(move || {
            serve(
                request_receiver,
                response_sender,
                reply,
                active,
                maximum_active,
            )
        });
        self.state
            .handlers
            .lock()
            .expect("fixture handler mutex poisoned")
            .push(handler);
        let config = details.config;
        let buffers = LazyBuffers::new(config.input_buffer_size(), config.output_buffer_size());
        Ok(Some(InMemoryTransport::new(
            request_sender,
            response_receiver,
            buffers,
        )))
    }
}

pub(super) struct FixtureServer {
    pub(super) base_url: String,
    pub(super) requests: Arc<AtomicUsize>,
    maximum_active: Arc<AtomicUsize>,
    state: Arc<FixtureState>,
}

impl FixtureServer {
    pub(super) fn new(replies: Vec<Reply>) -> Self {
        Self::with_replies(Replies::Sequential(Mutex::new(replies.into())))
    }

    pub(super) fn routed(replies: BTreeMap<String, Reply>) -> Self {
        Self::with_replies(Replies::Routed(replies))
    }

    fn with_replies(replies: Replies) -> Self {
        let requests = Arc::new(AtomicUsize::new(0));
        let maximum_active = Arc::new(AtomicUsize::new(0));
        let state = Arc::new(FixtureState {
            replies,
            requests: Arc::clone(&requests),
            active: Arc::new(AtomicUsize::new(0)),
            maximum_active: Arc::clone(&maximum_active),
            handlers: Mutex::new(Vec::new()),
        });
        Self {
            base_url: "http://127.0.0.1:1/objects/".into(),
            requests,
            maximum_active,
            state,
        }
    }

    pub(super) fn agent(&self, timeout: Duration) -> Agent {
        Agent::with_parts(
            agent_config(timeout),
            FixtureConnector {
                state: Arc::clone(&self.state),
            },
            DefaultResolver::default(),
        )
    }

    pub(super) fn finish(self) -> (usize, usize) {
        self.join_handlers();
        (
            self.requests.load(Ordering::SeqCst),
            self.maximum_active.load(Ordering::SeqCst),
        )
    }

    fn join_handlers(&self) {
        let handlers = std::mem::take(
            &mut *self
                .state
                .handlers
                .lock()
                .expect("fixture handler mutex poisoned"),
        );
        for handler in handlers {
            handler.join().expect("fixture connection handler");
        }
    }
}

impl Drop for FixtureServer {
    fn drop(&mut self) {
        self.join_handlers();
    }
}

fn serve(
    requests: Receiver<Vec<u8>>,
    responses: SyncSender<Vec<u8>>,
    reply: Reply,
    active_connections: Arc<AtomicUsize>,
    maximum_active: Arc<AtomicUsize>,
) {
    let active = active_connections.fetch_add(1, Ordering::SeqCst) + 1;
    maximum_active.fetch_max(active, Ordering::SeqCst);
    let mut request = Vec::new();
    while !request.windows(4).any(|window| window == b"\r\n\r\n") {
        request.extend(requests.recv().expect("receive in-memory fixture request"));
    }
    if let Some(cancellation) = &reply.wait_for_cancellation {
        for _ in 0..500 {
            if cancellation.is_cancelled() {
                break;
            }
            thread::sleep(Duration::from_millis(10));
        }
        assert!(
            cancellation.is_cancelled(),
            "fixture cancellation was not published within five seconds"
        );
    }
    thread::sleep(reply.delay);
    let reason = if reply.status == 200 {
        "OK"
    } else {
        "Not Found"
    };
    let length = reply.content_length.unwrap_or(reply.body.len() as u64);
    let mut response = format!(
        "HTTP/1.1 {} {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        reply.status, reason, length
    )
    .into_bytes();
    response.extend_from_slice(&reply.body);
    let _ = responses.send(response);
    active_connections.fetch_sub(1, Ordering::SeqCst);
}

#[derive(Debug)]
struct InMemoryTransport {
    requests: SyncSender<Vec<u8>>,
    responses: Mutex<Receiver<Vec<u8>>>,
    pending_input: VecDeque<u8>,
    buffers: LazyBuffers,
}

impl InMemoryTransport {
    fn new(
        requests: SyncSender<Vec<u8>>,
        responses: Receiver<Vec<u8>>,
        buffers: LazyBuffers,
    ) -> Self {
        Self {
            requests,
            responses: Mutex::new(responses),
            pending_input: VecDeque::new(),
            buffers,
        }
    }
}

impl Transport for InMemoryTransport {
    fn buffers(&mut self) -> &mut dyn Buffers {
        &mut self.buffers
    }

    fn transmit_output(&mut self, amount: usize, _timeout: NextTimeout) -> Result<(), Error> {
        let output = &self.buffers.output()[..amount];
        self.requests.send(output.to_vec()).map_err(|_| {
            io::Error::new(io::ErrorKind::BrokenPipe, "fixture request receiver closed").into()
        })
    }

    fn await_input(&mut self, timeout: NextTimeout) -> Result<bool, Error> {
        if self.pending_input.is_empty() {
            let received = if let Some(duration) = timeout.not_zero() {
                self.responses
                    .lock()
                    .expect("fixture response mutex poisoned")
                    .recv_timeout(*duration)
            } else {
                self.responses
                    .lock()
                    .expect("fixture response mutex poisoned")
                    .recv()
                    .map_err(|_| RecvTimeoutError::Disconnected)
            };
            match received {
                Ok(bytes) => self.pending_input.extend(bytes),
                Err(RecvTimeoutError::Timeout) => return Err(Error::Timeout(timeout.reason)),
                Err(RecvTimeoutError::Disconnected) => {
                    self.buffers.input_appended(0);
                    return Ok(false);
                }
            }
        }
        let input = self.buffers.input_append_buf();
        let amount = input.len().min(self.pending_input.len());
        for (destination, byte) in input[..amount]
            .iter_mut()
            .zip(self.pending_input.drain(..amount))
        {
            *destination = byte;
        }
        self.buffers.input_appended(amount);
        Ok(amount > 0)
    }

    fn is_open(&mut self) -> bool {
        false
    }
}
