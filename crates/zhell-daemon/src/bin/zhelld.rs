use std::io;
use std::thread;

use crossbeam_channel::unbounded;
use zhell_daemon::{OnDisconnect, Server, ipc};

fn main() {
    let name = ipc::socket_name();
    match std::env::args().nth(1).as_deref() {
        Some("--stop") => {
            use zhell_core::SessionHost;
            match ipc::IpcHost::connect(&name) {
                Ok(h) => {
                    h.send(zhell_proto::ClientMsg::Shutdown);
                    println!("zhelld stopped");
                }
                Err(_) => println!("zhelld is not running"),
            }
            return;
        }
        Some("--help" | "-h") => {
            println!("zhelld: Zhell session daemon (started automatically by zhell)\n\n  --stop   end all sessions and stop the daemon");
            return;
        }
        _ => {}
    }
    init_logging();
    let listener = match ipc::listen(&name) {
        Ok(l) => l,
        Err(e) if e.kind() == io::ErrorKind::AddrInUse => {
            log::info!("another zhelld already serves {name}");
            return;
        }
        Err(e) => {
            log::error!("cannot listen on {name}: {e}");
            std::process::exit(1);
        }
    };
    log::info!("zhelld {} listening on {name}", env!("CARGO_PKG_VERSION"));

    let (events, events_rx) = unbounded();
    let mut server = Server::new(events.clone(), OnDisconnect::Keep);
    let config = zhell_core::config::default_path()
        .and_then(|p| zhell_core::config::Config::load(&p).map_err(|e| log::warn!("{e}")).ok())
        .unwrap_or_default();
    if let Some(h) = zhell_daemon::history::HistorySettings::from_config(&config.history) {
        server = server.with_history(h);
    }
    if let Some(path) = zhell_daemon::snapshot::default_path() {
        server = server.with_snapshots(path);
        if config.sessions.restore_after_reboot {
            let shell = &config.shell;
            let spawn = zhell_proto::SpawnSpec {
                program: Some(shell.program.clone()).filter(|p| !p.is_empty()),
                args: shell.args.clone(),
                cwd: None,
                env: shell.env.iter().map(|(k, v)| (k.clone(), v.clone())).collect(),
                integration: shell.integration,
            };
            server.restore_snapshot(&spawn);
        }
    }
    #[cfg(unix)]
    {
        use signal_hook::consts::{SIGHUP, SIGINT, SIGTERM};
        let ev = events.clone();
        match signal_hook::iterator::Signals::new([SIGTERM, SIGHUP, SIGINT]) {
            Ok(mut signals) => {
                thread::spawn(move || {
                    if let Some(sig) = signals.forever().next() {
                        log::info!("signal {sig}: saving sessions and exiting");
                        let _ = ev.send(zhell_daemon::Event::SaveSnapshot);
                        let _ = ev.send(zhell_daemon::Event::Abort);
                    }
                });
            }
            Err(e) => log::warn!("signal handler: {e}"),
        }
    }
    thread::Builder::new()
        .name("accept".into())
        .spawn(move || ipc::serve(listener, events))
        .expect("spawn accept thread");
    server.run(events_rx);

    #[cfg(unix)]
    let _ = std::fs::remove_file(&name);
    log::info!("zhelld exiting");

    std::process::exit(0);
}

fn init_logging() {
    let mut builder = env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info"));
    if let Some(dir) = dirs::state_dir().or_else(dirs::data_local_dir).map(|d| d.join("zhell"))
        && std::fs::create_dir_all(&dir).is_ok()
        && let Ok(file) = std::fs::OpenOptions::new().create(true).append(true).open(dir.join("zhelld.log"))
    {
        builder.target(env_logger::Target::Pipe(Box::new(file)));
    }
    builder.init();
}
