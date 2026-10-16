//! `tez routes`: таблица эндпоинтов. Бинарник приложения запускается с `RUSTYAPI_PRINT=routes`
//! и печатает маршруты вместо запуска сервера.

use std::process::{Command, Stdio};

use serde_json::Value;

use crate::cargo::{self, Target};
use crate::{Result, TargetArgs};

/// Собирает приложение и запрашивает у него `RUSTYAPI_PRINT=<what>` (routes / openapi).
pub fn query_app(args: &TargetArgs, what: &str) -> Result<Value> {
    let target = Target::resolve(args)?;
    let exe = cargo::build(&target, false, true)?.ok_or("сборка не удалась")?;
    let output = Command::new(&exe)
        .env("RUSTYAPI_PRINT", what)
        .current_dir(&target.dir)
        .stderr(Stdio::inherit())
        .output()?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let last = stdout.lines().rev().find(|l| !l.trim().is_empty());
    match last.and_then(|l| serde_json::from_str(l).ok()) {
        Some(value) if output.status.success() => Ok(value),
        _ => Err(format!(
            "{} не вернул {what}: приложение должно запускаться через App::run() \
             (RustyAPI {} или новее)",
            target.bin,
            env!("CARGO_PKG_VERSION")
        )
        .into()),
    }
}

pub fn run(args: &TargetArgs, json: bool) -> Result<()> {
    let routes = query_app(args, "routes")?;
    if json {
        println!("{}", serde_json::to_string_pretty(&routes)?);
        return Ok(());
    }
    print!("{}", table(&routes));
    Ok(())
}

/// Таблица `METHOD  PATH  STATUS  NAME  TAGS`, отсортированная по пути.
pub fn table(routes: &Value) -> String {
    let mut rows: Vec<[String; 5]> = routes
        .as_array()
        .into_iter()
        .flatten()
        .map(|r| {
            let tags: Vec<&str> = r["tags"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .collect();
            let mut name = r["name"].as_str().unwrap_or_default().to_owned();
            if r["deprecated"] == true {
                name.push_str(" (deprecated)");
            }
            if r["include_in_schema"] == false {
                name.push_str(" (hidden)");
            }
            [
                r["method"].as_str().unwrap_or_default().to_owned(),
                r["path"].as_str().unwrap_or_default().to_owned(),
                r["status_code"].to_string(),
                name,
                tags.join(", "),
            ]
        })
        .collect();
    rows.sort_by(|a, b| (&a[1], method_order(&a[0])).cmp(&(&b[1], method_order(&b[0]))));
    let header = ["METHOD", "PATH", "STATUS", "HANDLER", "TAGS"].map(String::from);
    let mut widths = header.clone().map(|h| h.chars().count());
    for row in &rows {
        for (w, cell) in widths.iter_mut().zip(row) {
            *w = (*w).max(cell.chars().count());
        }
    }
    let mut out = String::new();
    for row in std::iter::once(&header).chain(&rows) {
        let cells: Vec<String> = row
            .iter()
            .zip(widths)
            .map(|(cell, w)| format!("{cell:<w$}"))
            .collect();
        out.push_str(cells.join("  ").trim_end());
        out.push('\n');
    }
    out.push_str(&format!("\n{} эндпоинтов\n", rows.len()));
    out
}

fn method_order(method: &str) -> usize {
    ["GET", "POST", "PUT", "PATCH", "DELETE", "HEAD", "OPTIONS"]
        .iter()
        .position(|m| *m == method)
        .unwrap_or(99)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    #[test]
    fn table() {
        let routes = json!([
            {"method": "POST", "path": "/todos", "name": "create_todo", "status_code": 201, "tags": ["todos"], "deprecated": false, "include_in_schema": true},
            {"method": "GET", "path": "/", "name": "root", "status_code": 200, "tags": [], "deprecated": true, "include_in_schema": true},
            {"method": "GET", "path": "/todos", "name": "list_todos", "status_code": 200, "tags": ["todos"], "deprecated": false, "include_in_schema": false},
        ]);
        assert_eq!(
            super::table(&routes),
            "METHOD  PATH    STATUS  HANDLER              TAGS\n\
             GET     /       200     root (deprecated)\n\
             GET     /todos  200     list_todos (hidden)  todos\n\
             POST    /todos  201     create_todo          todos\n\
             \n3 эндпоинтов\n"
        );
    }
}
