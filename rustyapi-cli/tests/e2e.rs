//! Сквозные тесты `tez`: новый проект, генерация CRUD, тесты проекта, `tez dev`.
//!
//! Долгие (собирают проект с нуля), поэтому `#[ignore]`; в CI — отдельная задача:
//! `cargo test -p rustyapi-cli --test e2e -- --ignored --test-threads=1`.
//! Нужен `DATABASE_URL` (как у остальных тестов с базой).

use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

const TEZ: &str = env!("CARGO_BIN_EXE_tez");

fn workspace() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .to_path_buf()
}

/// Общий target для сгенерированных проектов: зависимости собираются один раз.
fn target_dir() -> PathBuf {
    workspace().join("target").join("tez-e2e")
}

fn tez(dir: &Path, args: &[&str]) -> String {
    let output = Command::new(TEZ)
        .args(args)
        .current_dir(dir)
        .env("CARGO_TARGET_DIR", target_dir())
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    assert!(
        output.status.success(),
        "tez {args:?}:\n{stdout}\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    stdout
}

/// Новый проект во временном каталоге.
fn new_project(name: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!("tez-e2e-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    let _ = std::fs::remove_dir_all(root.join(name));
    let rustyapi = workspace().join("rustyapi");
    tez(
        &root,
        &["new", name, "--rustyapi-path", rustyapi.to_str().unwrap()],
    );
    root.join(name)
}

fn http_get(port: u16, path: &str) -> Option<String> {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).ok()?;
    stream.set_read_timeout(Some(Duration::from_secs(5))).ok()?;
    write!(
        stream,
        "GET {path} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n"
    )
    .ok()?;
    let mut response = String::new();
    stream.read_to_string(&mut response).ok()?;
    Some(response.split("\r\n\r\n").nth(1)?.to_owned())
}

fn wait_for(port: u16, needle: &str, timeout: Duration) -> Duration {
    let started = Instant::now();
    while started.elapsed() < timeout {
        if http_get(port, "/").is_some_and(|body| body.contains(needle)) {
            return started.elapsed();
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    panic!("за {timeout:?} сервер не ответил {needle:?}");
}

struct KillOnDrop(Child);

impl Drop for KillOnDrop {
    fn drop(&mut self) {
        #[cfg(unix)]
        unsafe {
            libc::kill(self.0.id() as libc::pid_t, libc::SIGINT);
        }
        let _ = self.0.wait();
    }
}

#[test]
#[ignore = "долгий: собирает новый проект"]
fn new_generate_and_test() {
    let Ok(url) = std::env::var("DATABASE_URL") else {
        eprintln!("DATABASE_URL не задан, тест пропущен");
        return;
    };
    let dir = new_project("blog");
    tez(&dir, &["generate", "crud", "post"]);
    tez(&dir, &["generate", "crud", "category"]);
    let lib = std::fs::read_to_string(dir.join("src/lib.rs")).unwrap();
    assert!(lib.contains("routers::posts::router()"));
    assert!(lib.contains("routers::categories::router()"));

    let fmt = Command::new("cargo")
        .args(["fmt", "--check"])
        .current_dir(&dir)
        .status()
        .unwrap();
    assert!(fmt.success(), "сгенерированный код отформатирован");

    let test = Command::new("cargo")
        .arg("test")
        .current_dir(&dir)
        .env("CARGO_TARGET_DIR", target_dir())
        .env("DATABASE_URL", &url)
        .status()
        .unwrap();
    assert!(test.success(), "тесты сгенерированного проекта");

    let routes = tez(&dir, &["routes"]);
    for line in [
        "POST    /posts",
        "DELETE  /categories/{id}",
        "GET     /health",
    ] {
        assert!(routes.contains(line), "{line}\n{routes}");
    }
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
#[ignore = "долгий: собирает новый проект"]
fn dev_restarts_and_survives_build_errors() {
    let dir = new_project("devapp");
    let port = 18000 + (std::process::id() % 1000) as u16;
    let child = Command::new(TEZ)
        .arg("dev")
        .current_dir(&dir)
        .env("CARGO_TARGET_DIR", target_dir())
        .env("PORT", port.to_string())
        // Без базы: миграций нет, приложению она не нужна.
        .env("DATABASE_URL", "postgres://nobody@127.0.0.1:1/none")
        .stdout(Stdio::null())
        .spawn()
        .unwrap();
    let _guard = KillOnDrop(child);
    wait_for(port, "Hello from devapp", Duration::from_secs(1200));

    let root = dir.join("src/routers/root.rs");
    let source = std::fs::read_to_string(&root).unwrap();
    std::fs::write(&root, source.replace("Hello from devapp", "Hello v2")).unwrap();
    let took = wait_for(port, "Hello v2", Duration::from_secs(120));
    eprintln!("перезапуск за {took:?}");

    // Ошибка компиляции: старый сервер продолжает отвечать.
    let broken = source.replace("Hello from devapp", "Hello v3").replace(
        "Json(json!({ \"message\": \"Hello v3\" }))",
        "Json(json!({ \"message\": \"Hello v3\" })) +",
    );
    assert_ne!(broken, source.replace("Hello from devapp", "Hello v3"));
    std::fs::write(&root, &broken).unwrap();
    std::thread::sleep(Duration::from_secs(8));
    assert!(http_get(port, "/").unwrap().contains("Hello v2"));

    std::fs::write(&root, source.replace("Hello from devapp", "Hello v3")).unwrap();
    wait_for(port, "Hello v3", Duration::from_secs(120));
    drop(_guard);
    std::fs::remove_dir_all(dir).unwrap();
}
