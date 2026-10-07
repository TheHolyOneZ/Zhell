mod blocks;
mod cast;
pub mod headless;
pub mod history;
mod integration;
pub mod ipc;
mod pane;
mod folds;
mod images;
mod ports;
pub mod snapshot;

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Arc, OnceLock};
use std::thread;
use std::time::{Duration, Instant};

use alacritty_terminal::grid::Scroll;
use crossbeam_channel::{Receiver, Sender, unbounded};
use parking_lot::Mutex;
use zhell_core::{SessionHost, Waker};
use zhell_proto::{ClientMsg, HostOptions, PROTO_VERSION, PaneId, Restore, ServerMsg, SpawnSpec};

use pane::Pane;

pub type ClientId = u64;

pub enum Event {
    Connected(ClientId, Outbox),
    Client(ClientId, ClientMsg),
    Disconnected(ClientId),

    Dirty(PaneId),
    Exited(PaneId, Option<i32>),

    SaveSnapshot,

    Abort,
}

#[derive(Clone)]
pub struct Outbox {
    tx: Sender<ServerMsg>,
    waker: Arc<OnceLock<Waker>>,
}

impl Outbox {
    pub fn new(tx: Sender<ServerMsg>) -> Self {
        Self { tx, waker: Arc::new(OnceLock::new()) }
    }

    pub(crate) fn send(&self, msg: ServerMsg) {
        if self.tx.send(msg).is_ok()
            && let Some(w) = self.waker.get()
        {
            w();
        }
    }
}

#[derive(Clone, Default)]
pub(crate) struct Sink(Arc<Mutex<Option<Outbox>>>);

impl Sink {
    pub(crate) fn send(&self, msg: ServerMsg) {
        if let Some(out) = &*self.0.lock() {
            out.send(msg);
        }
    }

    fn set(&self, out: Option<Outbox>) {
        *self.0.lock() = out;
    }
}

pub const QUAKE_CLIENT: &str = "zhell-quake";

struct Client {
    out: Outbox,

    name: String,
    layout: Vec<u8>,

    greeted: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OnDisconnect {
    Kill,

    Keep,
}

pub struct Server {
    panes: BTreeMap<PaneId, Pane>,
    owner: BTreeMap<PaneId, ClientId>,
    clients: BTreeMap<ClientId, Client>,

    orphans: Vec<Restore>,
    next_id: u64,
    options: HostOptions,
    on_disconnect: OnDisconnect,
    events: Sender<Event>,

    history: Option<Sender<history::HistoryCmd>>,
    history_settings: Option<Arc<history::HistorySettings>>,

    snapshot_path: Option<PathBuf>,

    snapshot_dirty: bool,
    last_snapshot: Instant,

    background_sent: Option<u32>,
    background_clients: usize,
    last_port_scan: Instant,
}

const SNAPSHOT_MIN_INTERVAL: Duration = Duration::from_secs(5);
const SNAPSHOT_MAX_INTERVAL: Duration = Duration::from_secs(30);

const PORT_SCAN_INTERVAL: Duration = Duration::from_secs(2);

impl Server {
    pub fn new(events: Sender<Event>, on_disconnect: OnDisconnect) -> Self {
        Self {
            panes: BTreeMap::new(),
            owner: BTreeMap::new(),
            clients: BTreeMap::new(),
            orphans: Vec::new(),
            next_id: 1,
            options: HostOptions::default(),
            on_disconnect,
            events,
            history: None,
            history_settings: None,
            snapshot_path: None,
            snapshot_dirty: false,
            last_snapshot: Instant::now(),
            background_sent: None,
            background_clients: 0,
            last_port_scan: Instant::now(),
        }
    }

    pub fn with_history(mut self, settings: history::HistorySettings) -> Self {
        self.history = history::start(settings.clone());
        if self.history.is_some() {
            self.history_settings = Some(Arc::new(settings));
        }
        self
    }

    fn recorder(&self) -> Option<pane::Recorder> {
        Some(pane::Recorder { tx: self.history.clone()?, settings: self.history_settings.clone()?, host: Arc::new(history::hostname()) })
    }

