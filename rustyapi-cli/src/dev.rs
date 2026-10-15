//! `tez dev`: сборка, запуск и перезапуск при изменениях.
//!
//! Пока идёт сборка, старый сервер продолжает работать. Если сборка не удалась — сервер тоже
//! остаётся, `tez` ждёт следующего изменения. Перед запуском нового процесса старый получает
//! SIGTERM (через 5 с — kill).

use std::path::{Component, Path, PathBuf};
use std::process::{Child, Command};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::time::{Duration, Instant};

use notify::{EventKind, RecursiveMode, Watcher};

use crate::cargo::{self, Target};
use crate::term;
use crate::{Result, TargetArgs};

/// Сколько ждать тишины после изменения файла.
const DEBOUNCE: Duration = Duration::from_millis(300);
/// Сколько ждать корректной остановки сервера после SIGTERM.
const STOP_TIMEOUT: Duration = Duration::from_secs(5);
/// Если пересборка дольше, один раз показываем подсказку про быстрый линкер.
const SLOW_BUILD: Duration = Duration::from_secs(5);

static STOP: AtomicBool = AtomicBool::new(false);

pub fn run(args: TargetArgs, app_args: Vec<String>) -> Result<()> {
    let target = Target::resolve(&args)?;
    install_signal_handlers();

    let (tx, rx) = mpsc::channel();
    let root = target.dir.clone();
    let mut watcher = notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
        let Ok(event) = event else { return };
        if matches!(
            event.kind,
            EventKind::Create(_) | EventKind::Modify(_) | EventKind::Remove(_)
        ) {
            for path in event.paths {
                if is_relevant(&root, &path) {
                    let _ = tx.send(path);
                }
            }
        }
    })?;
    watcher.watch(&target.dir, RecursiveMode::Recursive)?;
    term::status(&format!(
        "слежу за {} (src/, migrations/, .env, Cargo.toml); Ctrl+C — выход",
        target.dir.display()
    ));

    let mut server: Option<Child> = None;
    let mut first = true;
    let mut hinted = false;
    loop {
        let started = Instant::now();
        term::status(if first {
            "сборка..."
        } else {
            "пересборка..."
        });
        match cargo::build(&target, false, false)? {
            Some(exe) => {
                let took = started.elapsed();
                if let Some(old) = server.take() {
                    stop(old);
                }
                term::status(&format!(
                    "{} за {:.1} с, запуск {}",
                    term::green("собрано"),
                    took.as_secs_f64(),
                    target.bin
                ));
                if !first && !hinted && took > SLOW_BUILD {
                    hinted = true;
                    linker_hint(&target.dir);
                }
                server = Some(
                    Command::new(&exe)
                        .args(&app_args)
                        .current_dir(&target.dir)
                        .spawn()?,
                );
            }
            None => {
                eprintln!();
                term::status(&format!(
                    "{} {}; жду изменений",
                    term::red("ошибка сборки."),
                    if server.is_some() {
                        "Сервер продолжает работать на прошлой версии"
                    } else {
                        "Сервер не запущен"
                    }
                ));
            }
        }
        first = false;

        // Ждём изменений (или Ctrl+C), следим, не упал ли сервер.
        let changed = loop {
            if STOP.load(Ordering::SeqCst) {
                if let Some(old) = server.take() {
                    stop(old);
                }
                term::status("остановлено");
                return Ok(());
            }
            let exited = server
                .as_mut()
                .and_then(|child| child.try_wait().ok().flatten());
            if let Some(status) = exited {
                term::status(&format!(
                    "{} ({status}); жду изменений",
                    term::yellow("сервер завершился")
                ));
                server = None;
            }
            match rx.recv_timeout(Duration::from_millis(100)) {
                Ok(path) => break path,
                Err(RecvTimeoutError::Timeout) => continue,
                Err(RecvTimeoutError::Disconnected) => return Err("watcher остановился".into()),
            }
        };
        let mut changed = vec![changed];
        // Дебаунс: собираем изменения, пока не наступит тишина.
        while let Ok(path) = rx.recv_timeout(DEBOUNCE) {
            changed.push(path);
        }
        changed.sort();
        changed.dedup();
        let shown: Vec<String> = changed
            .iter()
            .take(3)
            .map(|p| {
                p.strip_prefix(&target.dir)
                    .unwrap_or(p)
                    .display()
                    .to_string()
            })
            .collect();
        let more = changed.len().saturating_sub(3);
        term::status(&format!(
            "изменено: {}{}",
            shown.join(", "),
            if more > 0 {
                format!(" и ещё {more}")
            } else {
                String::new()
            }
        ));
    }
}

