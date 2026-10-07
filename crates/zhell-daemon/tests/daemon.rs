#![cfg(unix)]

use std::thread;
use std::time::{Duration, Instant};

use crossbeam_channel::unbounded;
use zhell_core::SessionHost;
use zhell_daemon::ipc::{self, IpcHost};
use zhell_daemon::{Event, OnDisconnect, Server};
use zhell_proto::{ClientMsg, PROTO_VERSION, PaneId, Restore, ServerMsg, SpawnSpec, TermSize};

const SIZE: TermSize = TermSize { cols: 40, rows: 6, cell_width: 8, cell_height: 16 };
const TIMEOUT: Duration = Duration::from_secs(10);

fn daemon(tag: &str) -> (String, thread::JoinHandle<()>) {
    let (name, handle, _) = daemon_with(tag, None);
    (name, handle)
}

fn daemon_with(
    tag: &str,
    snapshot: Option<std::path::PathBuf>,
) -> (String, thread::JoinHandle<()>, crossbeam_channel::Sender<Event>) {
    let dir = std::env::temp_dir().join(format!("zhell-test-{}-{tag}", std::process::id()));
    let name = dir.join("d.sock").display().to_string();
    let listener = ipc::listen(&name).expect("listen");
    let (events, rx) = unbounded();
    let mut server = Server::new(events.clone(), OnDisconnect::Keep);
    if let Some(path) = snapshot {
        server = server.with_snapshots(path);
        server.restore_snapshot(&SpawnSpec { program: Some("/bin/sh".into()), args: vec![], cwd: None, env: vec![], integration: false });
    }
    let ev = events.clone();
    thread::spawn(move || ipc::serve(listener, events));
    let handle = thread::spawn(move || server.run(rx));
    (name, handle, ev)
}

fn recv(h: &IpcHost, until: Instant) -> ServerMsg {
    loop {
        match h.try_recv() {
            Some(ServerMsg::Background { .. }) => continue,
            Some(m) => return m,
            None => {}
        }
        assert!(Instant::now() < until, "timed out waiting for the daemon");
        thread::sleep(Duration::from_millis(5));
    }
}

fn hello(h: &IpcHost) -> Option<Restore> {
    h.send(ClientMsg::Hello { proto_version: PROTO_VERSION, client_name: "test".into(), restore: true });
    match recv(h, Instant::now() + TIMEOUT) {
        ServerMsg::HelloOk { restore, .. } => restore,
        m => panic!("expected HelloOk, got {m:?}"),
    }
}

fn screen_until(h: &IpcHost, pane: PaneId, pred: impl Fn(&str) -> bool) -> String {
    let mut rows = vec![String::new(); SIZE.rows as usize];
    let deadline = Instant::now() + TIMEOUT;
    loop {
        if let ServerMsg::Frame(f) = recv(h, deadline) {
            for l in &f.lines {
                rows[l.row as usize] = l.cells.iter().map(|c| c.ch).collect::<String>().trim_end().into();
            }
            h.send(ClientMsg::Ack { pane, seq: f.seq });
            let text = rows.join("\n");
            if pred(&text) {
                return text;
            }
        }
    }
}

