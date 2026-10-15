#!/usr/bin/env python3
"""Прогон бенчмарков: RustyAPI, axum, actix-web, FastAPI (uvicorn + uvloop + httptools).

    cd benchmarks
    cargo build --release
    pip install fastapi uvicorn uvloop httptools asyncpg
    DATABASE_URL=postgres://postgres:postgres@localhost:5432/bench python3 run.py

Нужен `oha` (`cargo install oha`). Результат печатается и пишется в RESULTS.md.
"""

import json
import os
import platform
import subprocess
import sys
import time
import urllib.request

DURATION = os.environ.get("BENCH_DURATION", "10s")
CONNECTIONS = os.environ.get("BENCH_CONNECTIONS", "64")
WORKERS = os.cpu_count() or 4
HERE = os.path.dirname(os.path.abspath(__file__))
DB = os.environ["DATABASE_URL"]

FRAMEWORKS = [
    ("RustyAPI", 8101, [f"{HERE}/target/release/rustyapi_bench"]),
    ("axum", 8102, [f"{HERE}/target/release/axum_bench"]),
    ("actix-web", 8103, [f"{HERE}/target/release/actix_bench"]),
    (
        f"FastAPI ({WORKERS} workers)",
        8104,
        [
            sys.executable, "-m", "uvicorn", "main:app", "--port", "8104",
            "--workers", str(WORKERS), "--log-level", "warning", "--no-access-log",
            "--loop", "uvloop", "--http", "httptools",
        ],
    ),
]

BODY = json.dumps({"name": "Widget", "price": 9.99, "tags": ["a", "b"]})
SCENARIOS = [
    ("hello world: GET /", ["/"], []),
    ("JSON + валидация: POST /items", ["/items"], ["-m", "POST", "-T", "application/json", "-d", BODY]),
    ("PostgreSQL: GET /todos/{id}", ["/todos/[1-9][0-9]{0,2}"], ["--rand-regex-url"]),
]


def seed() -> None:
    sql = (
        "DROP TABLE IF EXISTS bench_todos;"
        "CREATE TABLE bench_todos (id BIGINT PRIMARY KEY, title TEXT NOT NULL, done BOOLEAN NOT NULL);"
        "INSERT INTO bench_todos SELECT i, 'todo ' || i, i % 2 = 0 FROM generate_series(1, 1000) i;"
    )
    subprocess.run(["psql", DB, "-q", "-c", sql], check=True)


def wait_ready(port: int) -> None:
    for _ in range(100):
        try:
            urllib.request.urlopen(f"http://127.0.0.1:{port}/", timeout=1).read()
            return
        except OSError:
            time.sleep(0.1)
    raise RuntimeError(f"сервер на порту {port} не поднялся")


def oha(port: int, path: str, extra: list[str], duration: str) -> dict:
    url = f"http://127.0.0.1:{port}{path}"
    out = subprocess.run(
        ["oha", "-z", duration, "-c", CONNECTIONS, "--no-tui", "--output-format", "json", *extra, url],
        check=True, capture_output=True, text=True,
    ).stdout
    return json.loads(out)


def main() -> None:
    seed()
    results: dict[str, dict[str, tuple[float, float]]] = {}
    for name, port, cmd in FRAMEWORKS:
        env = dict(os.environ, PORT=str(port), RUST_LOG="warn", DATABASE_POOL_SIZE="16",
                   DATABASE_POOL_SIZE_PER_WORKER=str(max(1, 16 // WORKERS)))
        server = subprocess.Popen(cmd, cwd=f"{HERE}/fastapi_bench", env=env)
        try:
            wait_ready(port)
            results[name] = {}
            for scenario, (path,), extra in SCENARIOS:
                oha(port, path, extra, "2s")  # прогрев
                data = oha(port, path, extra, DURATION)
                codes = data.get("statusCodeDistribution", {})
                bad = {c: n for c, n in codes.items() if c != "200"}
                if bad:
                    raise RuntimeError(f"{name} {scenario}: не 200: {bad}")
                rps = data["summary"]["requestsPerSec"]
                p99 = data["latencyPercentiles"]["p99"] * 1000
                results[name][scenario] = (rps, p99)
                print(f"{name:<22} {scenario:<32} {rps:>10.0f} rps  p99 {p99:6.2f} ms", flush=True)
        finally:
            server.terminate()
            server.wait(timeout=10)

    lines = [
        "# Результаты бенчмарков",
        "",
        f"Машина: {platform.machine()}, ядер: {os.cpu_count()}, {platform.system()} {platform.release()}. "
        f"Генератор нагрузки `oha` на той же машине, соединений: {CONNECTIONS}, {DURATION} на сценарий "
        "после 2 с прогрева. PostgreSQL на той же машине, пул на 16 соединений.",
        "",
        "| Фреймворк | " + " | ".join(s for s, _, _ in SCENARIOS) + " |",
        "|---|" + "---|" * len(SCENARIOS),
    ]
    for name, by_scenario in results.items():
        cells = [f"{by_scenario[s][0]:,.0f} rps<br>p99 {by_scenario[s][1]:.1f} мс" for s, _, _ in SCENARIOS]
        lines.append(f"| {name} | " + " | ".join(cells) + " |")
    with open(f"{HERE}/RESULTS.md", "w") as f:
        f.write("\n".join(lines) + "\n")
    print("\n".join(lines))


if __name__ == "__main__":
    main()