/// Нужно ли перезапускаться из-за этого файла: исходники, миграции, .env, Cargo.toml.
/// target/, .git/ и временные файлы редакторов игнорируются.
pub fn is_relevant(root: &Path, path: &Path) -> bool {
    let relative = path.strip_prefix(root).unwrap_or(path);
    let ignored_dir = relative.components().any(|c| match c {
        Component::Normal(name) => {
            let name = name.to_string_lossy();
            name == "target" || name == ".git" || name == "node_modules"
        }
        _ => false,
    });
    if ignored_dir {
        return false;
    }
    let Some(name) = relative
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
    else {
        return false;
    };
    if name.ends_with('~') || name.starts_with(".#") || name.ends_with(".swp") {
        return false;
    }
    if name == ".env" || name.starts_with(".env.") {
        return true;
    }
    if name == "Cargo.lock" {
        // cargo сам пишет Cargo.lock во время сборки.
        return false;
    }
    let in_migrations = relative.components().any(|c| c.as_os_str() == "migrations");
    match Path::new(&name).extension().and_then(|e| e.to_str()) {
        Some("rs" | "toml") => true,
        Some("sql") => in_migrations,
        _ => false,
    }
}

/// Корректная остановка: SIGTERM, через 5 секунд — kill.
fn stop(mut child: Child) {
    if let Ok(Some(_)) = child.try_wait() {
        return;
    }
    term::status("останавливаю сервер...");
    #[cfg(unix)]
    {
        // SAFETY: kill с pid нашего дочернего процесса, который ещё не освобождён wait.
        unsafe {
            libc::kill(child.id() as libc::pid_t, libc::SIGTERM);
        }
        let deadline = Instant::now() + STOP_TIMEOUT;
        while Instant::now() < deadline {
            if let Ok(Some(_)) = child.try_wait() {
                return;
            }
            std::thread::sleep(Duration::from_millis(25));
        }
        term::status(&term::yellow(
            "сервер не остановился за 5 с после SIGTERM, kill",
        ));
    }
    let _ = child.kill();
    let _ = child.wait();
}

#[cfg(unix)]
fn install_signal_handlers() {
    extern "C" fn on_signal(_: libc::c_int) {
        STOP.store(true, Ordering::SeqCst);
    }
    // SAFETY: обработчик только пишет в атомарную переменную (async-signal-safe).
    unsafe {
        let handler = on_signal as extern "C" fn(libc::c_int) as libc::sighandler_t;
        libc::signal(libc::SIGINT, handler);
        libc::signal(libc::SIGTERM, handler);
    }
}

#[cfg(not(unix))]
fn install_signal_handlers() {}

/// Подсказка про mold, если линковка не настроена.
fn linker_hint(dir: &Path) {
    if !cfg!(target_os = "linux") {
        return;
    }
    let mut configured = std::env::var("RUSTFLAGS").is_ok_and(|f| f.contains("fuse-ld"));
    let mut current: Option<PathBuf> = Some(dir.to_path_buf());
    while let Some(d) = current {
        for name in ["config.toml", "config"] {
            if std::fs::read_to_string(d.join(".cargo").join(name))
                .is_ok_and(|c| c.contains("fuse-ld") || c.contains("mold"))
            {
                configured = true;
            }
        }
        current = d.parent().map(Path::to_path_buf);
    }
    if configured {
        return;
    }
    eprintln!(
        "{}",
        term::dim(
            "  tez  подсказка: пересборка медленная. Быстрый линкер mold ускоряет её в разы:\n\
             \x20      sudo apt install mold clang, затем в .cargo/config.toml проекта:\n\
             \x20      [target.x86_64-unknown-linux-gnu]\n\
             \x20      linker = \"clang\"\n\
             \x20      rustflags = [\"-C\", \"link-arg=-fuse-ld=mold\"]"
        )
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relevant_files() {
        let root = Path::new("/p");
        let yes = [
            "/p/src/main.rs",
            "/p/src/routers/items.rs",
            "/p/Cargo.toml",
            "/p/.env",
            "/p/.env.local",
            "/p/migrations/0001_init.sql",
            "/p/migrations/2024_x.up.sql",
        ];
        let no = [
            "/p/target/debug/app",
            "/p/target/debug/build/x/out.rs",
            "/p/.git/index",
            "/p/Cargo.lock",
            "/p/src/main.rs~",
            "/p/src/.main.rs.swp",
            "/p/src/.#main.rs",
            "/p/README.md",
            "/p/static/app.js",
            "/p/queries/report.sql",
        ];
        for path in yes {
            assert!(is_relevant(root, Path::new(path)), "{path}");
        }
        for path in no {
            assert!(!is_relevant(root, Path::new(path)), "{path}");
        }
    }
}
