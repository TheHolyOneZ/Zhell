use std::io::{self, BufReader, BufWriter, Write};
#[cfg(unix)]
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};
use std::thread;
use std::time::{Duration, Instant};

use crossbeam_channel::{Receiver, Sender, unbounded};
use interprocess::local_socket::prelude::*;
use interprocess::local_socket::{ListenerOptions, Stream};
use parking_lot::Mutex;
use zhell_core::{SessionHost, Waker};
use zhell_proto::{ClientMsg, PROTO_VERSION, ServerMsg, read_frame, write_frame};

use crate::{ClientId, Event, Outbox};

#[cfg(unix)]
pub fn runtime_dir() -> PathBuf {
    match std::env::var_os("XDG_RUNTIME_DIR") {
        Some(d) if !d.is_empty() => PathBuf::from(d).join("zhell"),
        _ => {
            let user = std::env::var("USER").unwrap_or_else(|_| "user".into());
            std::env::temp_dir().join(format!("zhell-{user}"))
        }
    }
}

pub fn socket_name() -> String {
    if let Ok(name) = std::env::var("ZHELL_SOCKET") {
        return name;
    }
    #[cfg(unix)]
    {
        runtime_dir().join(format!("zhelld-v{PROTO_VERSION}.sock")).display().to_string()
    }
    #[cfg(windows)]
    {
        let user = std::env::var("USERNAME").unwrap_or_else(|_| "user".into());
        format!("zhelld-v{PROTO_VERSION}-{user}")
    }
}

fn to_name(name: &str) -> io::Result<interprocess::local_socket::Name<'_>> {
    #[cfg(unix)]
    {
        name.to_fs_name::<interprocess::local_socket::GenericFilePath>()
    }
    #[cfg(windows)]
    {
        name.to_ns_name::<interprocess::local_socket::GenericNamespaced>()
    }
}

#[cfg(target_os = "linux")]
fn current_uid() -> Option<u32> {
    use std::os::unix::fs::MetadataExt;
    std::fs::metadata("/proc/self").ok().map(|m| m.uid())
}

#[cfg(unix)]
fn prepare_dir(socket: &str) -> io::Result<()> {
    use std::os::unix::fs::{DirBuilderExt, MetadataExt, PermissionsExt};
    let Some(dir) = std::path::Path::new(socket).parent() else { return Ok(()) };
    std::fs::DirBuilder::new().recursive(true).mode(0o700).create(dir)?;
    let meta = std::fs::metadata(dir)?;
    #[cfg(target_os = "linux")]
    if current_uid().is_some_and(|uid| uid != meta.uid()) {
        return Err(io::Error::new(io::ErrorKind::PermissionDenied, format!("{} is owned by another user", dir.display())));
    }
    if meta.mode() & 0o077 != 0 {
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

pub fn listen(name: &str) -> io::Result<interprocess::local_socket::Listener> {
    #[cfg(unix)]
    {
        use interprocess::os::unix::local_socket::ListenerOptionsExt;
        prepare_dir(name)?;

        if std::path::Path::new(name).exists() && Stream::connect(to_name(name)?).is_err() {
            let _ = std::fs::remove_file(name);
        }
        ListenerOptions::new().name(to_name(name)?).mode(0o600).create_sync()
    }
    #[cfg(windows)]
    {
        use interprocess::os::windows::local_socket::ListenerOptionsExt;
        use interprocess::os::windows::security_descriptor::SecurityDescriptor;

        let sddl: Vec<u16> = "D:P(A;;GA;;;OW)(A;;GA;;;SY)".encode_utf16().chain(std::iter::once(0)).collect();
        let sddl = widestring::U16CStr::from_slice_truncate(&sddl)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e))?;
        let sd = SecurityDescriptor::deserialize(sddl)?;
        ListenerOptions::new().name(to_name(name)?).security_descriptor(sd).create_sync()
    }
}

pub fn serve(listener: interprocess::local_socket::Listener, events: Sender<Event>) {
    let mut next_id: ClientId = 1;
    for conn in listener.incoming() {
        let conn = match conn {
            Ok(c) => c,
            Err(e) => {
                log::warn!("accept: {e}");
                continue;
            }
        };
        #[cfg(target_os = "linux")]
        {
            let peer = conn.peer_creds().ok().and_then(|c| c.euid());
            if peer != current_uid() {
                log::warn!("rejected connection from uid {peer:?}");
                continue;
            }
        }
        let id = next_id;
        next_id += 1;
        if let Err(e) = start_client(id, conn, events.clone()) {
            log::warn!("client {id}: {e}");
        }
    }
}