    pub fn with_snapshots(mut self, path: PathBuf) -> Self {
        self.snapshot_path = Some(path);
        self
    }

    pub fn restore_snapshot(&mut self, spawn: &SpawnSpec) {
        let Some(path) = self.snapshot_path.clone() else { return };
        let Some(snap) = snapshot::load(&path) else { return };
        let banner = snapshot::restored_banner(snap.saved_at);
        for group in snap.groups {
            let mut restored = Vec::new();
            for p in group.panes {
                let mut text = p.lines.join("\r\n");
                if !text.is_empty() {
                    text.push_str("\r\n");
                }
                text.push_str(&banner);
                text.push_str("\r\n");
                let spec = SpawnSpec { cwd: p.cwd.clone().filter(|d| std::path::Path::new(d).is_dir()), ..spawn.clone() };
                let ctx = pane::PaneCtx { options: self.options, sink: Sink::default(), events: self.events.clone(), recorder: self.recorder() };
                match Pane::spawn(p.id, &spec, p.size, ctx, Some(&text)) {
                    Ok(pane) => {
                        self.next_id = self.next_id.max(p.id.0 + 1);
                        self.panes.insert(p.id, pane);
                        restored.push(p.id);
                    }
                    Err(e) => log::warn!("restoring pane {}: {e:#}", p.id.0),
                }
            }
            if !restored.is_empty() {
                self.orphans.push(Restore { layout: group.layout, panes: restored });
            }
        }
        log::info!("restored {} session(s) from {}", self.panes.len(), path.display());
    }

    fn save_snapshot(&mut self) {
        let Some(path) = &self.snapshot_path else { return };
        self.snapshot_dirty = false;
        self.last_snapshot = Instant::now();
        if self.panes.is_empty() {
            snapshot::remove(path);
            return;
        }
        let mut groups: Vec<(Vec<u8>, Vec<PaneId>)> = Vec::new();
        for (id, c) in &self.clients {
            let panes: Vec<PaneId> = self.owner.iter().filter(|(_, o)| *o == id).map(|(p, _)| *p).collect();
            if !panes.is_empty() {
                groups.push((c.layout.clone(), panes));
            }
        }
        for o in &self.orphans {
            groups.push((o.layout.clone(), o.panes.clone()));
        }
        let covered: Vec<PaneId> = groups.iter().flat_map(|g| g.1.iter().copied()).collect();
        let loose: Vec<PaneId> = self.panes.keys().filter(|p| !covered.contains(p)).copied().collect();
        if !loose.is_empty() {
            groups.push((Vec::new(), loose));
        }
        let groups = groups
            .into_iter()
            .map(|(layout, panes)| snapshot::Group {
                layout,
                panes: panes.iter().filter_map(|p| self.panes.get(p)).map(Pane::snapshot).collect(),
            })
            .collect();
        if let Err(e) = snapshot::save(path, &snapshot::Snapshot::new(groups)) {
            log::warn!("saving session snapshot: {e}");
        }
    }

