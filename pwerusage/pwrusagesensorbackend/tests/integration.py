#!/usr/bin/env python3
"""Isolated two-broker test: container edge -> native Spin backend -> PostgreSQL."""

import copy
import importlib.util
import json
import os
from pathlib import Path
import secrets
import signal
import socket
import subprocess
import tempfile
import time
from unittest.mock import patch

APP = Path(__file__).resolve().parents[1]
ROOT = APP.parent
EDGE = ROOT.parent / "iotdevices/SaveEyeEdgeListenerApp"
TOPIC = "pwrusage/sensors/saveeye/v1"
spec = importlib.util.spec_from_file_location("setup_secrets", ROOT / "deploy/setup-secrets.py")
setup = importlib.util.module_from_spec(spec)
spec.loader.exec_module(setup)


def run(args, *, data=None, check=True, env=None):
    return subprocess.run(
        args, input=data, text=True, capture_output=True, check=check, timeout=180, env=env
    )


def wait_for(predicate, description, timeout=45):
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        if predicate():
            return
        time.sleep(0.5)
    raise AssertionError(f"Timed out: {description}")


def free_port():
    with socket.socket() as sock:
        sock.bind(("127.0.0.1", 0))
        return sock.getsockname()[1]


def exercise(directory):
    bundle = directory / "bundle"
    helper = bundle / "pwerusage/deploy/setup-secrets.py"
    helper.parent.mkdir(parents=True)
    helper.write_text((ROOT / "deploy/setup-secrets.py").read_text())
    local_secrets, server_secrets, tls_dir = (
        bundle / "iotdevices/SaveEyeEdgeListenerApp/deploy/secrets",
        bundle / "pwerusage/pwrusagesensorbackend/deploy/secrets",
        directory / "tls",
    )
    tls_dir.mkdir()
    run([
        "openssl", "req", "-x509", "-newkey", "rsa:2048", "-nodes", "-days", "1",
        "-subj", "/CN=localhost", "-addext", "subjectAltName=IP:127.0.0.1",
        "-keyout", str(tls_dir / "server.key"), "-out", str(tls_dir / "server.crt"),
    ])
    device_password, local_password, upstream_password, backend_password, db_password = (
        secrets.token_hex(16) for _ in range(5)
    )
    # Exercise role selection against the deploy-only bundle layout as well as
    # using the resulting real credentials in both brokers.
    for role, answers in (
        ("edge", [device_password, local_password, upstream_password]),
        ("server", [upstream_password, backend_password, "postgresql://test@localhost/saveeye"]),
    ):
        with patch.object(setup, "__file__", str(helper)), \
                patch("sys.argv", [str(helper), role]), \
                patch.object(setup.getpass, "getpass", side_effect=answers):
            setup.main()
    assert (local_secrets / "edge.env").is_file()
    assert (server_secrets / "backend.env").is_file()
    mqtt_port, upstream_port, db_port = free_port(), free_port(), free_port()
    project = f"saveeye-split-test-{secrets.token_hex(4)}"
    env_file = directory / "compose.env"
    settings = {
        "PI_LAN_IP": "127.0.0.1", "MQTT_PORT": mqtt_port,
        "SAVE_EYE_EDGE_IMAGE": os.environ.get("SAVE_EYE_TEST_IMAGE", "saveeye-edge-listener:0.1.0"),
        "SAVE_EYE_SECRETS_DIR": local_secrets,
        "EDGE_ID": "pi-integration", "UPSTREAM_ADDRESS": "mqtt://upstream:1883",
        "SAVE_EYE_DEVICE_SERIAL": "",
        "TEST_UPSTREAM_PORT": upstream_port, "TEST_SERVER_SECRETS": server_secrets,
        "TEST_POSTGRES_PORT": db_port, "TEST_POSTGRES_PASSWORD": secrets.token_hex(16),
        "TEST_TLS_DIR": tls_dir,
    }

    def write_settings():
        env_file.write_text("".join(f"{key}={value}\n" for key, value in settings.items()))
        os.chmod(env_file, 0o600)

    write_settings()
    compose_env = {key: value for key, value in os.environ.items() if key not in settings}
    compose = [
        "docker", "compose", "--project-directory", str(EDGE), "--env-file", str(env_file),
        "-p", project, "-f", str(EDGE / "compose.yaml"), "-f", str(APP / "tests/compose.yaml"),
    ]

    def dc(*args, data=None, check=True):
        return run([*compose, *args], data=data, check=check, env=compose_env)

    def sql(statement, user="postgres", database="saveeye"):
        return dc("exec", "-T", "database", "psql", "-X", "-U", user, "-d", database,
                  "-v", "ON_ERROR_STOP=1", "-At", data=statement).stdout.strip()

    def rows():
        return json.loads(sql(
            "SELECT coalesce(json_agg(t), '[]'::json) FROM "
            "(SELECT device_serial, received_at::text, reading, edge_id, "
            "backend_received_at::text FROM saveeye.latest_readings ORDER BY device_serial) t;"
        ))

    def publish(payload, broker="mosquitto", account="device", topic=None):
        if topic is None:
            topic = "saveeye/telemetry" if broker == "mosquitto" else TOPIC
        return dc("exec", "-T", broker, "mosquitto_pub",
                  "-o", f"/mosquitto/secrets/{account}.options", "-h", "127.0.0.1",
                  "-t", topic, "-q", "1", "-s", data=json.dumps(payload))

    def logs():
        return backend_log.read_text()

    backend_env = os.environ.copy()
    for key in list(backend_env):
        if key.startswith("SPIN_VARIABLE_"):
            del backend_env[key]
    backend_env.update({
        "SPIN_VARIABLE_MQTT_ADDRESS": f"mqtt://127.0.0.1:{upstream_port}",
        "SPIN_VARIABLE_MQTT_PASSWORD": backend_password,
        "SPIN_VARIABLE_DB_URL": (
            f"postgresql://saveeye_listener:{db_password}@127.0.0.1:{db_port}/saveeye"
            "?sslmode=require&connect_timeout=3&options=-c%20statement_timeout%3D5000"
        ),
        "SPIN_VARIABLE_DB_OUTBOUND_HOST": f"postgres://127.0.0.1:{db_port}",
        "SSL_CERT_FILE": str(tls_dir / "server.crt"),
    })
    backend_log = directory / "backend.log"
    backend = None

    def stop_backend():
        nonlocal backend
        if backend is not None:
            os.killpg(backend.pid, signal.SIGTERM)
            backend.wait(timeout=15)
            backend = None

    def start_backend():
        nonlocal backend
        stop_backend()
        with backend_log.open("a") as output:
            backend = subprocess.Popen(
                [os.environ.get("SPIN_BIN", "spin"), "up", "--from", str(APP / "spin.toml"),
                 "--state-dir="],
                cwd=APP, env=backend_env, stdout=output, stderr=subprocess.STDOUT,
                start_new_session=True,
            )

    fixture = json.loads((ROOT.parent / "components/saveeye-telemetry/tests/fixtures/kamstrup-telemetry.json").read_text())

    def send_and_wait(payload):
        serial, power = payload["saveeyeDeviceSerialNumber"], payload["activeActualConsumption"]["total"]

        def stored():
            publish(payload)
            time.sleep(0.3)
            return any(row["device_serial"] == serial and
                       row["reading"]["activeActualConsumption"]["total"] == power
                       for row in rows())

        wait_for(stored, f"two-hop persistence for {serial}")

    def capture_subscriber():
        return subprocess.Popen(
            [*compose, "exec", "-T", "upstream", "mosquitto_sub",
             "-o", "/mosquitto/secrets/backend.options", "-i", "envelope-capture",
             "-t", TOPIC, "-C", "1", "-W", "10"],
            stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True, env=compose_env,
        )

    def acl_read_check(broker, allowed, denied, payload):
        subscribers = {}
        topic = "saveeye/telemetry" if broker == "mosquitto" else TOPIC
        try:
            for account in (allowed, denied):
                client = f"acl-{broker}-{account}"
                subscribers[account] = subprocess.Popen(
                    [*compose, "exec", "-T", broker, "mosquitto_sub", "-o",
                     f"/mosquitto/secrets/{account}.options", "-i", client, "-t", topic,
                     "-C", "1", "-W", "5"],
                    stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True, env=compose_env,
                )
            wait_for(lambda: all(
                f"as acl-{broker}-{account} " in dc("logs", broker).stdout for account in subscribers
            ), "ACL subscribers connected")
            for _ in range(3):
                publish(payload, broker, "device" if broker == "mosquitto" else "edge")
                time.sleep(0.3)
            good, _ = subscribers[allowed].communicate(timeout=10)
            bad, _ = subscribers[denied].communicate(timeout=10)
            assert subscribers[allowed].returncode == 0 and json.loads(good) == payload
            assert subscribers[denied].returncode != 0 and not bad
        finally:
            for subscriber in subscribers.values():
                if subscriber.poll() is None:
                    subscriber.terminate()
                    subscriber.communicate(timeout=10)

    try:
        dc("config", "--quiet")
        dc("up", "-d", "--wait", "database", "mosquitto", "upstream")
        # Verify upgrading a database with existing rows, not just fresh schema setup.
        sql("CREATE ROLE saveeye_owner NOLOGIN; CREATE ROLE saveeye_listener LOGIN; "
            "CREATE DATABASE saveeye OWNER saveeye_owner;", database="postgres")
        sql("SET ROLE saveeye_owner;\n" + (APP / "migrations/001_latest_readings.sql").read_text())
        sql("INSERT INTO saveeye.latest_readings VALUES ('legacy','2026-01-01','{}');")
        sql("SET ROLE saveeye_owner;\n" + (APP / "migrations/002_edge_provenance.sql").read_text())
        assert sql("SELECT edge_id IS NULL AND backend_received_at IS NULL "
                   "FROM saveeye.latest_readings WHERE device_serial='legacy';") == "t"
        sql("DELETE FROM saveeye.latest_readings WHERE device_serial='legacy';")
        sql("REVOKE ALL ON DATABASE saveeye FROM PUBLIC; "
            "GRANT CONNECT ON DATABASE saveeye TO saveeye_listener; "
            "REVOKE CREATE ON SCHEMA public FROM PUBLIC; "
            "GRANT USAGE ON SCHEMA saveeye TO saveeye_listener; "
            "GRANT SELECT,INSERT,UPDATE ON saveeye.latest_readings TO saveeye_listener;")
        dc("exec", "-T", "database", "psql", "-X", "-U", "postgres", "-d", "saveeye",
           "-c", "\\password saveeye_listener", data=f"{db_password}\n{db_password}\n")
        for statement in (
            "CREATE TABLE saveeye.forbidden (id integer);", "DELETE FROM saveeye.latest_readings;",
        ):
            result = dc("exec", "-T", "database", "psql", "-X", "-U", "saveeye_listener",
                        "-d", "saveeye", "-v", "ON_ERROR_STOP=1", "-c", statement, check=False)
            assert result.returncode != 0
        start_backend()
        dc("up", "-d", "--no-build", "listener")
        send_and_wait(fixture)
        capture = capture_subscriber()
        try:
            wait_for(lambda: "as envelope-capture " in dc("logs", "upstream").stdout, "capture connected")
            publish(fixture)
            captured, _ = capture.communicate(timeout=15)
        finally:
            if capture.poll() is None:
                capture.terminate()
                capture.communicate(timeout=10)
        envelope = json.loads(captured)
        assert envelope["schemaVersion"] == 1 and envelope["edgeId"] == "pi-integration"
        assert envelope["reading"]["netConsumptionW"] == 812
        wait_for(lambda: rows()[0]["reading"]["receivedAt"] == envelope["reading"]["receivedAt"],
                 "edge receipt timestamp preserved")
        row = rows()[0]
        assert row["edge_id"] == "pi-integration" and row["backend_received_at"]
        assert row["reading"]["activeTotalConsumptionKwh"] == 18423.3
        assert row["reading"]["activeTotalProductionKwh"] == 2150.4
        assert sql("SELECT count(*) FROM pg_stat_ssl s JOIN pg_stat_activity a USING(pid) "
                   "WHERE a.usename='saveeye_listener' AND s.ssl;") != "0"

        acl_read_check("mosquitto", "listener", "device", fixture)
        acl_read_check("upstream", "backend", "edge", envelope)
        for broker in ("mosquitto", "upstream"):
            anonymous = dc("exec", "-T", broker, "mosquitto_sub", "-t", TOPIC,
                           "-E", "-W", "2", check=False)
            assert anonymous.returncode != 0
        baseline = rows()
        for broker, account, topic in (
            ("mosquitto", "listener", "saveeye/telemetry"),
            ("upstream", "backend", TOPIC),
            ("upstream", "edge", "unrelated/topic"),
        ):
            denied = dc("exec", "-T", broker, "mosquitto_pub", "-o",
                        f"/mosquitto/secrets/{account}.options", "-t", topic,
                        "-V", "mqttv5", "-q", "1", "-d", "-m", "{}", check=False)
            assert "RC:135" in denied.stdout
        assert rows() == baseline

        for bad in (
            dict(envelope, schemaVersion=99), dict(envelope, edgeId=""),
            dict(envelope, reading=dict(envelope["reading"], netConsumptionW=123456)),
            dict(envelope, reading=dict(envelope["reading"], receivedAt="not-a-time")),
            {"raw": "not an envelope"},
        ):
            publish(bad, "upstream", "edge")
        publish("{bad raw")
        time.sleep(1)
        assert rows() == baseline, "Invalid input changed stored readings"

        # Equal-time duplicate is a no-op, including backend receipt metadata.
        publish(envelope, "upstream", "edge")
        old = copy.deepcopy(envelope)
        old["reading"]["receivedAt"] = "2020-01-01T00:00:00Z"
        publish(old, "upstream", "edge")
        time.sleep(0.5)
        assert rows() == baseline
        second = dict(fixture, saveeyeDeviceSerialNumber="SECOND-DEVICE")
        send_and_wait(second)
        send_and_wait(dict(
            second,
            activeActualConsumption={"total": 18.336317597934625},
            activeActualProduction={"total": 18.336317597934625 / 3},
            activeTotalConsumption={"total": 18.336317597934625 * 1000},
        ))
        assert len(rows()) == 2

        # Filter still applies before forwarding.
        settings["SAVE_EYE_DEVICE_SERIAL"] = "J85LH2H4"
        write_settings()
        dc("up", "-d", "--no-build", "--force-recreate", "listener")
        baseline = rows()

        def filtered():
            publish(dict(second, activeActualConsumption={"total": 999}))
            time.sleep(0.3)
            return "ignoring telemetry" in dc("logs", "listener").stdout

        wait_for(filtered, "edge filtering")
        assert rows() == baseline
        settings["SAVE_EYE_DEVICE_SERIAL"] = ""
        write_settings()
        dc("up", "-d", "--no-build", "--force-recreate", "listener")
        send_and_wait(fixture)

        # No backend subscriber: broker PUBACK is not a storage acknowledgement.
        stop_backend()
        baseline = rows()
        publish(dict(fixture, activeActualConsumption={"total": 850}))
        time.sleep(1)
        assert rows() == baseline
        start_backend()
        fixture["activeActualConsumption"]["total"] = 851
        send_and_wait(fixture)

        dc("stop", "database")
        publish(dict(fixture, activeActualConsumption={"total": 852}))
        wait_for(lambda: "open SaveEye PostgreSQL connection failed" in logs(), "database failure logged")
        dc("up", "-d", "--wait", "database")
        assert rows()[0]["reading"]["activeActualConsumption"]["total"] == 851
        fixture["activeActualConsumption"]["total"] = 853
        send_and_wait(fixture)

        backend_env["SSL_CERT_FILE"] = "/dev/null"
        start_backend()
        baseline = rows()
        failures = logs().count("open SaveEye PostgreSQL connection failed")

        def tls_rejected():
            publish(dict(fixture, activeActualConsumption={"total": 854}))
            time.sleep(0.5)
            return logs().count("open SaveEye PostgreSQL connection failed") > failures

        wait_for(tls_rejected, "untrusted PostgreSQL certificate rejected")
        assert rows() == baseline, "Untrusted PostgreSQL certificate accepted"
        backend_env["SSL_CERT_FILE"] = str(tls_dir / "server.crt")
        start_backend()
        fixture["activeActualConsumption"]["total"] = 855
        send_and_wait(fixture)

        dc("stop", "upstream")
        publish(dict(fixture, activeActualConsumption={"total": 856}))
        wait_for(lambda: "upstream MQTT" in dc("logs", "listener").stdout, "upstream failure logged")
        dc("up", "-d", "--wait", "upstream")
        start_backend()  # Trigger plugin requires resubscription after broker restart.
        fixture["activeActualConsumption"]["total"] = 857
        send_and_wait(fixture)
        dc("restart", "mosquitto")
        dc("up", "-d", "--wait", "mosquitto")
        dc("restart", "listener")
        fixture["activeActualConsumption"]["total"] = 858
        send_and_wait(fixture)

        # An accepted TCP connection with no MQTT acknowledgement must time out.
        dc("up", "-d", "blackhole")
        settings["UPSTREAM_ADDRESS"] = "mqtt://blackhole:1885"
        write_settings()
        dc("up", "-d", "--no-build", "--force-recreate", "listener")

        def subscribed():
            publish("readiness probe")
            return "ignoring invalid telemetry" in dc("logs", "listener").stdout

        wait_for(subscribed, "edge subscription before stalled-broker test")
        started = time.monotonic()
        publish(fixture)
        # The pinned host also has a five-second MQTT connection timeout, which
        # may win the race with the component's deadline.
        wait_for(lambda: any(message in dc("logs", "listener").stdout for message in (
            "timed out after 5 seconds", "upstream MQTT publish failed",
        )), "bounded forwarding timeout", timeout=10)
        assert 4 <= time.monotonic() - started <= 12, "Stalled connection was not bounded"
        settings["UPSTREAM_ADDRESS"] = "mqtt://upstream:1883"
        write_settings()
        dc("up", "-d", "--no-build", "--force-recreate", "listener")
        fixture["activeActualConsumption"]["total"] = 859
        send_and_wait(fixture)

        dc("exec", "-T", "database", "pg_dump", "-U", "postgres", "-d", "saveeye",
           "-Fc", "-f", "/tmp/saveeye.dump")
        dc("exec", "-T", "database", "createdb", "-U", "postgres", "-O",
           "saveeye_owner", "saveeye_restore")
        dc("exec", "-T", "database", "pg_restore", "-U", "postgres", "-d",
           "saveeye_restore", "--exit-on-error", "/tmp/saveeye.dump")
        assert sql("SELECT count(*) FROM saveeye.latest_readings;", database="saveeye_restore") == "2"
        combined_logs = logs() + dc("logs", "listener").stdout
        for value in (device_password, local_password, upstream_password, backend_password, db_password):
            assert value not in combined_logs
        print("PASS: native backend, two-hop envelopes, ACLs, filtering, migration, ordering, "
              "timeouts, outages, TLS, broker recovery and backup/restore")
    except Exception:
        print(logs() if backend_log.exists() else "Backend not started")
        print(dc("logs", "--tail", "30", check=False).stdout)
        raise
    finally:
        stop_backend()
        dc("down", "-v", "--remove-orphans")


if __name__ == "__main__":
    with tempfile.TemporaryDirectory(prefix="saveeye-split-") as directory:
        exercise(Path(directory))
