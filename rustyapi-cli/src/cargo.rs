//! Сборка через cargo: выбор бинарника, `--message-format=json-render-diagnostics`, путь к артефакту.

use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use serde_json::Value;

use crate::{Result, TargetArgs};

/// Что собирать: пакет и бинарник, каталог проекта.
#[derive(Debug, Clone)]
pub struct Target {
    pub dir: PathBuf,
    pub package: String,
    pub bin: String,
}

impl Target {
    /// Определяет пакет и бинарник через `cargo metadata`.
    pub fn resolve(args: &TargetArgs) -> Result<Self> {
        let dir = match &args.manifest_dir {
            Some(dir) => dir.clone(),
            None => std::env::current_dir()?,
        };
        let dir = dir
            .canonicalize()
            .map_err(|e| format!("{}: {e}", dir.display()))?;
        if !dir.join("Cargo.toml").exists() {
            return Err(format!(
                "в {} нет Cargo.toml: запустите tez в каталоге проекта или создайте его через `tez new`",
                dir.display()
            )
            .into());
        }
        let output = Command::new(cargo())
            .args(["metadata", "--no-deps", "--format-version", "1"])
            .current_dir(&dir)
            .stderr(Stdio::inherit())
            .output()?;
        if !output.status.success() {
            return Err("cargo metadata завершился с ошибкой".into());
        }
        let metadata: Value = serde_json::from_slice(&output.stdout)?;
        let packages = metadata["packages"].as_array().cloned().unwrap_or_default();
        let own_manifest = dir.join("Cargo.toml");
        // (пакет, бинарник, default-run) для всех бинарников-кандидатов.
        let mut bins = Vec::new();
        for package in &packages {
            let name = package["name"].as_str().unwrap_or_default();
            if args.package.as_deref().is_some_and(|wanted| wanted != name) {
                continue;
            }
            let is_own =
                Path::new(package["manifest_path"].as_str().unwrap_or_default()) == own_manifest;
            for target in package["targets"].as_array().into_iter().flatten() {
                let is_bin = target["kind"]
                    .as_array()
                    .is_some_and(|k| k.iter().any(|k| k == "bin"));
                if !is_bin {
                    continue;
                }
                let bin = target["name"].as_str().unwrap_or_default();
                if args.bin.as_deref().is_some_and(|wanted| wanted != bin) {
                    continue;
                }
                let default = package["default_run"].as_str() == Some(bin);
                bins.push((name.to_owned(), bin.to_owned(), is_own, default));
            }
        }
        // Пакет в текущем каталоге важнее остальных членов workspace.
        if args.package.is_none() && bins.iter().any(|b| b.2) {
            bins.retain(|b| b.2);
        }
        if bins.len() > 1 && bins.iter().any(|b| b.3) {
            bins.retain(|b| b.3);
        }
        match bins.as_slice() {
            [(package, bin, ..)] => Ok(Target {
                dir,
                package: package.clone(),
                bin: bin.clone(),
            }),
            [] => Err("не найден бинарник для запуска (проверьте --package / --bin)".into()),
            many => {
                let list: Vec<String> = many.iter().map(|b| format!("{} ({})", b.1, b.0)).collect();
                Err(format!(
                    "несколько бинарников: {}; выберите через --bin или --package",
                    list.join(", ")
                )
                .into())
            }
        }
    }
}

/// Путь к cargo (`$CARGO`, если tez запущен из cargo).
pub fn cargo() -> String {
    std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_owned())
}

/// Собирает бинарник. Ошибки компилятора печатает cargo (в stderr, с цветом).
/// `Ok(None)` — сборка не удалась.
pub fn build(target: &Target, release: bool, quiet: bool) -> Result<Option<PathBuf>> {
    let mut command = Command::new(cargo());
    command
        .arg("build")
        .arg("--message-format=json-render-diagnostics")
        .args(["--package", &target.package, "--bin", &target.bin])
        .current_dir(&target.dir)
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit());
    if release {
        command.arg("--release");
    }
    if quiet {
        command.arg("--quiet");
    }
    if crate::term::red("x") != "x" {
        command.arg("--color=always");
    }
    let mut child = command.spawn()?;
    let stdout = child.stdout.take().expect("stdout piped");
    let mut executable = None;
    for line in BufReader::new(stdout).lines() {
        let Ok(message) = serde_json::from_str::<Value>(&line?) else {
            continue;
        };
        let ours = message["reason"] == "compiler-artifact"
            && message["target"]["name"] == target.bin.as_str();
        if let Some(path) = message["executable"].as_str().filter(|_| ours) {
            executable = Some(PathBuf::from(path));
        }
    }
    let status = child.wait()?;
    if !status.success() {
        return Ok(None);
    }
    Ok(Some(executable.ok_or("cargo не сообщил путь к бинарнику")?))
}

/// `tez run`: release-сборка и запуск (с передачей аргументов).
pub fn run_release(args: &TargetArgs, app_args: &[String]) -> Result<()> {
    let target = Target::resolve(args)?;
    let Some(exe) = build(&target, true, false)? else {
        return Err("сборка не удалась".into());
    };
    crate::term::status(&format!("запуск {}", exe.display()));
    let status = Command::new(exe)
        .args(app_args)
        .current_dir(&target.dir)
        .status()?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("приложение завершилось: {status}").into())
    }
}