fn start_client(id: ClientId, conn: Stream, events: Sender<Event>) -> io::Result<()> {
    let (recv, send) = conn.split();
    let (tx, rx) = unbounded::<ServerMsg>();
    let _ = events.send(Event::Connected(id, Outbox::new(tx)));

    thread::Builder::new().name(format!("client-{id}-write")).spawn(move || {
        let mut w = BufWriter::with_capacity(256 * 1024, send);
        while let Ok(msg) = rx.recv() {
            let mut ok = write_frame(&mut w, &msg).is_ok();
            while ok && let Ok(more) = rx.try_recv() {
                ok = write_frame(&mut w, &more).is_ok();
            }
            if !ok || w.flush().is_err() {
                break;
            }
        }
    })?;

    thread::Builder::new().name(format!("client-{id}-read")).spawn(move || {
        let mut r = BufReader::with_capacity(64 * 1024, recv);
        loop {
            match read_frame::<_, ClientMsg>(&mut r) {
                Ok(Some(msg)) => {
                    let bye = msg == ClientMsg::Bye;
                    if events.send(Event::Client(id, msg)).is_err() || bye {
                        return;
                    }
                }
                Ok(None) => break,
                Err(e) => {
                    log::warn!("client {id}: {e}");
                    break;
                }
            }
        }
        let _ = events.send(Event::Disconnected(id));
    })?;
    Ok(())
}

pub struct IpcHost {
    writer: Mutex<BufWriter<interprocess::local_socket::SendHalf>>,
    rx: Receiver<ServerMsg>,
    waker: Arc<OnceLock<Waker>>,
    lost: Arc<AtomicBool>,

    leaving: Arc<AtomicBool>,
}

impl IpcHost {
    pub fn connect(name: &str) -> io::Result<Self> {
        let conn = Stream::connect(to_name(name)?)?;
        let (recv, send) = conn.split();
        let (tx, rx) = unbounded();
        let waker: Arc<OnceLock<Waker>> = Arc::new(OnceLock::new());
        let w = waker.clone();
        let lost = Arc::new(AtomicBool::new(false));
        let l = lost.clone();
        let leaving = Arc::new(AtomicBool::new(false));
        let bye = leaving.clone();
        thread::Builder::new().name("zhelld-reader".into()).spawn(move || {
            let mut r = BufReader::with_capacity(256 * 1024, recv);
            while let Ok(Some(msg)) = read_frame::<_, ServerMsg>(&mut r) {
                if tx.send(msg).is_err() {
                    return;
                }
                if let Some(w) = w.get() {
                    w();
                }
            }

            if bye.load(Ordering::SeqCst) {
                log::debug!("disconnected from zhelld");
            } else {
                log::warn!("lost connection to zhelld");
            }
            l.store(true, Ordering::SeqCst);
            if let Some(w) = w.get() {
                w();
            }
        })?;
        Ok(Self { writer: Mutex::new(BufWriter::new(send)), rx, waker, lost, leaving })
    }

    pub fn connect_or_spawn(daemon: &std::path::Path) -> io::Result<Self> {
        let name = socket_name();
        if let Ok(h) = Self::connect(&name) {
            return Ok(h);
        }
        spawn_daemon(daemon)?;
        let deadline = Instant::now() + Duration::from_secs(3);
        let mut delay = Duration::from_millis(5);
        loop {
            match Self::connect(&name) {
                Ok(h) => return Ok(h),
                Err(e) if Instant::now() > deadline => return Err(e),
                Err(_) => {
                    thread::sleep(delay);
                    delay = (delay * 2).min(Duration::from_millis(100));
                }
            }
        }
    }
}

fn spawn_daemon(daemon: &std::path::Path) -> io::Result<()> {
    use std::process::{Command, Stdio};
    let mut cmd = Command::new(daemon);
    cmd.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;

        cmd.process_group(0);
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const DETACHED_PROCESS: u32 = 0x0000_0008;
        const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP | CREATE_NO_WINDOW);
    }
    let mut child = cmd.spawn()?;

    thread::Builder::new().name("zhelld-reaper".into()).spawn(move || {
        let _ = child.wait();
    })?;
    Ok(())
}

impl Drop for IpcHost {
    fn drop(&mut self) {
        self.leaving.store(true, Ordering::SeqCst);
        self.send(ClientMsg::Bye);
    }
}

impl SessionHost for IpcHost {
    fn send(&self, msg: ClientMsg) {
        let mut w = self.writer.lock();
        if write_frame(&mut *w, &msg).and_then(|()| w.flush().map_err(Into::into)).is_err() {
            log::debug!("send to zhelld failed");
        }
    }

    fn try_recv(&self) -> Option<ServerMsg> {
        self.rx.try_recv().ok()
    }

    fn set_waker(&mut self, waker: Waker) {
        let _ = self.waker.set(waker);
    }

    fn shared(&self) -> bool {
        true
    }

    fn is_alive(&self) -> bool {
        !self.lost.load(Ordering::SeqCst) || !self.rx.is_empty()
    }
}
