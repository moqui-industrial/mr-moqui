#!/usr/bin/env python3
"""Single-engine protocol smoke, NOT a cross-engine parity coordinator.

Runs the actual binary with default startup data/search on a disposable fresh PostgreSQL
container. Copies only this checkout's runtime. Artifacts remain under --output.
"""
import argparse
import http.server
import json
import os
from pathlib import Path
import shutil
import signal
import socket
import subprocess
import threading
import time
import xml.etree.ElementTree as ET


def port():
    with socket.socket() as sock:
        sock.bind(("127.0.0.1", 0))
        return sock.getsockname()[1]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("--timeout", type=int, default=300)
    args = parser.parse_args()
    root = Path(__file__).resolve().parents[1]
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=False)
    runtime = output / "runtime"
    shutil.copytree(root / "runtime", runtime)
    db_port, http_port = port(), port()
    container = f"moqui-deterministic-smoke-{os.getpid()}"
    events = []
    active = {}

    class Hook(http.server.BaseHTTPRequestHandler):
        def log_message(self, *_):
            pass

        def do_POST(self):
            assert self.path == "/transaction"
            request = json.loads(self.rfile.read(int(self.headers["Content-Length"])))
            assert request["version"] == 1 and request["engine"] == "rust"
            sequence = request["sequence"]
            if request["event"] == "begin":
                assert sequence not in active
                operation = f"op-{sequence:06}"
                active[sequence] = operation
                response = {"operation_id": operation, "time_epoch_ms": 1760000000000 + sequence}
            else:
                assert active.pop(sequence) == request["operation_id"]
                response = {"ok": True}
            events.append(request)
            with (output / "hook.jsonl").open("a") as log:
                log.write(json.dumps(request) + "\n")
            body = json.dumps(response).encode()
            self.send_response(200)
            self.send_header("Content-Type", "application/json")
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)

    hook = http.server.HTTPServer(("127.0.0.1", 0), Hook)
    thread = threading.Thread(target=hook.serve_forever, daemon=True)
    thread.start()
    process = None
    ready = False
    screen_status = None
    health_status = None
    try:
        subprocess.run(["docker", "run", "--rm", "-d", "--name", container,
                        "-e", "POSTGRES_USER=moqui", "-e", "POSTGRES_DB=moqui",
                        "-e", "POSTGRES_HOST_AUTH_METHOD=trust", "-p", f"127.0.0.1:{db_port}:5432",
                        "postgres:18"], check=True, stdout=subprocess.DEVNULL)
        for _ in range(100):
            probe = subprocess.run(["docker", "exec", container, "pg_isready", "-h", "127.0.0.1", "-U", "moqui"],
                                   stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
            if probe.returncode == 0:
                break
            time.sleep(.1)
        tree = ET.parse(runtime / "conf/MoquiDevConf.xml")
        conf = tree.getroot()
        settings = {
            "entity_ds_db_conf": "postgres", "entity_ds_host": "127.0.0.1",
            "entity_ds_port": str(db_port), "entity_ds_database": "moqui", "entity_ds_schema": "public",
            "entity_ds_user": "moqui", "entity_ds_password": "disposable", "entity_ds_sslmode": "disable",
            "entity_empty_db_load": "all", "webapp_http_port": str(http_port),
            "webapp_https_enabled": "false", "cluster_https_port": "0", "opensearch_run": "false",
        }
        for name, value in settings.items():
            node = conf.find(f"default-property[@name='{name}']")
            if node is None:
                node = ET.SubElement(conf, "default-property", {"name": name})
            node.set("value", value)
        tree.write(runtime / "conf/MoquiDevConf.xml")
        env = dict(os.environ, MOQUI_RUNTIME=str(runtime),
                   MOQUI_FRAMEWORK=str(runtime / "base-component/framework"),
                   MOQUI_DETERMINISTIC="true", MOQUI_DETERMINISTIC_ENGINE="rust",
                   MOQUI_DETERMINISTIC_RUN="rust-server-smoke",
                   MOQUI_DETERMINISTIC_HOOK=f"http://127.0.0.1:{hook.server_port}/transaction",
                   MOQUI_DETERMINISTIC_LOG=str(output / "deterministic.log"))
        env.pop("MOQUI_DETERMINISTIC_OFFLINE", None)
        with (output / "server.log").open("w") as log:
            process = subprocess.Popen([str(args.binary.resolve())], env=env, stdout=log, stderr=subprocess.STDOUT)
            deadline = time.monotonic() + args.timeout
            while time.monotonic() < deadline and process.poll() is None:
                text = (output / "server.log").read_text(errors="replace")
                if "Listening for HTTP on" in text:
                    ready = True
                    break
                time.sleep(.1)
            if ready:
                # A real request traverses the synchronous core and the serial application lane.
                import urllib.request
                with urllib.request.urlopen(f"http://127.0.0.1:{http_port}/health", timeout=30) as response:
                    health_status = response.status
                try:
                    with urllib.request.urlopen(f"http://127.0.0.1:{http_port}/status", timeout=30) as response:
                        screen_status = response.status
                        (output / "response.txt").write_bytes(response.read())
                except urllib.error.HTTPError as error:
                    screen_status = error.code
                    (output / "response.txt").write_bytes(error.read())
            if process.poll() is None:
                process.send_signal(signal.SIGTERM)
                try:
                    process.wait(60)
                except subprocess.TimeoutExpired:
                    process.kill()
                    process.wait()
            # Query evidence before the fresh container is removed.
            with (output / "database.txt").open("w") as evidence:
                subprocess.run(["docker", "exec", container, "psql", "-U", "moqui", "-Atc",
                                "SELECT 'public_tables', count(*) FROM information_schema.tables WHERE table_schema='public'; "
                                "SELECT 'enumeration_stamps', count(*), min(last_updated_stamp), max(last_updated_stamp) FROM enumeration; "
                                "SELECT 'search_registry', count(*) FROM search.document_index;"],
                               stdout=evidence, stderr=evidence)
        records = [json.loads(line) for line in (output / "deterministic.log").read_text().splitlines()]
        wire_events = []
        for record in records:
            if record["event"] == "begin":
                assert record["time_epoch_ms"] == 1760000000000 + record["sequence"]
                assert record["artifact"] == record["artifact_stack"][0]
                record = {key: value for key, value in record.items() if key not in ("operation_id", "time_epoch_ms")}
            wire_events.append(record)
        assert wire_events == events, "the engine log must represent actual acknowledged hook traffic"
        summary = {"ready": ready, "exit_code": process.returncode, "hook_events": len(events),
                   "open_operations": sorted(active), "health_status": health_status,
                   "screen_status": screen_status, "single_engine_only": True}
        (output / "summary.json").write_text(json.dumps(summary, indent=2) + "\n")
        print(json.dumps(summary))
        return 0 if ready and health_status == 200 and process.returncode == 0 and not active else 1
    finally:
        if process is not None and process.poll() is None:
            process.kill()
            process.wait()
        hook.shutdown()
        hook.server_close()
        subprocess.run(["docker", "rm", "-f", container], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)


if __name__ == "__main__":
    raise SystemExit(main())