    pub fn run(mut self, events: Receiver<Event>) {
        let mut ever_connected = !self.panes.is_empty();
        let mut shutdown = false;
        loop {
            if self.last_port_scan.elapsed() >= PORT_SCAN_INTERVAL {
                self.scan_ports();
            }
            let ev = match events.recv_timeout(PORT_SCAN_INTERVAL) {
                Ok(ev) => ev,
                Err(crossbeam_channel::RecvTimeoutError::Timeout) => {
                    let since = self.last_snapshot.elapsed();
                    if (self.snapshot_dirty && since >= SNAPSHOT_MIN_INTERVAL) || since >= SNAPSHOT_MAX_INTERVAL {
                        self.save_snapshot();
                    }
                    continue;
                }
                Err(crossbeam_channel::RecvTimeoutError::Disconnected) => break,
            };
            match ev {
                Event::Connected(id, out) => {
                    ever_connected = true;
                    self.clients.insert(id, Client { out, name: String::new(), layout: Vec::new(), greeted: false });
                }
                Event::Client(id, ClientMsg::Shutdown) => {
                    log::info!("client {id} requested shutdown");
                    shutdown = true;
                    break;
                }
                Event::Client(id, ClientMsg::Bye) => self.disconnected(id),
                Event::Client(id, msg) => self.handle_client(id, msg),
                Event::Disconnected(id) => self.disconnected(id),
                Event::Dirty(pane) => {
                    if let Some(p) = self.panes.get_mut(&pane) {
                        p.dirty = true;
                    }
                    self.flush(pane);
                }
                Event::Exited(pane, code) => self.exited(pane, code),
                Event::SaveSnapshot => self.save_snapshot(),
                Event::Abort => return,
            }
            self.announce_background();
            if ever_connected && self.clients.is_empty() && self.panes.is_empty() {
                log::info!("no clients and no sessions left; stopping");
                break;
            }
        }

        if shutdown || self.panes.is_empty() {
            if let Some(path) = &self.snapshot_path {
                snapshot::remove(path);
            }
        } else {
            self.save_snapshot();
        }
        for p in self.panes.values_mut() {
            p.kill();
        }
    }

    fn scan_ports(&mut self) {
        self.last_port_scan = Instant::now();
        let sessions: Vec<u32> = self.panes.values().filter_map(|p| p.shell_pid).collect();
        let found = ports::listening_by_session(&sessions);
        for p in self.panes.values_mut() {
            let now = p.shell_pid.and_then(|sid| found.get(&sid)).cloned().unwrap_or_default();
            if ports::ports(&now) != ports::ports(&p.ports) {
                p.sink.send(ServerMsg::Ports { pane: p.id, ports: now.keys().copied().collect() });
            }
            p.ports = now;
        }
    }

    fn announce_background(&mut self) {
        let count = self.panes.keys().filter(|p| !self.owner.contains_key(p)).count() as u32;
        let greeted = self.clients.values().filter(|c| c.greeted).count();
        if self.background_sent == Some(count) && greeted == self.background_clients {
            return;
        }
        self.background_sent = Some(count);
        self.background_clients = greeted;
        for c in self.clients.values().filter(|c| c.greeted) {
            c.out.send(ServerMsg::Background { count });
        }
    }

    fn reply(&self, client: ClientId, msg: ServerMsg) {
        if let Some(c) = self.clients.get(&client) {
            c.out.send(msg);
        }
    }

    fn disconnected(&mut self, client: ClientId) {
        let Some(c) = self.clients.remove(&client) else { return };
        let panes: Vec<PaneId> = self.owner.iter().filter(|(_, c)| **c == client).map(|(p, _)| *p).collect();
        for pane in &panes {
            self.owner.remove(pane);
            if let Some(p) = self.panes.get_mut(pane) {
                p.sink.set(None);
                p.inflight = None;
                if self.on_disconnect == OnDisconnect::Kill {
                    p.kill();
                }
            }
        }
        if self.on_disconnect == OnDisconnect::Keep && !panes.is_empty() {
            log::info!("client {client} left; keeping {} session(s)", panes.len());
            self.orphans.push(Restore { layout: c.layout, panes });
        }
    }

    fn exited(&mut self, pane: PaneId, code: Option<i32>) {
        if let Some(mut p) = self.panes.remove(&pane) {
            if p.dirty {
                let frame = p.take_frame(false);
                p.sink.send(ServerMsg::Frame(frame));
            }
            p.sink.send(ServerMsg::Exited { pane, code });
        }
        self.owner.remove(&pane);
        for o in &mut self.orphans {
            o.panes.retain(|p| *p != pane);
        }
        self.orphans.retain(|o| !o.panes.is_empty());
        self.snapshot_dirty = true;
    }

    fn take_restore(&mut self) -> Option<Restore> {
        let mut restore = self.orphans.pop().unwrap_or(Restore { layout: Vec::new(), panes: Vec::new() });
        let claimed: Vec<PaneId> = self.orphans.iter().flat_map(|o| o.panes.iter().copied()).collect();
        for pane in self.panes.keys() {
            if !self.owner.contains_key(pane) && !claimed.contains(pane) && !restore.panes.contains(pane) {
                restore.panes.push(*pane);
            }
        }
        restore.panes.retain(|p| self.panes.contains_key(p));
        (!restore.panes.is_empty()).then_some(restore)
    }