#[test]
fn sessions_survive_client_disconnect_and_reattach() {
    let (name, server) = daemon("reattach");

    let first = IpcHost::connect(&name).expect("connect");
    assert_eq!(hello(&first), None, "fresh daemon has nothing to restore");
    let spawn = SpawnSpec {
        program: Some("/bin/sh".into()),
        args: vec!["-c".into(), "echo before; read x; echo got:$x; sleep 30".into()],
        cwd: None,
        env: vec![],
        integration: false,
    };
    first.send(ClientMsg::CreatePane { req: 1, spawn, size: SIZE });
    let pane = match recv(&first, Instant::now() + TIMEOUT) {
        ServerMsg::PaneCreated { pane, .. } => pane,
        m => panic!("{m:?}"),
    };
    screen_until(&first, pane, |t| t.contains("before"));
    first.send(ClientMsg::StoreLayout(b"my-layout".to_vec()));
    drop(first);

    thread::sleep(Duration::from_millis(100));
    let second = IpcHost::connect(&name).expect("reconnect");
    let restore = hello(&second).expect("restore offered");
    assert_eq!(restore.layout, b"my-layout");
    assert_eq!(restore.panes, vec![pane]);
    second.send(ClientMsg::Attach { pane });
    let text = screen_until(&second, pane, |t| t.contains("before"));
    assert!(text.starts_with("before"), "{text:?}");
    second.send(ClientMsg::Input { pane, bytes: b"hi\r".to_vec() });
    screen_until(&second, pane, |t| t.contains("got:hi"));

    second.send(ClientMsg::ClosePane { pane });
    drop(second);
    let deadline = Instant::now() + TIMEOUT;
    while !server.is_finished() {
        assert!(Instant::now() < deadline, "daemon did not stop");
        thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn socket_is_private() {
    use std::os::unix::fs::PermissionsExt;
    let (name, _server) = daemon("perms");
    let dir = std::path::Path::new(&name).parent().unwrap();
    assert_eq!(std::fs::metadata(dir).unwrap().permissions().mode() & 0o777, 0o700);
    assert_eq!(std::fs::metadata(&name).unwrap().permissions().mode() & 0o077, 0);
}

#[test]
fn second_daemon_on_same_socket_fails_with_addr_in_use() {
    let (name, _server) = daemon("double");
    let err = ipc::listen(&name).expect_err("must not steal a live socket");
    assert_eq!(err.kind(), std::io::ErrorKind::AddrInUse);
}

#[test]
fn sessions_come_back_after_a_reboot() {
    let dir = std::env::temp_dir().join(format!("zhell-test-{}-reboot", std::process::id()));
    let snap = dir.join("sessions.bin");
    let work = dir.join("work");
    std::fs::create_dir_all(&work).unwrap();

    let (name, server, events) = daemon_with("reboot-a", Some(snap.clone()));
    let c = IpcHost::connect(&name).unwrap();
    hello(&c);
    let spawn = SpawnSpec {
        program: Some("/bin/sh".into()),
        args: vec!["-c".into(), format!("cd '{}'; echo hello-before-reboot; sleep 30", work.display())],
        cwd: None,
        env: vec![],
        integration: false,
    };
    c.send(ClientMsg::CreatePane { req: 1, spawn, size: SIZE });
    let pane = match recv(&c, Instant::now() + TIMEOUT) {
        ServerMsg::PaneCreated { pane, .. } => pane,
        m => panic!("{m:?}"),
    };
    screen_until(&c, pane, |t| t.contains("hello-before-reboot"));
    c.send(ClientMsg::StoreLayout(b"saved-layout".to_vec()));
    thread::sleep(Duration::from_millis(100));
    events.send(Event::SaveSnapshot).unwrap();
    events.send(Event::Abort).unwrap();
    server.join().unwrap();
    drop(c);

    let (name, _server, _) = daemon_with("reboot-b", Some(snap));
    let c = IpcHost::connect(&name).unwrap();
    let restore = hello(&c).expect("restore after reboot");
    assert_eq!(restore.layout, b"saved-layout");
    assert_eq!(restore.panes, vec![pane]);
    c.send(ClientMsg::Attach { pane });
    let text = screen_until(&c, pane, |t| t.contains("session restored"));
    assert!(text.contains("hello-before-reboot"), "{text:?}");
    c.send(ClientMsg::Input { pane, bytes: b"pwd\r".to_vec() });
    let work = work.canonicalize().unwrap().display().to_string();
    screen_until(&c, pane, |t| t.lines().any(|l| l.trim() == work));
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn panes_move_between_windows() {
    let (name, _server) = daemon("move");
    let a = IpcHost::connect(&name).expect("connect");
    hello(&a);
    let spawn = SpawnSpec { program: Some("/bin/sh".into()), args: vec!["-c".into(), "echo moved; sleep 30".into()], cwd: None, env: vec![], integration: false };
    a.send(ClientMsg::CreatePane { req: 1, spawn, size: SIZE });
    let pane = match recv(&a, Instant::now() + TIMEOUT) {
        ServerMsg::PaneCreated { pane, .. } => pane,
        m => panic!("{m:?}"),
    };
    screen_until(&a, pane, |t| t.contains("moved"));

    let b = IpcHost::connect(&name).expect("connect b");
    b.send(ClientMsg::Hello { proto_version: PROTO_VERSION, client_name: "b".into(), restore: false });
    assert!(matches!(recv(&b, Instant::now() + TIMEOUT), ServerMsg::HelloOk { restore: None, .. }));

    b.send(ClientMsg::Attach { pane });
    screen_until(&b, pane, |t| t.contains("moved"));
    let deadline = Instant::now() + TIMEOUT;
    loop {
        match recv(&a, deadline) {
            ServerMsg::Detached { pane: p } => {
                assert_eq!(p, pane);
                break;
            }
            ServerMsg::Frame(_) => {}
            m => panic!("unexpected {m:?}"),
        }
    }
    a.send(ClientMsg::Shutdown);
}
