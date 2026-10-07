#![cfg(unix)]

use std::time::{Duration, Instant};

use zhell_core::SessionHost;
use zhell_daemon::InProcHost;
use zhell_proto::{
    ClientMsg, FrameDiff, MarkKind, PROTO_VERSION, PaneId, ServerMsg, SpawnSpec, TermSize,
};

const SIZE: TermSize = TermSize { cols: 40, rows: 10, cell_width: 8, cell_height: 16 };
const TIMEOUT: Duration = Duration::from_secs(10);

fn recv(h: &InProcHost, t: Duration) -> Option<ServerMsg> {
    let deadline = Instant::now() + t;
    loop {
        match h.recv_timeout(deadline.saturating_duration_since(Instant::now()))? {
            ServerMsg::Background { .. } => continue,
            m => return Some(m),
        }
    }
}

fn sh(script: &str) -> SpawnSpec {
    SpawnSpec {
        program: Some("/bin/sh".into()),
        args: vec!["-c".into(), script.into()],
        cwd: None,
        env: vec![],
        integration: false,
    }
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

struct Session {
    host: InProcHost,
    pane: PaneId,
}

impl Session {
    fn start(spec: SpawnSpec) -> Self {
        let host = InProcHost::start().unwrap();
        host.send(ClientMsg::Hello { proto_version: PROTO_VERSION, client_name: "test".into(), restore: true });
        assert!(matches!(recv(&host, TIMEOUT), Some(ServerMsg::HelloOk { .. })));
        host.send(ClientMsg::CreatePane { req: 7, spawn: spec, size: SIZE });
        let pane = match recv(&host, TIMEOUT) {
            Some(ServerMsg::PaneCreated { req: 7, pane }) => pane,
            other => panic!("expected PaneCreated, got {other:?}"),
        };
        Self { host, pane }
    }

    fn run_to_exit(&self) -> (Mirror, Vec<ServerMsg>, Option<i32>) {
        let mut mirror = Mirror(Vec::new());
        let mut other = Vec::new();
        let deadline = Instant::now() + TIMEOUT;
        loop {
            let msg = recv(&self.host, deadline - Instant::now()).expect("timed out");
            match msg {
                ServerMsg::Frame(f) => {
                    mirror.apply(&f);
                    self.host.send(ClientMsg::Ack { pane: self.pane, seq: f.seq });
                }
                ServerMsg::Exited { code, .. } => return (mirror, other, code),
                m => other.push(m),
            }
        }
    }
}

#[test]
fn version_mismatch_is_reported() {
    let host = InProcHost::start().unwrap();
    host.send(ClientMsg::Hello { proto_version: PROTO_VERSION + 1, client_name: "old".into(), restore: true });
    assert!(matches!(recv(&host, TIMEOUT), Some(ServerMsg::VersionMismatch { .. })));
}

#[test]
fn output_reaches_mirror_and_exit_code_propagates() {
    let s = Session::start(sh("printf 'hello\\r\\nworld'; exit 3"));
    let (mirror, _, code) = s.run_to_exit();
    let text = mirror.text();
    assert!(text.starts_with("hello\nworld"), "{text:?}");
    assert_eq!(code, Some(3));
}

#[test]
fn input_is_written_to_pty() {
    let s = Session::start(sh("read line; printf 'got:%s' \"$line\""));
    s.host.send(ClientMsg::Input { pane: s.pane, bytes: b"abc\r".to_vec() });
    let (mirror, _, code) = s.run_to_exit();
    assert!(mirror.text().contains("got:abc"), "{:?}", mirror.text());
    assert_eq!(code, Some(0));
}

#[test]
fn shell_marks_and_cwd_are_reported() {
    let script = r"printf '\033]7;file:///tmp/x\007\033]133;A\007$ \033]133;B\007echo hi\r\n\033]133;C\007hi\r\n\033]133;D;0\007\033]7;file://otherbox.lan/srv/app\007'";
    let s = Session::start(sh(script));
    let (_, msgs, _) = s.run_to_exit();
    let marks: Vec<_> = msgs
        .iter()
        .filter_map(|m| match m {
            ServerMsg::Mark { mark, .. } => Some((mark.kind.clone(), mark.abs_line)),
            _ => None,
        })
        .collect();
    assert_eq!(
        marks,
        vec![
            (MarkKind::PromptStart, 0),
            (MarkKind::CommandStart, 0),
            (MarkKind::OutputStart, 1),
            (MarkKind::CommandFinished { exit_code: Some(0) }, 2),
        ]
    );
    assert!(msgs.iter().any(|m| matches!(m, ServerMsg::Cwd { cwd, .. } if cwd == "/tmp/x")));

    assert!(msgs.iter().any(|m| matches!(m, ServerMsg::RemoteCwd { host, cwd, .. } if host == "otherbox.lan" && cwd == "/srv/app")));
    assert!(!msgs.iter().any(|m| matches!(m, ServerMsg::Cwd { cwd, .. } if cwd == "/srv/app")));
}

#[test]
fn frames_wait_for_ack() {
    let s = Session::start(sh("seq 1 50000; sleep 3"));
    let mut frames = 0;
    let deadline = Instant::now() + Duration::from_millis(800);
    while let Some(m) = recv(&s.host, deadline.saturating_duration_since(Instant::now())) {
        if let ServerMsg::Frame(_) = m {
            frames += 1;
        }
        if Instant::now() >= deadline {
            break;
        }
    }
    assert_eq!(frames, 1, "host sent frames without acks");
}

#[test]
fn resize_produces_full_frame_of_new_size() {
    let s = Session::start(sh("sleep 0.5"));
    let first = loop {
        if let Some(ServerMsg::Frame(f)) = recv(&s.host, TIMEOUT) {
            break f;
        }
    };
    let size = TermSize { cols: 60, rows: 5, ..SIZE };
    s.host.send(ClientMsg::Resize { pane: s.pane, size });
    s.host.send(ClientMsg::Ack { pane: s.pane, seq: first.seq });
    let f = loop {
        if let Some(ServerMsg::Frame(f)) = recv(&s.host, TIMEOUT) {
            break f;
        }
    };
    assert!(f.full);
    assert_eq!((f.cols, f.rows, f.lines.len()), (60, 5, 5));
}

#[test]
fn selection_word_line_and_simple_copy() {
    use zhell_proto::{CopyTarget, SelectKind};
    let s = Session::start(sh("printf 'hello world\\r\\nsecond line'; sleep 2"));

    let mut mirror = Mirror(Vec::new());
    let deadline = Instant::now() + TIMEOUT;
    while !mirror.text().contains("second line") {
        if let Some(ServerMsg::Frame(f)) = recv(&s.host, deadline - Instant::now()) {
            mirror.apply(&f);
            s.host.send(ClientMsg::Ack { pane: s.pane, seq: f.seq });
        }
    }
    let copy = |kind, (row, col), end: Option<(i32, u16)>| {
        s.host.send(ClientMsg::SelectStart { pane: s.pane, row, col, right_half: false, kind });
        if let Some((r, c)) = end {
            s.host.send(ClientMsg::SelectUpdate { pane: s.pane, row: r, col: c, right_half: true });
        }
        s.host.send(ClientMsg::Copy { pane: s.pane, target: CopyTarget::Clipboard });
        loop {
            match recv(&s.host, TIMEOUT).expect("no CopyText") {
                ServerMsg::CopyText { text, .. } => return text,
                ServerMsg::Frame(f) => s.host.send(ClientMsg::Ack { pane: s.pane, seq: f.seq }),
                _ => {}
            }
        }
    };
    assert_eq!(copy(SelectKind::Word, (0, 8), None), "world");

    assert_eq!(copy(SelectKind::Line, (1, 0), None), "second line\n");
    assert_eq!(copy(SelectKind::Simple, (0, 6), Some((1, 5))), "world\nsecond");
    assert_eq!(copy(SelectKind::Block, (0, 1), Some((1, 3))), "ell\neco");
}

#[test]
fn resolves_paths_against_the_shells_cwd() {
    let dir = std::env::temp_dir().join(format!("zhell-resolve-{}", std::process::id()));
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(dir.join("src/main.rs"), "fn main() {}").unwrap();

    let s = Session::start(sh(&format!("cd '{}' && sleep 2", dir.display())));
    std::thread::sleep(Duration::from_millis(300));
    let resolve = |path: &str| {
        s.host.send(ClientMsg::ResolvePath { pane: s.pane, req: 1, path: path.into() });
        loop {
            match recv(&s.host, TIMEOUT).expect("no reply") {
                ServerMsg::PathResolved { path, .. } => return path,
                ServerMsg::Frame(f) => s.host.send(ClientMsg::Ack { pane: s.pane, seq: f.seq }),
                _ => {}
            }
        }
    };
    let expected = dir.join("src/main.rs").canonicalize().unwrap().display().to_string();
    assert_eq!(resolve("src/main.rs"), Some(expected));
    assert_eq!(resolve("src/missing.rs"), None);
    std::fs::remove_dir_all(&dir).unwrap();
}

fn bash_with_integration(home: &std::path::Path) -> SpawnSpec {
    std::fs::create_dir_all(home).unwrap();
    SpawnSpec {
        program: Some("bash".into()),
        args: vec![],
        cwd: Some(home.display().to_string()),
        env: vec![("HOME".into(), home.display().to_string()), ("PS1".into(), "$ ".into())],
        integration: true,
    }
}

#[test]
fn bash_integration_produces_command_blocks() {
    use zhell_proto::{BlockState, CopyTarget};
    let home = std::env::temp_dir().join(format!("zhell-blocks-{}", std::process::id()));

    std::fs::create_dir_all(&home).unwrap();
    std::fs::write(home.join(".bash_history"), "true\n".repeat(150)).unwrap();
    let mut spec = bash_with_integration(&home);
    spec.env.push(("HISTFILE".into(), home.join(".bash_history").display().to_string()));
    let s = Session::start(spec);
    s.host.send(ClientMsg::Input { pane: s.pane, bytes: b"echo hello\r".to_vec() });
    std::thread::sleep(Duration::from_millis(300));
    s.host.send(ClientMsg::Input { pane: s.pane, bytes: b"(exit 3)\r".to_vec() });

    let deadline = Instant::now() + TIMEOUT;
    let blocks = loop {
        if let ServerMsg::Frame(f) = recv(&s.host, deadline - Instant::now()).expect("timed out") {
            s.host.send(ClientMsg::Ack { pane: s.pane, seq: f.seq });
            let done = f.blocks.iter().filter(|b| matches!(b.state, BlockState::Done { .. })).count();
            let at_prompt = f.blocks.last().is_some_and(|b| b.state == BlockState::Editing);
            if done >= 2 && at_prompt {
                break f.blocks;
            }
        }
    };
    let echo = blocks.iter().find(|b| b.cmd.as_deref() == Some("echo hello")).expect("echo block");
    assert_eq!(echo.state, BlockState::Done { exit: Some(0) });
    assert_eq!(echo.output_row, echo.prompt_row + 1, "{blocks:?}");
    let fail = blocks.iter().find(|b| b.cmd.as_deref() == Some("(exit 3)")).expect("exit block");
    assert_eq!(fail.state, BlockState::Done { exit: Some(3) });
    assert!(fail.prompt_row > echo.prompt_row);

    assert_eq!(blocks.last().unwrap().state, BlockState::Editing);

    s.host.send(ClientMsg::CopyBlockOutput { pane: s.pane, block: echo.id, target: CopyTarget::Clipboard });
    let text = loop {
        match recv(&s.host, TIMEOUT).expect("no copy") {
            ServerMsg::CopyText { text, .. } => break text,
            ServerMsg::Frame(f) => s.host.send(ClientMsg::Ack { pane: s.pane, seq: f.seq }),
            _ => {}
        }
    };
    assert_eq!(text, "hello");
    let _ = std::fs::remove_dir_all(&home);
}

#[test]
fn find_in_scrollback_jumps_to_matches() {
    let s = Session::start(sh("echo needle-old; seq 1 200; echo needle-new; seq 1 3; sleep 30"));
    let next_frame = |pred: &dyn Fn(&FrameDiff) -> bool| -> FrameDiff {
        let deadline = Instant::now() + TIMEOUT;
        let mut last = None;
        loop {
            assert!(Instant::now() < deadline, "no matching frame; last find state: {last:?}");
            let m = recv(&s.host, deadline.saturating_duration_since(Instant::now()));
            if let Some(ServerMsg::Frame(f)) = m {
                s.host.send(ClientMsg::Ack { pane: s.pane, seq: f.seq });
                last = Some((f.find.clone(), f.display_offset));
                if pred(&f) {
                    return f;
                }
            }
        }
    };

    std::thread::sleep(Duration::from_millis(400));
    s.host.send(ClientMsg::Find { pane: s.pane, query: "NEEDLE".to_lowercase(), regex: false });
    let f = next_frame(&|f| f.find.as_ref().is_some_and(|x| x.current.is_some()));
    let find = f.find.unwrap();
    assert!(find.found && !find.invalid);
    let cur = find.current.unwrap();
    assert_eq!(cur.end_col - cur.start_col, 5, "matches the 6-letter word");
    assert_eq!(f.display_offset, 0, "newest match is on screen without scrolling");

    s.host.send(ClientMsg::FindNext { pane: s.pane, older: true });
    let f = next_frame(&|f| f.display_offset > 0 && f.find.as_ref().is_some_and(|x| x.current.is_some()));
    let cur = f.find.unwrap().current.unwrap();
    assert!(cur.start_row >= 0 && cur.start_row < f.rows as i32);

    s.host.send(ClientMsg::Find { pane: s.pane, query: "(".into(), regex: true });
    let f = next_frame(&|f| f.find.as_ref().is_some_and(|x| x.invalid));
    assert!(!f.find.unwrap().found);
}

#[test]
fn folding_a_block_hides_its_output_behind_one_row() {
    use zhell_proto::BlockState;
    let home = std::env::temp_dir().join(format!("zhell-fold-{}", std::process::id()));
    let s = Session::start(bash_with_integration(&home));
    s.host.send(ClientMsg::Input { pane: s.pane, bytes: b"seq 1 50\r".to_vec() });
    let wait = |pred: &dyn Fn(&FrameDiff) -> bool| -> FrameDiff {
        let deadline = Instant::now() + TIMEOUT;
        loop {
            assert!(Instant::now() < deadline, "timed out");
            if let Some(ServerMsg::Frame(f)) = recv(&s.host, deadline.saturating_duration_since(Instant::now())) {
                s.host.send(ClientMsg::Ack { pane: s.pane, seq: f.seq });
                if pred(&f) {
                    return f;
                }
            }
        }
    };
    let f = wait(&|f| f.blocks.iter().any(|b| b.cmd.as_deref() == Some("seq 1 50") && matches!(b.state, BlockState::Done { .. })));
    let block = f.blocks.iter().find(|b| b.cmd.as_deref() == Some("seq 1 50")).unwrap().id;

    s.host.send(ClientMsg::Fold { pane: s.pane, block, folded: true });

    let f = wait(&|f| !f.folds.is_empty() && f.blocks.last().is_some_and(|b| b.state == BlockState::Editing));
    let fold = f.folds[0];
    assert_eq!((fold.block, fold.hidden), (block, 47), "3 lines kept, 47 hidden");
    let text: Vec<String> = f.lines.iter().map(|l| l.cells.iter().map(|c| c.ch).collect::<String>().trim_end().to_owned()).collect();
    let r = fold.row as usize;
    assert_eq!(&text[r - 3..r], &["1", "2", "3"], "{text:?}");
    assert!(text[r].contains("47 more lines"), "{text:?}");

    assert!(text[r + 1..].iter().any(|l| l.contains('$')), "{text:?}");

    s.host.send(ClientMsg::Fold { pane: s.pane, block, folded: false });
    wait(&|f| f.folds.is_empty() && f.full);
    let _ = std::fs::remove_dir_all(&home);
}

#[test]
fn folding_while_scrolled_up_shows_the_summary() {
    use zhell_proto::BlockState;
    let home = std::env::temp_dir().join(format!("zhell-fold2-{}", std::process::id()));
    let s = Session::start(bash_with_integration(&home));
    s.host.send(ClientMsg::Input { pane: s.pane, bytes: b"seq 1 200\r".to_vec() });
    let wait = |pred: &dyn Fn(&FrameDiff) -> bool| -> FrameDiff {
        let deadline = Instant::now() + TIMEOUT;
        loop {
            assert!(Instant::now() < deadline, "timed out");
            if let Some(ServerMsg::Frame(f)) = recv(&s.host, deadline.saturating_duration_since(Instant::now())) {
                s.host.send(ClientMsg::Ack { pane: s.pane, seq: f.seq });
                if pred(&f) {
                    return f;
                }
            }
        }
    };
    let f = wait(&|f| f.blocks.last().is_some_and(|b| b.state == BlockState::Editing) && f.blocks.len() >= 2);
    let block = f.blocks.iter().find(|b| b.cmd.as_deref() == Some("seq 1 200")).unwrap().id;

    s.host.send(ClientMsg::JumpBlock { pane: s.pane, forward: false });
    wait(&|f| f.display_offset > 0);
    s.host.send(ClientMsg::Fold { pane: s.pane, block, folded: true });
    let f = wait(&|f| !f.folds.is_empty());
    let text: Vec<String> = f.lines.iter().map(|l| l.cells.iter().map(|c| c.ch).collect::<String>().trim_end().to_owned()).collect();
    let r = f.folds[0].row as usize;
    assert!(text[r].contains("197 more lines"), "{text:?}");

    assert!(!text.iter().any(|l| l == "100" || l == "33"), "{text:?}");
    let _ = std::fs::remove_dir_all(&home);
}