    fn attach(&mut self, client: ClientId, pane: PaneId) {
        let Some(out) = self.clients.get(&client).map(|c| c.out.clone()) else { return };
        let Some(p) = self.panes.get_mut(&pane) else {
            out.send(ServerMsg::Exited { pane, code: None });
            return;
        };

        if let Some(prev) = self.owner.insert(pane, client).filter(|c| *c != client)
            && let Some(c) = self.clients.get(&prev)
        {
            c.out.send(ServerMsg::Detached { pane });
        }
        for o in &mut self.orphans {
            o.panes.retain(|p| *p != pane);
        }
        self.orphans.retain(|o| !o.panes.is_empty());
        p.sink.set(Some(out.clone()));
        p.inflight = None;
        if !p.ports.is_empty() {
            out.send(ServerMsg::Ports { pane, ports: p.ports.keys().copied().collect() });
        }
        for (id, img) in p.images.lock().images.iter().filter(|(_, i)| i.cols > 0) {
            out.send(pane::image_msg(pane, *id, img));
        }
        out.send(ServerMsg::Title { pane, title: p.title.lock().clone() });
        if let Some(cwd) = p.cwd() {
            out.send(ServerMsg::Cwd { pane, cwd });
        }
        let frame = p.take_frame(true);
        p.inflight = Some(frame.seq);
        out.send(ServerMsg::Frame(frame));
    }

