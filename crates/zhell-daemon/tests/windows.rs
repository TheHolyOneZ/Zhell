#![cfg(windows)]

use std::thread;
use std::time::{Duration, Instant};

use zhell_core::SessionHost;
use zhell_daemon::InProcHost;
use zhell_daemon::ipc::{self, IpcHost};
use zhell_daemon::{OnDisconnect, Server};
use zhell_proto::{ClientMsg, FrameDiff, MarkKind, PROTO_VERSION, PaneId, ServerMsg, SpawnSpec, TermSize};

const SIZE: TermSize = TermSize { cols: 60, rows: 12, cell_width: 8, cell_height: 16 };
const TIMEOUT: Duration = Duration::from_secs(20);

fn cmd(args: &[&str]) -> SpawnSpec {
    SpawnSpec { program: Some("cmd.exe".into()), args: args.iter().map(|a| (*a).to_owned()).collect(), cwd: None, env: vec![], integration: false }
}

struct Mirror(Vec<String>);

impl Mirror {
    fn apply(&mut self, f: &FrameDiff) {
        if f.full || self.0.len() != f.rows as usize {
            self.0 = vec![String::new(); f.rows as usize];
        }
        for l in &f.lines {
            self.0[l.row as usize] = l.cells.iter().map(|c| c.ch).collect::<String>().trim_end().into();
        }
    }
    fn text(&self) -> String {
        self.0.join("\n")
    }
}

fn next(recv: &dyn Fn(Duration) -> Option<ServerMsg>, until: Instant) -> Option<ServerMsg> {
    loop {
        match recv(until.saturating_duration_since(Instant::now()))? {
            ServerMsg::Background { .. } => continue,
            m => return Some(m),
        }
    }
}

struct Session {
    host: InProcHost,
    pane: PaneId,
}

impl Session {
    fn start(spec: SpawnSpec) -> Self {
        let host = InProcHost::start().unwrap();
        host.send(ClientMsg::Hello { proto_version: PROTO_VERSION, client_name: "test".into(), restore: true });
        let until = Instant::now() + TIMEOUT;
        assert!(matches!(next(&|t| host.recv_timeout(t), until), Some(ServerMsg::HelloOk { .. })));
        host.send(ClientMsg::CreatePane { req: 1, spawn: spec, size: SIZE });
        let pane = match next(&|t| host.recv_timeout(t), until) {
            Some(ServerMsg::PaneCreated { pane, .. }) => pane,
            other => panic!("expected PaneCreated, got {other:?}"),
        };
        Self { host, pane }
    }

    fn until(&self, done: impl Fn(&Mirror, &[ServerMsg]) -> bool) -> (Mirror, Vec<ServerMsg>, Option<Option<i32>>) {
        let mut mirror = Mirror(Vec::new());
        let mut other = Vec::new();
        let deadline = Instant::now() + TIMEOUT;
        loop {
            let msg = next(&|t| self.host.recv_timeout(t), deadline);
            match msg {
                Some(ServerMsg::Frame(f)) => {
                    mirror.apply(&f);
                    self.host.send(ClientMsg::Ack { pane: self.pane, seq: f.seq });
                }
                Some(ServerMsg::Exited { code, .. }) => return (mirror, other, Some(code)),
                Some(m) => other.push(m),
                None => panic!("timed out; screen:\n{}\nmessages: {other:?}", mirror.text()),
            }
            if done(&mirror, &other) {
                return (mirror, other, None);
            }
        }
    }
}

#[test]
fn cmd_output_and_exit_code_come_through_conpty() {
    let s = Session::start(cmd(&["/c", "echo zhell-ok& exit /b 3"]));
    let (mirror, _, exit) = s.until(|_, _| false);
    assert!(mirror.text().contains("zhell-ok"), "{:?}", mirror.text());
    assert_eq!(exit, Some(Some(3)));
}

#[test]
fn typing_reaches_cmd() {
    let s = Session::start(cmd(&["/q", "/k"]));
    s.host.send(ClientMsg::Input { pane: s.pane, bytes: b"echo typed-%COMPUTERNAME%\r".to_vec() });
    let (mirror, _, _) = s.until(|m, _| m.text().contains("typed-") && !m.text().contains("typed-%"));
    s.host.send(ClientMsg::Input { pane: s.pane, bytes: b"exit\r".to_vec() });
    let (_, _, exit) = s.until(|_, _| false);
    assert!(mirror.text().contains("typed-"), "{:?}", mirror.text());
    assert_eq!(exit, Some(Some(0)));
}

#[test]
fn default_shell_reports_blocks_and_folder() {
    let spec = SpawnSpec { program: None, args: vec![], cwd: None, env: vec![], integration: true };
    let s = Session::start(spec);

    let prompt_seen = |o: &[ServerMsg]| o.iter().any(|m| matches!(m, ServerMsg::Mark { mark, .. } if mark.kind == MarkKind::PromptStart));
    s.until(|_, o| prompt_seen(o));
    s.host.send(ClientMsg::Input { pane: s.pane, bytes: b"echo block-test\r".to_vec() });
    let (mirror, other, _) = s.until(|_, o| o.iter().filter(|m| matches!(m, ServerMsg::Mark { mark, .. } if matches!(mark.kind, MarkKind::CommandFinished { .. }))).count() >= 1);
    assert!(mirror.text().contains("block-test"), "{:?}", mirror.text());
    assert!(other.iter().any(|m| matches!(m, ServerMsg::Cwd { cwd, .. } if cwd.contains(':'))), "no folder reported: {other:?}");
    s.host.send(ClientMsg::Input { pane: s.pane, bytes: b"exit\r".to_vec() });
}

#[test]
fn daemon_serves_a_named_pipe() {
    let name = format!("zhell-test-{}", std::process::id());
    let listener = ipc::listen(&name).expect("listen");
    let (events, rx) = crossbeam_channel::unbounded();
    let server = Server::new(events.clone(), OnDisconnect::Keep);
    thread::spawn(move || ipc::serve(listener, events));
    thread::spawn(move || server.run(rx));

    let h = IpcHost::connect(&name).expect("connect");
    let poll = |t: Duration| {
        let until = Instant::now() + t;
        loop {
            if let Some(m) = h.try_recv() {
                return Some(m);
            }
            if Instant::now() >= until {
                return None;
            }
            thread::sleep(Duration::from_millis(5));
        }
    };
    let until = Instant::now() + TIMEOUT;
    h.send(ClientMsg::Hello { proto_version: PROTO_VERSION, client_name: "test".into(), restore: true });
    assert!(matches!(next(&poll, until), Some(ServerMsg::HelloOk { .. })));
    h.send(ClientMsg::CreatePane { req: 1, spawn: cmd(&["/c", "echo over-the-pipe& ping -n 3 127.0.0.1 >nul"]), size: SIZE });
    let pane = match next(&poll, until) {
        Some(ServerMsg::PaneCreated { pane, .. }) => pane,
        other => panic!("{other:?}"),
    };
    let mut mirror = Mirror(Vec::new());
    loop {
        match next(&poll, until).expect("timed out") {
            ServerMsg::Frame(f) => {
                mirror.apply(&f);
                h.send(ClientMsg::Ack { pane, seq: f.seq });
                if mirror.text().contains("over-the-pipe") {
                    break;
                }
            }
            ServerMsg::Exited { .. } => panic!("exited early: {:?}", mirror.text()),
            _ => {}
        }
    }
    h.send(ClientMsg::Shutdown);
}