    fn handle_client(&mut self, client: ClientId, msg: ClientMsg) {
        match msg {
            ClientMsg::Hello { proto_version, client_name, restore } => {
                log::info!("client {client} ({client_name}) connected");
                let reply = if proto_version == PROTO_VERSION {
                    ServerMsg::HelloOk {
                        proto_version: PROTO_VERSION,
                        host_version: env!("CARGO_PKG_VERSION").into(),
                        restore: if restore { self.take_restore() } else { None },
                    }
                } else {
                    ServerMsg::VersionMismatch { host_proto_version: PROTO_VERSION }
                };
                self.reply(client, reply);
                if let Some(c) = self.clients.get_mut(&client) {
                    c.greeted = true;
                    c.name = client_name;
                }
            }
            ClientMsg::CreatePane { req, spawn, size } => {
                let id = PaneId(self.next_id);
                self.next_id += 1;
                let sink = Sink::default();
                sink.set(self.clients.get(&client).map(|c| c.out.clone()));
                let ctx = pane::PaneCtx { options: self.options, sink, events: self.events.clone(), recorder: self.recorder() };
                match Pane::spawn(id, &spawn, size, ctx, None) {
                    Ok(p) => {
                        self.panes.insert(id, p);
                        self.owner.insert(id, client);
                        self.reply(client, ServerMsg::PaneCreated { req, pane: id });
                        self.flush(id);
                    }
                    Err(e) => self.reply(client, ServerMsg::SpawnFailed { req, error: format!("{e:#}") }),
                }
            }
            ClientMsg::Attach { pane } => self.attach(client, pane),
            ClientMsg::StoreLayout(layout) => {
                if let Some(c) = self.clients.get_mut(&client) {
                    c.layout = layout;
                }
                self.snapshot_dirty = true;
            }
            ClientMsg::Input { pane, bytes } => {
                if let Some(p) = self.panes.get(&pane) {
                    p.write(&bytes);
                }
            }
            ClientMsg::Resize { pane, size } => self.with_pane(pane, |p| p.resize(size)),
            ClientMsg::Scroll { pane, delta } => self.with_pane(pane, |p| p.scroll(Scroll::Delta(delta))),
            ClientMsg::ScrollToBottom { pane } => self.with_pane(pane, |p| p.scroll(Scroll::Bottom)),
            ClientMsg::SetOptions(options) => {
                self.options = options;
                let ids: Vec<_> = self.panes.keys().copied().collect();
                for id in ids {
                    self.with_pane(id, |p| p.set_options(options));
                }
            }
            ClientMsg::Focus { pane, focused } => self.with_pane(pane, |p| p.set_focus(focused)),
            ClientMsg::Ack { pane, seq } => {
                if let Some(p) = self.panes.get_mut(&pane)
                    && p.inflight == Some(seq)
                {
                    p.inflight = None;
                }
                self.flush(pane);
            }
            ClientMsg::SelectStart { pane, row, col, right_half, kind } => {
                self.with_pane(pane, |p| p.select_start(row, col, right_half, kind))
            }
            ClientMsg::SelectUpdate { pane, row, col, right_half } => {
                self.with_pane(pane, |p| p.select_update(row, col, right_half))
            }
            ClientMsg::SelectClear { pane } => self.with_pane(pane, Pane::select_clear),
            ClientMsg::ResolvePath { pane, req, path } => {
                let path = self.panes.get(&pane).and_then(|p| p.resolve_path(&path));
                self.reply(client, ServerMsg::PathResolved { req, path });
            }
            ClientMsg::Find { pane, query, regex } => self.with_pane(pane, |p| p.find(&query, regex)),
            ClientMsg::FindNext { pane, older } => self.with_pane(pane, |p| p.find_next(older)),
            ClientMsg::FindClose { pane } => self.with_pane(pane, Pane::find_close),
            ClientMsg::Fold { pane, block, folded } => self.with_pane(pane, |p| p.set_fold(block, folded)),
            ClientMsg::CopyBlockOutput { pane, block, target } => {
                if let Some(text) = self.panes.get(&pane).and_then(|p| p.block_output(block)) {
                    self.reply(client, ServerMsg::CopyText { pane, text, target });
                }
            }
            ClientMsg::JumpBlock { pane, forward } => self.with_pane(pane, |p| p.jump_block(forward)),
            ClientMsg::StopPort { pane, port } => {
                if let Some(pid) = self.panes.get(&pane).and_then(|p| p.ports.get(&port).copied()) {
                    log::info!("stopping pid {pid} listening on port {port}");
                    ports::terminate(pid);
                }
                self.last_port_scan = Instant::now() - PORT_SCAN_INTERVAL;
            }
            ClientMsg::QueryForeground { req, pane } => {
                let name = self.panes.get(&pane).and_then(Pane::foreground_name);
                self.reply(client, ServerMsg::Foreground { req, name });
            }
            ClientMsg::HistorySearch { req, query } => match (&self.history, self.clients.get(&client)) {
                (Some(h), Some(c)) => {
                    let _ = h.send(history::HistoryCmd::Search { req, query, reply: c.out.clone() });
                }
                _ => self.reply(client, ServerMsg::HistoryResults { req, hits: Vec::new() }),
            },
            ClientMsg::HistoryGet { req, id } => match (&self.history, self.clients.get(&client)) {
                (Some(h), Some(c)) => {
                    let _ = h.send(history::HistoryCmd::Get { req, id, reply: c.out.clone() });
                }
                _ => self.reply(client, ServerMsg::HistoryEntry { req, entry: None }),
            },
            ClientMsg::HistoryStar { id, starred } => {
                if let Some(h) = &self.history {
                    let _ = h.send(history::HistoryCmd::Star { id, starred });
                }
            }
            ClientMsg::HistoryNote { id, note } => {
                if let Some(h) = &self.history {
                    let _ = h.send(history::HistoryCmd::Note { id, note });
                }
            }
            ClientMsg::HistoryTemplate { id, template } => {
                if let Some(h) = &self.history {
                    let _ = h.send(history::HistoryCmd::Template { id, template });
                }
            }
            ClientMsg::HistoryForget { id } => {
                if let Some(h) = &self.history {
                    let _ = h.send(history::HistoryCmd::Forget { id });
                }
            }
            ClientMsg::Copy { pane, target } => {
                if let Some(text) = self.panes.get(&pane).and_then(Pane::selection_text) {
                    self.reply(client, ServerMsg::CopyText { pane, text, target });
                }
            }
            ClientMsg::ToggleQuake => {
                let quake = self.clients.values().find(|c| c.greeted && c.name == QUAKE_CLIENT);
                if let Some(q) = quake {
                    q.out.send(ServerMsg::Quake);
                }
                self.reply(client, ServerMsg::QuakeHandled { handled: quake.is_some() });
            }
            ClientMsg::CopyMode { pane, cmd } => {
                let text = self.panes.get_mut(&pane).and_then(|p| p.copy_mode(cmd));
                self.flush(pane);
                if let Some(text) = text {
                    self.reply(client, ServerMsg::CopyText { pane, text, target: zhell_proto::CopyTarget::Clipboard });
                }
            }
            ClientMsg::BlockCells { req, pane, block } => {
                let rows = self.panes.get(&pane).and_then(|p| p.block_cells(block)).unwrap_or_default();
                self.reply(client, ServerMsg::BlockCells { req, rows });
            }
            ClientMsg::Record { pane, path } => {
                let Some(p) = self.panes.get(&pane) else { return };
                let reply = match path {
                    Some(path) => match p.start_recording(std::path::Path::new(&path)) {
                        Ok(()) => ServerMsg::Recording { pane, path: Some(path), done: false, error: None },
                        Err(e) => ServerMsg::Recording { pane, path: None, done: false, error: Some(format!("{path}: {e}")) },
                    },
                    None => ServerMsg::Recording { pane, path: p.stop_recording().map(|p| p.display().to_string()), done: true, error: None },
                };
                self.reply(client, reply);
            }
            ClientMsg::ClosePane { pane } => {
                if let Some(p) = self.panes.get_mut(&pane) {
                    p.kill();
                }
            }
            ClientMsg::Bye | ClientMsg::Shutdown => {}
        }
    }

    fn with_pane(&mut self, id: PaneId, f: impl FnOnce(&mut Pane)) {
        if let Some(p) = self.panes.get_mut(&id) {
            f(p);
        }
        self.flush(id);
    }

    fn flush(&mut self, id: PaneId) {
        if !self.owner.contains_key(&id) {
            return;
        }
        let Some(p) = self.panes.get_mut(&id) else { return };
        if !p.dirty || p.inflight.is_some() {
            return;
        }
        let frame = p.take_frame(false);
        p.inflight = Some(frame.seq);
        p.sink.send(ServerMsg::Frame(frame));
    }
}

pub struct InProcHost {
    events: Sender<Event>,
    server_rx: Receiver<ServerMsg>,
    waker: Arc<OnceLock<Waker>>,
}

impl InProcHost {
    pub fn start() -> std::io::Result<Self> {
        Self::start_with(None)
    }

    pub fn start_with(history: Option<history::HistorySettings>) -> std::io::Result<Self> {
        let (events, events_rx) = unbounded();
        let (server_tx, server_rx) = unbounded();
        let out = Outbox::new(server_tx);
        let waker = out.waker.clone();
        let mut server = Server::new(events.clone(), OnDisconnect::Kill);
        if let Some(h) = history {
            server = server.with_history(h);
        }
        thread::Builder::new().name("zhell-host".into()).spawn(move || server.run(events_rx))?;
        let _ = events.send(Event::Connected(0, out));
        Ok(Self { events, server_rx, waker })
    }

    pub fn recv_timeout(&self, t: std::time::Duration) -> Option<ServerMsg> {
        self.server_rx.recv_timeout(t).ok()
    }
}

impl SessionHost for InProcHost {
    fn send(&self, msg: ClientMsg) {
        let _ = self.events.send(Event::Client(0, msg));
    }

    fn try_recv(&self) -> Option<ServerMsg> {
        self.server_rx.try_recv().ok()
    }

    fn set_waker(&mut self, waker: Waker) {
        let _ = self.waker.set(waker);
    }
}

impl Drop for InProcHost {
    fn drop(&mut self) {
        let _ = self.events.send(Event::Client(0, ClientMsg::Shutdown));
    }
}
